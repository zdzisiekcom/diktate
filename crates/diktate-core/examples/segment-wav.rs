//! Diagnostyka segmentacji: przepuszcza plik WAV (mono 16 kHz) przez VAD
//! i segmenter, wypisując, gdzie i dlaczego segmenty się domykają.
//!
//! cargo run -p diktate-core --example segment-wav -- testdata/jfk.wav

use diktate_core::segmenter::{Segmenter, SegmenterConfig};
use diktate_core::vad::{Vad, CHUNK_SAMPLES};
use diktate_core::SAMPLE_RATE;

fn main() -> anyhow::Result<()> {
    let path = std::env::args().nth(1).unwrap_or_else(|| "testdata/jfk.wav".into());
    let mut reader = hound::WavReader::open(&path)?;
    let spec = reader.spec();
    let samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Int => reader
            .samples::<i16>()
            .map(|s| s.map(|v| v as f32 / i16::MAX as f32))
            .collect::<Result<_, _>>()?,
        hound::SampleFormat::Float => reader.samples::<f32>().collect::<Result<_, _>>()?,
    };
    println!(
        "{path}: {} kanał(ów), {} Hz, {:.1} s",
        spec.channels,
        spec.sample_rate,
        samples.len() as f32 / spec.sample_rate as f32
    );

    let mut vad = Vad::new()?;
    let mut segmenter = Segmenter::new(SegmenterConfig::default());
    let mut speech_frames = 0usize;
    let mut total_frames = 0usize;
    let mut emitted = 0usize;

    for (i, chunk) in samples.chunks(CHUNK_SAMPLES).enumerate() {
        if chunk.len() < CHUNK_SAMPLES {
            break;
        }
        let is_speech = vad.is_speech(chunk);
        total_frames += 1;
        if is_speech {
            speech_frames += 1;
        }
        if let Some(segment) = segmenter.push(chunk, is_speech) {
            emitted += 1;
            println!(
                "segment #{emitted} domknięty na ramce {i} ({:.2} s od początku): {:.2} s materiału",
                (i * CHUNK_SAMPLES) as f32 / SAMPLE_RATE as f32,
                segment.len() as f32 / SAMPLE_RATE as f32
            );
        }
    }
    if let Some(segment) = segmenter.flush() {
        emitted += 1;
        println!(
            "segment #{emitted} domknięty dopiero przez flush(): {:.2} s materiału",
            segment.len() as f32 / SAMPLE_RATE as f32
        );
    }
    println!(
        "ramki z mową: {speech_frames}/{total_frames} ({:.1}%), segmentów: {emitted}",
        100.0 * speech_frames as f32 / total_frames as f32
    );
    Ok(())
}
