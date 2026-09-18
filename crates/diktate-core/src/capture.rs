//! Przechwytywanie z mikrofonu przez `cpal`.
//!
//! Strumień `cpal` nie jest `Send`, więc żyje na własnym wątku. Wątek jedynie
//! dopisuje próbki do wspólnego bufora i czeka na sygnał zatrzymania; cała
//! konwersja (miks do mono, zmiana częstotliwości) dzieje się dopiero przy
//! `stop`, poza ścieżką czasu rzeczywistego.

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
            run_stream(thread_buffer, ready_tx, stop_rx);
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

fn run_stream(
    buffer: Arc<Mutex<Vec<f32>>>,
    ready_tx: Sender<Result<(u16, u32)>>,
    stop_rx: Receiver<()>,
) {
    let stream = match build_stream(buffer) {
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

fn build_stream(buffer: Arc<Mutex<Vec<f32>>>) -> Result<(cpal::Stream, (u16, u32))> {
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

    let err_fn = |err| eprintln!("błąd strumienia wejściowego: {err}");

    let stream = match sample_format {
        SampleFormat::F32 => device.build_input_stream(
            &config,
            move |data: &[f32], _: &cpal::InputCallbackInfo| {
                if let Ok(mut guard) = buffer.lock() {
                    guard.extend_from_slice(data);
                }
            },
            err_fn,
            None,
        ),
        SampleFormat::I16 => device.build_input_stream(
            &config,
            move |data: &[i16], _: &cpal::InputCallbackInfo| {
                if let Ok(mut guard) = buffer.lock() {
                    guard.extend(data.iter().map(|s| *s as f32 / i16::MAX as f32));
                }
            },
            err_fn,
            None,
        ),
        SampleFormat::U16 => device.build_input_stream(
            &config,
            move |data: &[u16], _: &cpal::InputCallbackInfo| {
                if let Ok(mut guard) = buffer.lock() {
                    guard
                        .extend(data.iter().map(|s| {
                            (*s as f32 - u16::MAX as f32 / 2.0) / (u16::MAX as f32 / 2.0)
                        }));
                }
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
}
