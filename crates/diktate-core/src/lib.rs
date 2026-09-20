//! Rdzeń: przechwytywanie dźwięku, VAD, segmentacja i transkrypcja.
//!
//! Poza wariantem z etapu 0 (całe nagranie w pamięci, transkrybowane jednym
//! przebiegiem po zatrzymaniu — `Recorder::start`/`Recording::stop`, wciąż
//! używane przez CLI) jest teraz `session`: strumieniowy potok spinający
//! przechwytywanie, VAD, segmenter i transkrypcję w jedną, trwającą sesję.

pub mod capture;
pub mod filters;
pub mod level;
pub mod parakeet;
pub mod prompt;
pub mod segmenter;
pub mod session;
pub mod transcribe;
pub mod vad;

/// Częstotliwość, której oczekują oba modele — whisper i Parakeet
/// (`docs/12-backendy-transkrypcji.md`). Żaden nie resampluje.
pub const SAMPLE_RATE: u32 = 16_000;

pub use capture::{input_device_names, Recorder, Recording};
pub use session::{Session, SessionConfig};
pub use parakeet::ParakeetTranscriber;
pub use transcribe::{Transcribe, Transcriber};
