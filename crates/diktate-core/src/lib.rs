//! Rdzeń: przechwytywanie dźwięku, VAD, segmentacja i transkrypcja.
//!
//! Poza wariantem z etapu 0 (całe nagranie w pamięci, transkrybowane jednym
//! przebiegiem po zatrzymaniu — `Recorder::start`/`Recording::stop`, wciąż
//! używane przez CLI) jest teraz `session`: strumieniowy potok spinający
//! przechwytywanie, VAD, segmenter i transkrypcję w jedną, trwającą sesję.

pub mod capture;
pub mod filters;
pub mod segmenter;
pub mod session;
pub mod transcribe;
pub mod vad;

/// Częstotliwość, której oczekuje whisper.
pub const SAMPLE_RATE: u32 = 16_000;

pub use capture::{Recorder, Recording};
pub use session::Session;
pub use transcribe::Transcriber;
