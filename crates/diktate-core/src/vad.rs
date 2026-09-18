//! Wykrywanie mowy (VAD).
//!
//! Zasada: model Silero VAD (przez `voice_activity_detector`) klasyfikuje
//! pojedynczą, samodzielną ramkę dźwięku — nie trzyma żadnej historii poza
//! wewnętrznym stanem sieci rekurencyjnej, który jest aktualizowany przy
//! każdym wywołaniu. Ten moduł jest więc tylko cienką warstwą: podaje ramkę
//! modelowi i porównuje zwrócone prawdopodobieństwo z progiem. Sklejanie
//! kolejnych decyzji w segmenty wypowiedzi to zadanie modułu `segmenter`,
//! nie tego pliku.

use anyhow::{Context, Result};
use voice_activity_detector::VoiceActivityDetector;

/// Rozmiar ramki wymagany przez model Silero przy 16 kHz (32 ms).
pub const CHUNK_SAMPLES: usize = 512;

/// Domyślny próg prawdopodobieństwa, powyżej którego ramka jest mową.
const DEFAULT_THRESHOLD: f32 = 0.5;

/// Detektor mowy działający na pojedynczych ramkach o stałym rozmiarze.
pub struct Vad {
    detector: VoiceActivityDetector,
    threshold: f32,
}

impl Vad {
    /// Tworzy detektor skonfigurowany na `crate::SAMPLE_RATE` i ramkę
    /// [`CHUNK_SAMPLES`], z domyślnym progiem 0.5.
    pub fn new() -> Result<Self> {
        let detector = VoiceActivityDetector::builder()
            .sample_rate(crate::SAMPLE_RATE as i64)
            .chunk_size(CHUNK_SAMPLES)
            .build()
            .context("nie udało się zbudować detektora VAD")?;
        Ok(Self {
            detector,
            threshold: DEFAULT_THRESHOLD,
        })
    }

    /// Ustawia próg decyzyjny (0.0–1.0) używany przez [`Vad::is_speech`].
    pub fn with_threshold(mut self, threshold: f32) -> Self {
        self.threshold = threshold;
        self
    }

    /// Zwraca surowe prawdopodobieństwo mowy w ramce, przydatne do strojenia
    /// progu i w testach. Ramka krótsza niż [`CHUNK_SAMPLES]` jest dopełniana
    /// zerami, dłuższa — obcinana; model nigdy nie panikuje na złym rozmiarze
    /// wejścia.
    pub fn probability(&mut self, chunk: &[f32]) -> f32 {
        self.detector.predict(chunk.iter().copied())
    }

    /// Klasyfikuje ramkę jako mowę na podstawie progu ustawionego przez
    /// [`Vad::with_threshold`] (domyślnie 0.5).
    pub fn is_speech(&mut self, chunk: &[f32]) -> bool {
        self.probability(chunk) >= self.threshold
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Dzieli próbki na ramki [`CHUNK_SAMPLES`]; ostatnia, niepełna ramka
    /// jest przekazywana tak jak jest — `Vad` sam ją dopełnia zerami.
    fn chunks(samples: &[f32]) -> impl Iterator<Item = &[f32]> {
        samples.chunks(CHUNK_SAMPLES)
    }

    #[test]
    fn cisza_nie_jest_mowa() {
        let mut vad = Vad::new().expect("budowa detektora");
        let silence = vec![0.0f32; CHUNK_SAMPLES * 20];
        for chunk in chunks(&silence) {
            assert!(!vad.is_speech(chunk), "cisza sklasyfikowana jako mowa");
        }
    }

    #[test]
    fn jfk_wav_jest_w_wiekszosci_mowa() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../testdata/jfk.wav");
        let mut reader = hound::WavReader::open(path).expect("nie znaleziono testdata/jfk.wav");
        let spec = reader.spec();
        assert_eq!(spec.sample_rate, crate::SAMPLE_RATE, "oczekiwano 16 kHz");
        assert_eq!(spec.channels, 1, "oczekiwano mono");

        let samples: Vec<f32> = reader
            .samples::<i16>()
            .map(|s| s.expect("błąd odczytu próbki") as f32 / 32768.0)
            .collect();

        let mut vad = Vad::new().expect("budowa detektora");
        let total = samples.chunks(CHUNK_SAMPLES).count();
        let speech = chunks(&samples).filter(|c| vad.is_speech(c)).count();
        let fraction = speech as f32 / total as f32;

        eprintln!("jfk.wav: {speech}/{total} ramek jako mowa ({fraction:.3})");

        // Zmierzony udział to ok. 0.60 (nagranie ma naturalne przerwy między
        // zdaniami oraz ciszę na początku/końcu pliku, więc to nie jest
        // 100%). Wymagamy wyraźnej większości, z zapasem w dół na wahania
        // modelu, i z góry, żeby test faktycznie coś sprawdzał.
        assert!(
            (0.4..=0.85).contains(&fraction),
            "udział ramek z mową poza oczekiwanym zakresem: {fraction:.3}"
        );
    }
}
