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
//! Moduł raportuje też na bieżąco zaległość kolejki transkrypcji (ile
//! wypowiedzi czeka na przetworzenie plus ta aktualnie transkrybowana) przez
//! callback `on_backlog` w [`Session::start`] — patrz [`Backlog`]. Dzięki
//! temu okno wie, czy to, co ktoś właśnie powiedział, już dotarło na ekran,
//! czy jeszcze się liczy.
//!
//! `Session::stop` zatrzymuje capture, domyka ewentualny trwający segment
//! przez `Segmenter::flush`, i dopiero po dokończeniu przez wątek
//! transkrypcji tego, co miał już w kolejce, wraca do wywołującego.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::Arc;
use std::time::Instant;
use std::thread::JoinHandle;

use anyhow::{anyhow, Context, Result};

use crate::capture::{Recorder, StreamingRecording};
use crate::filters;
use crate::level::{Level, LevelMeter, DEFAULT_INTERVAL_MS};
use crate::segmenter::{Segment, Segmenter, SegmenterConfig};
use crate::transcribe::Transcribe;
use crate::vad::{Vad, CHUNK_SAMPLES};

/// Parametry sesji — wszystkie pochodzą z `daemon.toml` (`docs/04`).
/// Wartości domyślne są tu tylko po to, żeby brak pliku nie blokował startu.
#[derive(Debug, Clone)]
pub struct SessionConfig {
    /// Progi składania ramek w wypowiedzi.
    pub segmenter: SegmenterConfig,
    /// Próg decyzyjny Silero (0.0–1.0).
    pub vad_threshold: f32,
    /// Ile wypowiedzi może czekać na transkrypcję. Limit jest celowy — patrz
    /// komentarz modułu.
    pub queue_capacity: usize,
    /// Odstęp między odczytami poziomu sygnału.
    pub level_interval_ms: u32,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            segmenter: SegmenterConfig::default(),
            vad_threshold: 0.5,
            queue_capacity: 16,
            level_interval_ms: DEFAULT_INTERVAL_MS,
        }
    }
}

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

/// Licznik zaległości kolejki transkrypcji: liczba segmentów czekających w
/// kolejce plus ten aktualnie transkrybowany. [`Session::start`] dzieli go
/// (przez `Arc`) między wątek segmentujący, który zwiększa go o 1 po każdym
/// udanym wysłaniu segmentu do kolejki ([`dispatch_segment`]), a wątek
/// transkrypcji, który zmniejsza go o 1 po zakończeniu obsługi segmentu —
/// niezależnie od tego, czy wynik trafił do `on_segment`, czy został
/// odrzucony przez filtr. Segment porzucony z powodu pełnej kolejki nigdy
/// nie trafił do kolejki, więc licznika nie dotyczy. Przy każdej zmianie
/// woła `on_change` z nową wartością.
struct Backlog<G> {
    count: AtomicU32,
    on_change: G,
}

impl<G: Fn(u32)> Backlog<G> {
    fn new(on_change: G) -> Self {
        Self {
            count: AtomicU32::new(0),
            on_change,
        }
    }

    /// Nowy segment trafił do kolejki transkrypcji — zaległość rośnie o 1.
    fn inc(&self) {
        let new = self.count.fetch_add(1, Ordering::SeqCst) + 1;
        (self.on_change)(new);
    }

    /// Segment przestał być zaległością (obsłużony albo odrzucony przez
    /// filtr) — zaległość maleje o 1.
    fn dec(&self) {
        let new = self.count.fetch_sub(1, Ordering::SeqCst) - 1;
        (self.on_change)(new);
    }

    /// Wymusza `on_change(0)` niezależnie od bieżącej wartości. Wołane po
    /// zakończeniu pracy wątku transkrypcji, żeby okno na pewno nie zostało
    /// z „przetwarzam" na zawsze — także wtedy, gdy sesja nie miała ani
    /// jednego segmentu i licznik nigdy się nie zmienił.
    fn finish(&self) {
        self.count.store(0, Ordering::SeqCst);
        (self.on_change)(0);
    }
}

/// Gotowa wypowiedź po transkrypcji — to, co daemon rozgłasza jako
/// `segment.added` (`docs/02-protokol-ipc.md`).
///
/// `seq` jest monotoniczny i nadawany w chwili domknięcia segmentu przez
/// VAD, czyli **w kolejności wypowiadania**. Kolejność dotarcia do klienta
/// może być inna przy zaległościach kolejki, dlatego o miejscu w dokumencie
/// rozstrzyga oś czasu audio (`audio_start_ms`), a nie moment przybycia.
/// Dziura w numeracji oznacza wypowiedź porzuconą z powodu przepełnionej
/// kolejki — świadomie widoczną, a nie zamiecioną pod dywan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscribedSegment {
    pub seq: u64,
    pub text: String,
    pub audio_start_ms: u64,
    pub audio_end_ms: u64,
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
    /// `on_backlog` jest wołane za każdym razem, gdy zmienia się liczba
    /// zaległości kolejki transkrypcji (segmenty czekające plus ten aktualnie
    /// transkrybowany) — patrz [`Backlog`]. Musi być `Send + Sync + 'static`,
    /// bo jest dzielone między wątek segmentujący i wątek transkrypcji.
    /// `on_level` dostaje odczyt poziomu sygnału ~20 razy na sekundę —
    /// wskaźnik w oknie ma pokazać brak wychyleń natychmiast, zanim ktoś
    /// przedyktuje minutę do wyłączonego mikrofonu.
    pub fn start<F, G, L>(
        transcriber: Arc<dyn Transcribe>,
        config: SessionConfig,
        on_segment: F,
        on_backlog: G,
        on_level: L,
    ) -> Result<Self>
    where
        F: Fn(TranscribedSegment) + Send + 'static,
        G: Fn(u32) + Send + Sync + 'static,
        L: Fn(Level) + Send + 'static,
    {
        let vad = Vad::new()
            .context("nie udało się utworzyć detektora VAD")?
            .with_threshold(config.vad_threshold);
        let segmenter = Segmenter::new(config.segmenter);

        let (chunk_tx, chunk_rx) = mpsc::channel::<Vec<f32>>();
        let capture = Recorder::start_streaming(chunk_tx)
            .context("nie udało się uruchomić przechwytywania strumieniowego")?;

        let (segment_tx, segment_rx) = mpsc::sync_channel::<(u64, Segment, Instant)>(config.queue_capacity);
        let backlog = Arc::new(Backlog::new(on_backlog));

        let segmenter_backlog = Arc::clone(&backlog);
        let segmenter_handle = std::thread::spawn(move || {
            run_segmenter(
                chunk_rx,
                segment_tx,
                vad,
                segmenter,
                &segmenter_backlog,
                on_level,
                config.level_interval_ms,
            );
        });

        let transcriber_backlog = Arc::clone(&backlog);
        let transcriber_handle = std::thread::spawn(move || {
            run_transcriber(
                segment_rx,
                transcriber.as_ref(),
                on_segment,
                &transcriber_backlog,
            );
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
fn run_segmenter<G: Fn(u32), L: Fn(Level)>(
    chunk_rx: Receiver<Vec<f32>>,
    segment_tx: SyncSender<(u64, Segment, Instant)>,
    mut vad: Vad,
    mut segmenter: Segmenter,
    backlog: &Backlog<G>,
    on_level: L,
    level_interval_ms: u32,
) {
    let mut assembler = FrameAssembler::new();
    let mut meter = LevelMeter::new(level_interval_ms);
    let mut next_seq = 0u64;
    while let Ok(chunk) = chunk_rx.recv() {
        // Poziom liczymy z surowej porcji, przed złożeniem w ramki VAD —
        // wskaźnik ma pokazywać wejście, a nie to, co zdążył zobaczyć VAD.
        if let Some(level) = meter.push(&chunk) {
            on_level(level);
        }
        for frame in assembler.push(&chunk) {
            let is_speech = vad.is_speech(&frame);
            if let Some(segment) = segmenter.push(&frame, is_speech) {
                dispatch_segment(&segment_tx, next_seq, segment, backlog);
                next_seq += 1;
            }
        }
    }
    // Capture się zatrzymało (kanał rozłączony) — domykamy trwający segment,
    // jeśli jakiś był. Reszta niedopełniająca pełnej ramki w assemblerze
    // (poniżej 32 ms) przepada; to nieistotna strata na końcu nagrania.
    if let Some(segment) = segmenter.flush() {
        dispatch_segment(&segment_tx, next_seq, segment, backlog);
    }
    // `segment_tx` spada tu wraz z końcem funkcji — to sygnał dla wątku
    // transkrypcji, że nic więcej nie przyjdzie.
}

/// Wysyła gotowy segment do wątku transkrypcji, nie blokując się. Gdy kolejka
/// jest pełna (transkrypcja nie nadąża) albo odbiorca już zniknął, segment
/// jest porzucany z komunikatem na stderr — nigdy nie czekamy. Zaległość
/// ([`Backlog`]) rośnie tylko wtedy, gdy segment faktycznie trafił do
/// kolejki — porzucony segment nigdy w niej nie był, więc jej nie zwiększa.
fn dispatch_segment<G: Fn(u32)>(
    segment_tx: &SyncSender<(u64, Segment, Instant)>,
    seq: u64,
    segment: Segment,
    backlog: &Backlog<G>,
) {
    match segment_tx.try_send((seq, segment, Instant::now())) {
        Ok(()) => backlog.inc(),
        Err(mpsc::TrySendError::Full(_)) => {
            eprintln!(
                "sesja: kolejka transkrypcji pełna — segment porzucony (transkrypcja nie nadąża za mową)"
            );
        }
        Err(mpsc::TrySendError::Disconnected(_)) => {
            eprintln!("sesja: wątek transkrypcji już zakończony — segment porzucony");
        }
    }
}

/// Ciało wątku transkrypcji: bierze gotowe segmenty i woła `Transcriber`.
/// Kończy się, gdy `segment_rx` się rozłączy i kolejka jest pusta — czyli
/// dopiero po tym, jak wątek segmentujący skończył i domknął ostatni segment.
///
/// Wynik przechodzi przez [`filters::clean`] zanim trafi do `on_segment`:
/// whisper na ciszy albo szumie potrafi zmyślić tekst (znaczniki w rodzaju
/// „[muzyka]", formułki z napisów, samą interpunkcję) — taki tekst jest
/// odrzucany i tylko logowany, nie wstawiany do dokumentu.
///
/// Każdy odebrany segment przestaje być zaległością dopiero po zakończeniu
/// jego obsługi (`backlog.dec()`) — niezależnie od tego, czy trafił do
/// `on_segment`, czy odpadł w filtrze. Po zakończeniu pętli (kanał
/// rozłączony i pusty) `backlog.finish()` gwarantuje, że licznik wraca do 0 i
/// `on_backlog(0)` zostaje wywołane, nawet jeśli sesja nie miała ani jednego
/// segmentu.
fn run_transcriber<F, G: Fn(u32)>(
    segment_rx: Receiver<(u64, Segment, Instant)>,
    transcriber: &dyn Transcribe,
    on_segment: F,
    backlog: &Backlog<G>,
) where
    F: Fn(TranscribedSegment),
{
    while let Ok((seq, segment, queued_at)) = segment_rx.recv() {
        let waited = queued_at.elapsed();
        let started = Instant::now();
        let result = transcriber.transcribe(&segment.audio, "");
        // Diagnostyka rytmu: gdzie naprawdę schodzi czas między końcem
        // wypowiedzi a tekstem na ekranie. Oczekiwanie w kolejce rosnące
        // z segmentu na segment znaczy, że transkrypcja nie nadąża za mową
        // i progi w `[vad]` są dobrane pod za wolny backend.
        eprintln!(
            "sesja: segment {seq} — w kolejce {} ms, transkrypcja {} ms",
            waited.as_millis(),
            started.elapsed().as_millis()
        );
        match result {
            Ok(text) => match filters::clean(&text) {
                Some(cleaned) => on_segment(TranscribedSegment {
                    seq,
                    text: cleaned,
                    audio_start_ms: segment.start_ms(),
                    audio_end_ms: segment.end_ms(),
                }),
                None => {
                    // Pusty wynik transkrypcji (brak mowy w segmencie) jest
                    // normalny i nie jest logowany — logujemy tylko, gdy
                    // filtr faktycznie coś odrzucił.
                    if !text.trim().is_empty() {
                        eprintln!("sesja: odrzucono halucynację: {text}");
                    }
                }
            },
            Err(err) => {
                eprintln!("sesja: błąd transkrypcji segmentu: {err:#}");
            }
        }
        backlog.dec();
    }
    backlog.finish();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

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

    /// Segment testowy o zadanej wartości próbek — treść audio nie ma tu
    /// znaczenia, liczy się tylko to, czy trafił do kolejki.
    fn segment(value: f32) -> Segment {
        Segment {
            audio: vec![value],
            start_sample: 0,
            end_sample: 1,
        }
    }

    #[test]
    fn dispatch_segment_porzuca_gdy_kolejka_pelna_nie_blokujac() {
        let (tx, rx) = mpsc::sync_channel::<(u64, Segment, Instant)>(1);
        let backlog = Backlog::new(|_| {});
        dispatch_segment(&tx, 0, segment(1.0), &backlog);
        // Kolejka ma pojemność 1 i jest teraz pełna — drugie wysłanie musi
        // się nie zablokować, tylko porzucić segment.
        dispatch_segment(&tx, 1, segment(2.0), &backlog);
        let (seq, received, _queued_at) = rx.try_recv().unwrap();
        assert_eq!((seq, received), (0, segment(1.0)));
        assert!(rx.try_recv().is_err());
    }

    /// Zwraca `Backlog`, którego `on_change` dopisuje każdą kolejną wartość
    /// do współdzielonego wektora — pozwala sprawdzić nie tylko stan
    /// końcowy licznika, ale i historię wywołań callbacku.
    #[allow(clippy::type_complexity)]
    fn backlog_z_historia() -> (Backlog<impl Fn(u32)>, Rc<RefCell<Vec<u32>>>) {
        let history = Rc::new(RefCell::new(Vec::new()));
        let history_clone = Rc::clone(&history);
        let backlog = Backlog::new(move |n| history_clone.borrow_mut().push(n));
        (backlog, history)
    }

    #[test]
    fn backlog_rosnie_przy_udanym_wyslaniu_segmentu() {
        let (tx, _rx) = mpsc::sync_channel::<(u64, Segment, Instant)>(8);
        let (backlog, history) = backlog_z_historia();

        dispatch_segment(&tx, 0, segment(1.0), &backlog);
        dispatch_segment(&tx, 1, segment(2.0), &backlog);

        assert_eq!(backlog.count.load(Ordering::SeqCst), 2);
        assert_eq!(*history.borrow(), vec![1, 2]);
    }

    #[test]
    fn backlog_nie_rosnie_gdy_segment_porzucony_z_pelnej_kolejki() {
        let (tx, _rx) = mpsc::sync_channel::<(u64, Segment, Instant)>(1);
        let (backlog, history) = backlog_z_historia();

        dispatch_segment(&tx, 0, segment(1.0), &backlog); // trafia do kolejki, licznik = 1
        dispatch_segment(&tx, 1, segment(2.0), &backlog); // kolejka pełna, porzucony

        assert_eq!(backlog.count.load(Ordering::SeqCst), 1);
        // Callback wołany tylko raz — przy udanym wysłaniu.
        assert_eq!(*history.borrow(), vec![1]);
    }

    #[test]
    fn backlog_maleje_po_zakonczeniu_obslugi_segmentu() {
        let (backlog, history) = backlog_z_historia();

        backlog.inc();
        backlog.inc();
        backlog.dec();

        assert_eq!(backlog.count.load(Ordering::SeqCst), 1);
        assert_eq!(*history.borrow(), vec![1, 2, 1]);
    }

    #[test]
    fn backlog_finish_zeruje_i_zawsze_woła_on_change_z_zerem() {
        // Przypadek, w którym sesja nie miała ani jednego segmentu — licznik
        // nigdy się nie zmienił, ale `finish` i tak musi wywołać on_change(0),
        // żeby okno nie zostało z „przetwarzam" na zawsze.
        let (backlog, history) = backlog_z_historia();
        backlog.finish();
        assert_eq!(backlog.count.load(Ordering::SeqCst), 0);
        assert_eq!(*history.borrow(), vec![0]);

        // Przypadek typowy: po ostatnim dec() licznik już jest 0, finish()
        // i tak jawnie to potwierdza.
        let (backlog2, history2) = backlog_z_historia();
        backlog2.inc();
        backlog2.dec();
        backlog2.finish();
        assert_eq!(backlog2.count.load(Ordering::SeqCst), 0);
        assert_eq!(*history2.borrow(), vec![1, 0, 0]);
    }
}
