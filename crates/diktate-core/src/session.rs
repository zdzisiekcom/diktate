//! Sesja: spina przechwytywanie, VAD, segmentację i transkrypcję w jeden
//! strumieniowy potok.
//!
//! Zasada: praca dzieje się na trzech wątkach, każdy zna tylko sąsiada
//! kanałem `mpsc`. Wątek `cpal` (uruchomiony przez
//! [`crate::capture::Recorder::start_streaming`]) tylko miksuje do mono i
//! resampluje do 16 kHz, po czym wypycha porcje kanałem — bez blokowania
//! ścieżki czasu rzeczywistego. Ten moduł dokłada wątek segmentujący: składa
//! nadchodzące porcje w ramki o stałym rozmiarze [`crate::vad::CHUNK_SAMPLES`]
//! (przez [`FrameAssembler`]), pyta [`crate::vad::Vad`] o każdą ramkę i karmi
//! [`crate::segmenter::Segmenter`]. Gotowe segmenty trafiają kanałem
//! *ograniczonej* pojemności do wątku transkrypcji.
//!
//! Ograniczenie kolejki jest celowe: transkrypcja na CPU bywa wolniejsza niż
//! mowa, więc kolejka segmentów nie może rosnąć bez końca — zjadłaby pamięć
//! i sprawiła, że napisy pojawiałyby się z coraz większym opóźnieniem. Gdy
//! kolejka jest pełna, nowy segment jest porzucany (z komunikatem na
//! stderr), a nie kolejkowany — wątek segmentujący nigdy się nie blokuje na
//! wątku transkrypcji.
//!
//! `Session::stop` zatrzymuje capture, domyka ewentualny trwający segment
//! przez `Segmenter::flush`, i dopiero po dokończeniu przez wątek
//! transkrypcji tego, co miał już w kolejce, wraca do wywołującego.

use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::Arc;
use std::thread::JoinHandle;

use anyhow::{anyhow, Context, Result};

use crate::capture::{Recorder, StreamingRecording};
use crate::segmenter::{Segmenter, SegmenterConfig};
use crate::transcribe::Transcriber;
use crate::vad::{Vad, CHUNK_SAMPLES};

/// Pojemność kanału między wątkiem segmentującym a wątkiem transkrypcji.
/// Patrz komentarz modułu — to celowy, twardy limit.
const SEGMENT_QUEUE_CAPACITY: usize = 8;

/// Skleja porcje o dowolnej, nierównej długości (takie, jakie dają callbacki
/// `cpal`) w ramki o stałym rozmiarze [`CHUNK_SAMPLES`], wymaganym przez VAD.
/// Resztki niedopełniające pełnej ramki zostają w wewnętrznym buforze do
/// kolejnego wywołania [`FrameAssembler::push`].
#[derive(Default)]
struct FrameAssembler {
    buffer: Vec<f32>,
}

impl FrameAssembler {
    fn new() -> Self {
        Self {
            buffer: Vec::with_capacity(CHUNK_SAMPLES * 2),
        }
    }

    /// Dokłada porcję i zwraca tyle pełnych ramek [`CHUNK_SAMPLES`], ile
    /// udało się złożyć. Ostatnia, niedopełniona reszta zostaje w buforze.
    fn push(&mut self, chunk: &[f32]) -> Vec<Vec<f32>> {
        self.buffer.extend_from_slice(chunk);
        let mut frames = Vec::new();
        while self.buffer.len() >= CHUNK_SAMPLES {
            frames.push(self.buffer.drain(..CHUNK_SAMPLES).collect());
        }
        frames
    }
}

/// Trwająca sesja: przechwytywanie + VAD + segmentacja + transkrypcja na
/// bieżąco. Gotowy tekst każdego segmentu trafia do `on_segment` podanego
/// przy [`Session::start`].
pub struct Session {
    capture: Option<StreamingRecording>,
    segmenter_handle: Option<JoinHandle<()>>,
    transcriber_handle: Option<JoinHandle<()>>,
}

impl Session {
    /// Uruchamia mikrofon i dwa wątki robocze (segmentujący i transkrypcji).
    /// `on_segment` jest wołane z wątku transkrypcji dla każdego niepustego
    /// wyniku — musi być `Send + 'static`, bo żyje poza wątkiem wołającym.
    pub fn start<F>(transcriber: Arc<Transcriber>, on_segment: F) -> Result<Self>
    where
        F: Fn(String) + Send + 'static,
    {
        let vad = Vad::new().context("nie udało się utworzyć detektora VAD")?;
        let segmenter = Segmenter::new(SegmenterConfig::default());

        let (chunk_tx, chunk_rx) = mpsc::channel::<Vec<f32>>();
        let capture = Recorder::start_streaming(chunk_tx)
            .context("nie udało się uruchomić przechwytywania strumieniowego")?;

        let (segment_tx, segment_rx) = mpsc::sync_channel::<Vec<f32>>(SEGMENT_QUEUE_CAPACITY);

        let segmenter_handle = std::thread::spawn(move || {
            run_segmenter(chunk_rx, segment_tx, vad, segmenter);
        });

        let transcriber_handle = std::thread::spawn(move || {
            run_transcriber(segment_rx, transcriber.as_ref(), on_segment);
        });

        Ok(Self {
            capture: Some(capture),
            segmenter_handle: Some(segmenter_handle),
            transcriber_handle: Some(transcriber_handle),
        })
    }

    /// Zatrzymuje sesję: najpierw mikrofon (co rozłącza kanał do wątku
    /// segmentującego), potem czeka aż wątek segmentujący domknie ostatni
    /// segment i skończy, na końcu czeka aż wątek transkrypcji dokończy to,
    /// co miał już w kolejce. Wraca dopiero, gdy cała trójka ucichła.
    pub fn stop(mut self) -> Result<()> {
        if let Some(capture) = self.capture.take() {
            capture
                .stop()
                .context("nie udało się zatrzymać przechwytywania")?;
        }
        if let Some(handle) = self.segmenter_handle.take() {
            handle
                .join()
                .map_err(|_| anyhow!("wątek segmentujący spanikował"))?;
        }
        if let Some(handle) = self.transcriber_handle.take() {
            handle
                .join()
                .map_err(|_| anyhow!("wątek transkrypcji spanikował"))?;
        }
        Ok(())
    }
}

/// Ciało wątku segmentującego: porcje z capture -> ramki -> VAD -> Segmenter
/// -> kolejka do transkrypcji. Kończy się, gdy `chunk_rx` się rozłączy (czyli
/// po `capture.stop()`), po czym domyka ewentualny trwający segment.
fn run_segmenter(
    chunk_rx: Receiver<Vec<f32>>,
    segment_tx: SyncSender<Vec<f32>>,
    mut vad: Vad,
    mut segmenter: Segmenter,
) {
    let mut assembler = FrameAssembler::new();
    while let Ok(chunk) = chunk_rx.recv() {
        for frame in assembler.push(&chunk) {
            let is_speech = vad.is_speech(&frame);
            if let Some(segment) = segmenter.push(&frame, is_speech) {
                dispatch_segment(&segment_tx, segment);
            }
        }
    }
    // Capture się zatrzymało (kanał rozłączony) — domykamy trwający segment,
    // jeśli jakiś był. Reszta niedopełniająca pełnej ramki w assemblerze
    // (poniżej 32 ms) przepada; to nieistotna strata na końcu nagrania.
    if let Some(segment) = segmenter.flush() {
        dispatch_segment(&segment_tx, segment);
    }
    // `segment_tx` spada tu wraz z końcem funkcji — to sygnał dla wątku
    // transkrypcji, że nic więcej nie przyjdzie.
}

/// Wysyła gotowy segment do wątku transkrypcji, nie blokując się. Gdy kolejka
/// jest pełna (transkrypcja nie nadąża) albo odbiorca już zniknął, segment
/// jest porzucany z komunikatem na stderr — nigdy nie czekamy.
fn dispatch_segment(segment_tx: &SyncSender<Vec<f32>>, segment: Vec<f32>) {
    if let Err(err) = segment_tx.try_send(segment) {
        match err {
            mpsc::TrySendError::Full(_) => {
                eprintln!(
                    "sesja: kolejka transkrypcji pełna — segment porzucony (transkrypcja nie nadąża za mową)"
                );
            }
            mpsc::TrySendError::Disconnected(_) => {
                eprintln!("sesja: wątek transkrypcji już zakończony — segment porzucony");
            }
        }
    }
}

/// Ciało wątku transkrypcji: bierze gotowe segmenty i woła `Transcriber`.
/// Kończy się, gdy `segment_rx` się rozłączy i kolejka jest pusta — czyli
/// dopiero po tym, jak wątek segmentujący skończył i domknął ostatni segment.
fn run_transcriber<F>(segment_rx: Receiver<Vec<f32>>, transcriber: &Transcriber, on_segment: F)
where
    F: Fn(String),
{
    while let Ok(segment) = segment_rx.recv() {
        match transcriber.transcribe(&segment) {
            Ok(text) => {
                if !text.is_empty() {
                    on_segment(text);
                }
            }
            Err(err) => {
                eprintln!("sesja: błąd transkrypcji segmentu: {err:#}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assembler_sklada_porcje_w_pelne_ramki() {
        let mut assembler = FrameAssembler::new();

        // Porcje o nierównej długości, żadna nie jest wielokrotnością
        // CHUNK_SAMPLES — tak wyglądają prawdziwe callbacki cpal.
        let sizes = [100usize, 300, 50, 700, 1_337, 20];
        let total: usize = sizes.iter().sum();

        let mut next_value = 0.0f32;
        let mut all_frames: Vec<Vec<f32>> = Vec::new();
        for size in sizes {
            let chunk: Vec<f32> = (0..size)
                .map(|_| {
                    let v = next_value;
                    next_value += 1.0;
                    v
                })
                .collect();
            all_frames.extend(assembler.push(&chunk));
        }

        // Liczba pełnych ramek zgadza się z całkowitą liczbą próbek.
        assert_eq!(all_frames.len(), total / CHUNK_SAMPLES);
        for frame in &all_frames {
            assert_eq!(frame.len(), CHUNK_SAMPLES);
        }

        // Resztka w buforze to dokładnie to, co zostało po odjęciu pełnych ramek.
        assert_eq!(assembler.buffer.len(), total % CHUNK_SAMPLES);

        // Kolejność próbek jest zachowana — sklejone ramki, spłaszczone,
        // odtwarzają ciąg 0..pełne_ramki*CHUNK_SAMPLES bez dziur i przestawień.
        let flat: Vec<f32> = all_frames.into_iter().flatten().collect();
        let expected: Vec<f32> = (0..flat.len() as u32).map(|i| i as f32).collect();
        assert_eq!(flat, expected);
    }

    #[test]
    fn assembler_pusta_porcja_nic_nie_zwraca() {
        let mut assembler = FrameAssembler::new();
        assert!(assembler.push(&[]).is_empty());
    }

    #[test]
    fn assembler_dokladnie_jedna_ramka_bez_reszty() {
        let mut assembler = FrameAssembler::new();
        let chunk = vec![1.0f32; CHUNK_SAMPLES];
        let frames = assembler.push(&chunk);
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0], chunk);
        assert!(assembler.buffer.is_empty());
    }

    #[test]
    fn dispatch_segment_porzuca_gdy_kolejka_pelna_nie_blokujac() {
        let (tx, rx) = mpsc::sync_channel::<Vec<f32>>(1);
        dispatch_segment(&tx, vec![1.0]);
        // Kolejka ma pojemność 1 i jest teraz pełna — drugie wysłanie musi
        // się nie zablokować, tylko porzucić segment.
        dispatch_segment(&tx, vec![2.0]);
        assert_eq!(rx.try_recv().unwrap(), vec![1.0]);
        assert!(rx.try_recv().is_err());
    }
}
