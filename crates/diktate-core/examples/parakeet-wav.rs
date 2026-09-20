//! Diagnostyka backendu Parakeet bez mikrofonu: tnie plik WAV na kawałki
//! zadanej długości i transkrybuje każdy osobno, mierząc czas.
//!
//! Po co tnie: daemon nie transkrybuje całych nagrań, tylko segmenty
//! domknięte przez VAD. Dopiero pomiar kawałek po kawałku pokazuje to, co
//! w dyktowaniu naprawdę boli — opóźnienie pojedynczego segmentu i jakość
//! tekstu przy krótkim audio (model wymaga minimum 3 s, krótsze jest
//! dopełniane ciszą).
//!
//! cargo run -p diktate-core --example parakeet-wav -- plik.wav [sekundy_kawałka]
//!
//! Model wskazuje `$DIKTATE_PARAKEET_MODEL` albo ścieżka domyślna, liczbę
//! wątków — `$DIKTATE_PARAKEET_THREADS` (0 = tyle, ile widzi system).

use std::time::Instant;

use diktate_core::parakeet::default_parakeet_model_path;
use diktate_core::{ParakeetTranscriber, Transcribe, SAMPLE_RATE};

fn main() -> anyhow::Result<()> {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "testdata/jfk.wav".into());
    let chunk_s: f32 = std::env::args()
        .nth(2)
        .and_then(|a| a.parse().ok())
        .unwrap_or(5.0);
    let threads: u32 = std::env::var("DIKTATE_PARAKEET_THREADS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);

    let mut reader = hound::WavReader::open(&path)?;
    let spec = reader.spec();
    if spec.sample_rate != SAMPLE_RATE || spec.channels != 1 {
        anyhow::bail!(
            "plik ma {} Hz i {} kanał(y), a potok wymaga {SAMPLE_RATE} Hz mono i nie resampluje",
            spec.sample_rate,
            spec.channels
        );
    }
    let samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Int => reader
            .samples::<i16>()
            .map(|s| s.map(|v| v as f32 / i16::MAX as f32))
            .collect::<Result<_, _>>()?,
        hound::SampleFormat::Float => reader.samples::<f32>().collect::<Result<_, _>>()?,
    };

    let model = default_parakeet_model_path();
    let transcriber = ParakeetTranscriber::open(&model, threads)?;
    println!("model: {}", transcriber.model().display());
    println!(
        "{} wątków, minimum {:.1} s audio",
        transcriber.threads(),
        transcriber.min_audio_s()
    );
    println!(
        "materiał: {path} ({:.1} s), kawałki po {chunk_s:.1} s\n",
        samples.len() as f32 / SAMPLE_RATE as f32
    );

    let chunk_len = (chunk_s * SAMPLE_RATE as f32) as usize;
    let mut total_ms = 0u128;
    for (i, chunk) in samples.chunks(chunk_len.max(1)).enumerate() {
        let chunk_seconds = chunk.len() as f32 / SAMPLE_RATE as f32;
        let started = Instant::now();
        let text = transcriber.transcribe(chunk)?;
        let ms = started.elapsed().as_millis();
        total_ms += ms;
        println!("kawałek {} ({chunk_seconds:.1} s): {ms} ms — {text}", i + 1);
    }
    println!(
        "\nrazem {total_ms} ms na {:.1} s materiału ({:.1}× szybciej niż realtime)",
        samples.len() as f32 / SAMPLE_RATE as f32,
        samples.len() as f32 / SAMPLE_RATE as f32 / (total_ms as f32 / 1000.0)
    );
    Ok(())
}
