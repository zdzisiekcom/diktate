//! Przechwytywanie z mikrofonu przez `cpal`.
//!
//! Strumień `cpal` nie jest `Send`, więc żyje na własnym wątku. Wątek jedynie
//! dopisuje próbki do wspólnego bufora i czeka na sygnał zatrzymania; cała
//! konwersja (miks do mono, zmiana częstotliwości) dzieje się dopiero przy
//! `stop`, poza ścieżką czasu rzeczywistego.
//!
//! Obok tego jest wariant strumieniowy (`Recorder::start_streaming`) dla
//! `Session`: tu konwersja (miks do mono, resampling do 16 kHz — te same
//! funkcje `to_mono`/`resample`) dzieje się już w callbacku audio, a gotowe
//! porcje lecą od razu kanałem `mpsc`. Callback tylko konwertuje i wysyła —
//! `mpsc::Sender::send` na kanale nieograniczonym nie blokuje, więc ścieżka
//! czasu rzeczywistego nigdy nie czeka na odbiorcę.

use anyhow::{anyhow, Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::SampleFormat;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use crate::SAMPLE_RATE;

/// Surowe próbki w formacie urządzenia, wraz z jego parametrami.
struct Captured {
    samples: Vec<f32>,
    channels: u16,
    sample_rate: u32,
}

pub struct Recorder;

impl Recorder {
    /// Otwiera domyślne urządzenie wejściowe i zaczyna nagrywać.
    pub fn start() -> Result<Recording> {
        let buffer: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::new()));
        let (stop_tx, stop_rx) = mpsc::channel::<()>();
        let (ready_tx, ready_rx) = mpsc::channel::<Result<(u16, u32)>>();

        let thread_buffer = Arc::clone(&buffer);
        let handle = std::thread::spawn(move || {
            run_stream(
                |_channels, _sample_rate| {
                    move |data: &[f32]| {
                        if let Ok(mut guard) = thread_buffer.lock() {
                            guard.extend_from_slice(data);
                        }
                    }
                },
                ready_tx,
                stop_rx,
            );
        });

        let (channels, sample_rate) = ready_rx
            .recv()
            .context("wątek nagrywania zakończył się przed otwarciem strumienia")??;

        Ok(Recording {
            buffer,
            stop_tx,
            handle: Some(handle),
            channels,
            sample_rate,
        })
    }

    /// Jak [`Recorder::start`], ale zamiast buforować całe nagranie wysyła
    /// kolejne porcje mono 16 kHz kanałem `tx`, w miarę jak nadchodzą z
    /// urządzenia. Miks do mono i resampling dzieją się w callbacku audio
    /// (funkcje [`to_mono`]/[`resample`], te same co w wariancie buforującym).
    /// Callback tylko konwertuje i wysyła — nigdy nie czeka i nie blokuje.
    pub fn start_streaming(tx: Sender<Vec<f32>>) -> Result<StreamingRecording> {
        let (stop_tx, stop_rx) = mpsc::channel::<()>();
        let (ready_tx, ready_rx) = mpsc::channel::<Result<(u16, u32)>>();

        let handle = std::thread::spawn(move || {
            run_stream(
                move |channels, sample_rate| {
                    // Jeden resampler na cały strumień — przenosi ułamkową
                    // pozycję i ostatnią próbkę między kolejnymi porcjami z
                    // callbacku, więc sklejenie porcji nie gubi fazy ani
                    // nie przycina końcówek na granicach porcji.
                    let mut resampler = Resampler::new(sample_rate, SAMPLE_RATE);
                    move |data: &[f32]| {
                        let mono = to_mono(data, channels);
                        let resampled = resampler.push(&mono);
                        if !resampled.is_empty() {
                            // Odbiorca (wątek segmentujący `Session`) mógł już
                            // zakończyć pracę — wtedy po prostu przestajemy
                            // wysyłać, strumień i tak zaraz zostanie zatrzymany.
                            let _ = tx.send(resampled);
                        }
                    }
                },
                ready_tx,
                stop_rx,
            );
        });

        ready_rx
            .recv()
            .context("wątek nagrywania zakończył się przed otwarciem strumienia")??;

        Ok(StreamingRecording {
            stop_tx,
            handle: Some(handle),
        })
    }
}

/// Trwające nagranie. `stop` zwraca próbki gotowe dla whispera.
pub struct Recording {
    buffer: Arc<Mutex<Vec<f32>>>,
    stop_tx: Sender<()>,
    handle: Option<JoinHandle<()>>,
    channels: u16,
    sample_rate: u32,
}

impl Recording {
    /// Liczba próbek zebranych do tej pory (w formacie urządzenia).
    pub fn frames_captured(&self) -> usize {
        self.buffer.lock().map(|b| b.len()).unwrap_or(0)
    }

    /// Zatrzymuje strumień i zwraca mono 16 kHz.
    pub fn stop(mut self) -> Result<Vec<f32>> {
        let _ = self.stop_tx.send(());
        if let Some(handle) = self.handle.take() {
            handle
                .join()
                .map_err(|_| anyhow!("wątek nagrywania spanikował"))?;
        }
        let samples = {
            let guard = self
                .buffer
                .lock()
                .map_err(|_| anyhow!("bufor nagrania uszkodzony przez panikę"))?;
            guard.clone()
        };
        Ok(prepare(Captured {
            samples,
            channels: self.channels,
            sample_rate: self.sample_rate,
        }))
    }
}

/// Trwające nagranie strumieniowe uruchomione przez [`Recorder::start_streaming`].
/// W odróżnieniu od [`Recording`] nie trzyma żadnego bufora próbek — te
/// płyną na bieżąco kanałem przekazanym przy starcie.
pub struct StreamingRecording {
    stop_tx: Sender<()>,
    handle: Option<JoinHandle<()>>,
}

impl StreamingRecording {
    /// Zatrzymuje strumień wejściowy. Po powrocie z tej funkcji urządzenie
    /// jest zamknięte i nadawca porcji (kanał podany do `start_streaming`)
    /// zostaje odłączony — odbiorca dostanie o tym znać przez rozłączenie
    /// kanału (`recv` zwróci błąd).
    pub fn stop(mut self) -> Result<()> {
        let _ = self.stop_tx.send(());
        if let Some(handle) = self.handle.take() {
            handle
                .join()
                .map_err(|_| anyhow!("wątek nagrywania spanikował"))?;
        }
        Ok(())
    }
}

/// Uruchamia strumień zbudowany przez `make_sink` i czeka na sygnał stopu.
/// `make_sink` dostaje parametry urządzenia (kanały, częstotliwość) i zwraca
/// callback wywoływany dla każdej porcji próbek w formacie urządzenia —
/// wspólne dla wariantu buforującego i strumieniowego.
fn run_stream<F, S>(make_sink: F, ready_tx: Sender<Result<(u16, u32)>>, stop_rx: Receiver<()>)
where
    F: FnOnce(u16, u32) -> S,
    S: FnMut(&[f32]) + Send + 'static,
{
    let stream = match build_stream(make_sink) {
        Ok((stream, params)) => {
            let _ = ready_tx.send(Ok(params));
            stream
        }
        Err(err) => {
            let _ = ready_tx.send(Err(err));
            return;
        }
    };

    // Strumień gra, dopóki żyje. Czekamy na sygnał stopu, potem go upuszczamy.
    let _ = stop_rx.recv();
    drop(stream);
}

fn build_stream<F, S>(make_sink: F) -> Result<(cpal::Stream, (u16, u32))>
where
    F: FnOnce(u16, u32) -> S,
    S: FnMut(&[f32]) + Send + 'static,
{
    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .ok_or_else(|| anyhow!("brak domyślnego urządzenia wejściowego"))?;
    let supported = device
        .default_input_config()
        .context("nie udało się odczytać domyślnej konfiguracji wejścia")?;

    let channels = supported.channels();
    let sample_rate = supported.sample_rate().0;
    let sample_format = supported.sample_format();
    let config: cpal::StreamConfig = supported.into();

    let mut sink = make_sink(channels, sample_rate);
    let err_fn = |err| eprintln!("błąd strumienia wejściowego: {err}");

    let stream = match sample_format {
        SampleFormat::F32 => device.build_input_stream(
            &config,
            move |data: &[f32], _: &cpal::InputCallbackInfo| sink(data),
            err_fn,
            None,
        ),
        SampleFormat::I16 => device.build_input_stream(
            &config,
            move |data: &[i16], _: &cpal::InputCallbackInfo| {
                let converted: Vec<f32> =
                    data.iter().map(|s| *s as f32 / i16::MAX as f32).collect();
                sink(&converted);
            },
            err_fn,
            None,
        ),
        SampleFormat::U16 => device.build_input_stream(
            &config,
            move |data: &[u16], _: &cpal::InputCallbackInfo| {
                let converted: Vec<f32> = data
                    .iter()
                    .map(|s| (*s as f32 - u16::MAX as f32 / 2.0) / (u16::MAX as f32 / 2.0))
                    .collect();
                sink(&converted);
            },
            err_fn,
            None,
        ),
        other => return Err(anyhow!("nieobsługiwany format próbek: {other:?}")),
    }
    .context("nie udało się otworzyć strumienia wejściowego")?;

    stream
        .play()
        .context("nie udało się uruchomić strumienia")?;
    Ok((stream, (channels, sample_rate)))
}

/// Nazwy urządzeń wejściowych widzianych przez cpal.
///
/// Kreator wymienia je przy pytaniu o mikrofon. Brak serwera dźwięku nie
/// jest tu błędem krytycznym — lista bywa pusta i wtedy pytamy o nazwę
/// tekstem.
pub fn input_device_names() -> Result<Vec<String>> {
    let host = cpal::default_host();
    let devices = host
        .input_devices()
        .context("nie udało się wymienić urządzeń wejściowych")?;
    Ok(devices.filter_map(|d| d.name().ok()).collect())
}

/// Miks do mono i zmiana częstotliwości na 16 kHz.
fn prepare(captured: Captured) -> Vec<f32> {
    let mono = to_mono(&captured.samples, captured.channels);
    resample(&mono, captured.sample_rate, SAMPLE_RATE)
}

fn to_mono(samples: &[f32], channels: u16) -> Vec<f32> {
    if channels <= 1 {
        return samples.to_vec();
    }
    let channels = channels as usize;
    samples
        .chunks_exact(channels)
        .map(|frame| frame.iter().sum::<f32>() / channels as f32)
        .collect()
}

/// Interpolacja liniowa. Do etapu 0 wystarcza; docelowo `rubato`.
fn resample(input: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || input.is_empty() {
        return input.to_vec();
    }
    let ratio = from as f64 / to as f64;
    let out_len = ((input.len() as f64) / ratio).floor() as usize;
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let pos = i as f64 * ratio;
        let left = pos.floor() as usize;
        let frac = (pos - left as f64) as f32;
        let a = input[left];
        let b = *input.get(left + 1).unwrap_or(&a);
        out.push(a + (b - a) * frac);
    }
    out
}

/// Resampler ze stanem — ta sama interpolacja liniowa co [`resample`], ale
/// dla strumienia porcji zamiast całego nagrania naraz. Przenosi między
/// wywołaniami `push` ułamkową pozycję wyjścia oraz ostatnią próbkę
/// poprzedniej porcji, więc sklejenie wyników kolejnych `push` odpowiada
/// (z dokładnością do błędu zmiennoprzecinkowego) resamplingowi całości za
/// jednym razem — z wyjątkiem samego końca strumienia: `resample` dokleja
/// tam ostatnią próbkę zamiast czekać na kolejną, a strumieniowy resampler
/// po prostu nie emituje tej ostatniej, niekompletnej próbki (bo kolejna
/// porcja już nie nadejdzie). Stąd długość wyniku może się różnić o co
/// najwyżej jedną próbkę.
struct Resampler {
    ratio: f64,
    /// `from == to` — wtedy przepuszczamy dane bez zmian, bez liczenia pozycji.
    identity: bool,
    /// Indeks kolejnej próbki wyjściowej do policzenia.
    next_out: u64,
    /// Ile próbek wejściowych podano łącznie przed bieżącą porcją.
    offset: u64,
    /// Ostatnia próbka poprzedniej niepustej porcji — potrzebna, gdy pozycja
    /// wypada tuż przed bieżącą porcją (na granicy porcji).
    prev_last: Option<f32>,
}

impl Resampler {
    fn new(from: u32, to: u32) -> Self {
        Self {
            ratio: from as f64 / to as f64,
            identity: from == to,
            next_out: 0,
            offset: 0,
            prev_last: None,
        }
    }

    /// Zamienia kolejną porcję próbek wejściowych na próbki w docelowej
    /// częstotliwości, kontynuując pozycję z poprzednich wywołań.
    fn push(&mut self, input: &[f32]) -> Vec<f32> {
        if self.identity {
            return input.to_vec();
        }
        if input.is_empty() {
            return Vec::new();
        }

        // Wartości potrzebne w domknięciu — kopiujemy, żeby domknięcie nie
        // pożyczało `self` (dalej trzeba by mutować `self.next_out`).
        let offset = self.offset;
        let prev_last = self.prev_last;
        let mut next_out = self.next_out;

        let sample_at = |idx: u64| -> Option<f32> {
            if idx >= offset {
                input.get((idx - offset) as usize).copied()
            } else if idx + 1 == offset {
                prev_last
            } else {
                // Pozycja dalej w tyle niż poprzednia próbka — przy naszym
                // sposobie wołania to się nie zdarza.
                None
            }
        };

        let mut out = Vec::new();
        loop {
            let pos = next_out as f64 * self.ratio;
            let left = pos.floor() as u64;
            let frac = (pos - left as f64) as f32;
            let (a, b) = match (sample_at(left), sample_at(left + 1)) {
                (Some(a), Some(b)) => (a, b),
                // Brakuje kolejnej próbki (jeszcze nie nadeszła) — czekamy
                // na następną porcję zamiast przycinać albo dublować.
                _ => break,
            };
            out.push(a + (b - a) * frac);
            next_out += 1;
        }

        self.next_out = next_out;
        self.offset = offset + input.len() as u64;
        self.prev_last = input.last().copied();
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mono_averages_channels() {
        let stereo = [1.0, 0.0, 0.5, 0.5];
        assert_eq!(to_mono(&stereo, 2), vec![0.5, 0.5]);
    }

    #[test]
    fn mono_passes_single_channel_through() {
        let mono = [0.1, 0.2, 0.3];
        assert_eq!(to_mono(&mono, 1), mono.to_vec());
    }

    #[test]
    fn resample_shortens_by_ratio() {
        let input: Vec<f32> = (0..48_000).map(|i| i as f32).collect();
        let out = resample(&input, 48_000, 16_000);
        assert_eq!(out.len(), 16_000);
        assert!((out[1] - 3.0).abs() < 1e-3);
    }

    #[test]
    fn resample_is_identity_for_equal_rates() {
        let input = vec![0.1, 0.2, 0.3];
        assert_eq!(resample(&input, 16_000, 16_000), input);
    }

    #[test]
    fn resampler_streaming_is_identity_for_equal_rates() {
        // Równe częstotliwości — dane mają przejść bez zmiany, też w
        // wariancie strumieniowym i mimo dzielenia na porcje.
        let mut r = Resampler::new(16_000, 16_000);
        assert_eq!(r.push(&[0.1, 0.2, 0.3]), vec![0.1, 0.2, 0.3]);
        assert_eq!(r.push(&[0.4, 0.5]), vec![0.4, 0.5]);
        assert_eq!(r.push(&[]), Vec::<f32>::new());
    }

    #[test]
    fn resampler_streaming_matches_whole_for_uneven_chunks() {
        // Sygnał nieliniowy (sinus) — dla rampy liniowej interpolacja byłaby
        // dokładna niezależnie od błędu fazy, więc nie wykryłaby regresji.
        let chunk_lens = [100usize, 997, 3, 5000];
        let total: usize = chunk_lens.iter().sum();
        let input: Vec<f32> = (0..total).map(|i| (i as f32 * 0.073).sin()).collect();

        let whole = resample(&input, 44_100, 16_000);

        let mut resampler = Resampler::new(44_100, 16_000);
        let mut streamed = Vec::new();
        let mut pos = 0;
        for len in chunk_lens {
            streamed.extend(resampler.push(&input[pos..pos + len]));
            pos += len;
        }

        // Ta sama długość co resampling całości naraz, z tolerancją jednej
        // próbki na samym końcu strumienia (patrz komentarz przy `Resampler`).
        let diff = whole.len() as i64 - streamed.len() as i64;
        assert!(
            diff.abs() <= 1,
            "różnica długości poza tolerancją: whole={}, streamed={}",
            whole.len(),
            streamed.len()
        );

        let common = whole.len().min(streamed.len());
        for i in 0..common {
            let err = (whole[i] - streamed[i]).abs();
            assert!(
                err < 1e-4,
                "rozjazd na próbce {i}: whole={}, streamed={}, err={err}",
                whole[i],
                streamed[i]
            );
        }
    }
}
