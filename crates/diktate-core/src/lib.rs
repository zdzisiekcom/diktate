//! Rdzeń: przechwytywanie dźwięku i transkrypcja.
//!
//! Etap 0 celowo nie ma VAD ani kolejki segmentów. Nagranie jest trzymane
//! w pamięci w całości i transkrybowane jednym przebiegiem po zatrzymaniu.

pub mod capture;
pub mod segmenter;
pub mod transcribe;
pub mod vad;

/// Częstotliwość, której oczekuje whisper.
pub const SAMPLE_RATE: u32 = 16_000;

pub use capture::{Recorder, Recording};
pub use transcribe::Transcriber;
