//! Klient terminalowy.
//!
//! Etap 0 (0a): jedna komenda `record`, która omija daemona i woła rdzeń
//! bezpośrednio. Służy do sprawdzenia audio i whispera bez GTK i bez IPC.

mod model;
mod setup;

use anyhow::{anyhow, Context, Result};
use clap::{Parser, Subcommand};
use diktate_core::{transcribe::default_model_path, Recorder, Transcriber};
use std::path::PathBuf;
use std::time::{Duration, Instant};

#[derive(Parser)]
#[command(name = "diktate", version, about = "Dyktowanie lokalne")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Nagraj N sekund i wypisz transkrypcję.
    Record {
        /// Długość nagrania w sekundach.
        #[arg(short, long, default_value_t = 10)]
        seconds: u64,
        /// Ścieżka modelu ggml (domyślnie $DIKTATE_MODEL).
        #[arg(long)]
        model: Option<PathBuf>,
        /// Zamiast nagrywać z mikrofonu, wczytaj i ztranskrybuj plik WAV
        /// (mono, 16 kHz). Gdy podane, `--seconds` jest ignorowane.
        #[arg(long)]
        wav: Option<PathBuf>,
        /// Język transkrypcji przekazywany do whispera.
        #[arg(long, default_value = "pl")]
        language: String,
    },
    /// Modele whispera: lista i pobieranie.
    Model {
        #[command(subcommand)]
        action: ModelAction,
    },
}

#[derive(Subcommand)]
enum ModelAction {
    /// Wypisz znane modele i zaznacz te, które już masz.
    List,
    /// Pobierz model do ~/.diktate/models.
    Pull {
        /// Nazwa z `diktate model list`.
        name: String,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Record {
            seconds,
            model,
            wav,
            language,
        } => record(seconds, model, wav, language),
        Command::Model { action } => match action {
            ModelAction::List => model::cmd::list(),
            ModelAction::Pull { name } => model::cmd::pull(&name).map(|_| ()),
        },
    }
}

fn record(
    seconds: u64,
    model: Option<PathBuf>,
    wav: Option<PathBuf>,
    language: String,
) -> Result<()> {
    let model = model.unwrap_or_else(default_model_path);
    eprintln!("Wczytuję model {}…", model.display());
    let transcriber = Transcriber::new(&model)?.with_language(language);

    let samples = match wav {
        Some(path) => {
            eprintln!("Wczytuję WAV {}…", path.display());
            load_wav(&path)?
        }
        None => {
            eprintln!("Mów przez {seconds} s…");
            let recording = Recorder::start()?;
            std::thread::sleep(Duration::from_secs(seconds));
            let samples = recording.stop()?;
            eprintln!(
                "Nagrano {:.1} s, transkrybuję…",
                samples.len() as f32 / diktate_core::SAMPLE_RATE as f32
            );
            samples
        }
    };

    let started = Instant::now();
    let text = transcriber.transcribe(&samples)?;
    eprintln!("Gotowe w {:.1} s.", started.elapsed().as_secs_f32());
    println!("{text}");
    Ok(())
}

/// Wczytuje plik WAV i zwraca próbki mono f32 w 16 kHz.
///
/// Etap 0: brak resamplera, więc plik musi już być mono 16 kHz — inaczej
/// zwracamy czytelny błąd zamiast po cichu psuć dźwięk.
fn load_wav(path: &PathBuf) -> Result<Vec<f32>> {
    let mut reader = hound::WavReader::open(path)
        .with_context(|| format!("nie udało się otworzyć pliku WAV {}", path.display()))?;
    let spec = reader.spec();
    if spec.channels != 1 {
        return Err(anyhow!(
            "plik WAV {} ma {} kanałów, oczekiwano mono (1)",
            path.display(),
            spec.channels
        ));
    }
    if spec.sample_rate != diktate_core::SAMPLE_RATE {
        return Err(anyhow!(
            "plik WAV {} ma częstotliwość {} Hz, oczekiwano {} Hz",
            path.display(),
            spec.sample_rate,
            diktate_core::SAMPLE_RATE
        ));
    }

    let samples: Result<Vec<f32>, _> = match (spec.sample_format, spec.bits_per_sample) {
        (hound::SampleFormat::Float, 32) => reader.samples::<f32>().collect(),
        (hound::SampleFormat::Int, 16) => reader
            .samples::<i16>()
            .map(|s| s.map(|v| v as f32 / i16::MAX as f32))
            .collect(),
        (format, bits) => {
            return Err(anyhow!(
                "nieobsługiwany format próbek WAV: {format:?} {bits} bit"
            ))
        }
    };
    samples.with_context(|| format!("nie udało się odczytać próbek z {}", path.display()))
}
