//! Poziom sygnału z mikrofonu — dla wskaźnika w oknie.
//!
//! Zasada: wskaźnik pełni funkcję **diagnostyczną** (`docs/06-gui.md`) —
//! brak wychyleń natychmiast pokazuje, że wybrane urządzenie wejściowe jest
//! nie to, co trzeba, zamiast pozwolić komuś dyktować w ciszę przez minutę.
//! Do tego nie potrzeba każdej porcji z `cpal`, tylko kilkunastu odczytów na
//! sekundę.
//!
//! Dławienie liczone jest **w próbkach, nie w zegarze ściennym**: strumień
//! ma stałą częstotliwość próbkowania, więc liczba próbek jest wierniejszą
//! miarą upływu czasu nagrania niż `Instant::now()`, a przy okazji czyni
//! moduł w pełni testowalnym bez czekania.

use crate::SAMPLE_RATE;

/// Domyślny odstęp między odczytami — ~20 Hz, tak jak w `docs/02`.
pub const DEFAULT_INTERVAL_MS: u32 = 50;

/// Jeden odczyt poziomu: wartość skuteczna i szczyt, oba w zakresie 0.0–1.0.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Level {
    pub rms: f32,
    pub peak: f32,
}

/// Zbiera próbki i co `interval_ms` zwraca uśredniony odczyt poziomu.
pub struct LevelMeter {
    interval_samples: usize,
    samples: usize,
    sum_squares: f64,
    peak: f32,
}

impl LevelMeter {
    pub fn new(interval_ms: u32) -> Self {
        Self {
            interval_samples: (interval_ms as u64 * SAMPLE_RATE as u64 / 1000) as usize,
            samples: 0,
            sum_squares: 0.0,
            peak: 0.0,
        }
    }

    /// Dokłada porcję próbek. Zwraca odczyt tylko wtedy, gdy od poprzedniego
    /// uzbierało się co najmniej `interval_ms` materiału.
    pub fn push(&mut self, chunk: &[f32]) -> Option<Level> {
        for &sample in chunk {
            self.sum_squares += (sample as f64) * (sample as f64);
            let abs = sample.abs();
            if abs > self.peak {
                self.peak = abs;
            }
        }
        self.samples += chunk.len();
        if self.samples < self.interval_samples || self.samples == 0 {
            return None;
        }
        let level = Level {
            rms: (self.sum_squares / self.samples as f64).sqrt() as f32,
            peak: self.peak,
        };
        self.samples = 0;
        self.sum_squares = 0.0;
        self.peak = 0.0;
        Some(level)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const INTERVAL_MS: u32 = 50;
    const INTERVAL_SAMPLES: usize = (SAMPLE_RATE as usize) * INTERVAL_MS as usize / 1000;

    #[test]
    fn odczyt_dopiero_po_uzbieraniu_pelnego_odstepu() {
        let mut meter = LevelMeter::new(INTERVAL_MS);
        assert!(meter.push(&vec![0.5; INTERVAL_SAMPLES - 1]).is_none());
        assert!(meter.push(&[0.5]).is_some());
    }

    #[test]
    fn rms_stalego_sygnalu_rowna_sie_jego_amplitudzie() {
        let mut meter = LevelMeter::new(INTERVAL_MS);
        let level = meter
            .push(&vec![0.5; INTERVAL_SAMPLES])
            .expect("pełny odstęp daje odczyt");
        assert!((level.rms - 0.5).abs() < 1e-5, "rms = {}", level.rms);
        assert!((level.peak - 0.5).abs() < 1e-5, "peak = {}", level.peak);
    }

    #[test]
    fn szczyt_lapie_pojedyncze_wychylenie_ktore_gubi_srednia() {
        let mut meter = LevelMeter::new(INTERVAL_MS);
        let mut chunk = vec![0.0; INTERVAL_SAMPLES];
        chunk[7] = -0.9;
        let level = meter.push(&chunk).expect("pełny odstęp daje odczyt");
        assert!((level.peak - 0.9).abs() < 1e-5, "peak = {}", level.peak);
        // Jedno wychylenie rozłożone na cały odstęp prawie znika w średniej —
        // i właśnie dlatego wskaźnik pokazuje obie wartości, nie samą średnią.
        assert!(
            level.rms < level.peak / 10.0,
            "rms = {}, peak = {}",
            level.rms,
            level.peak
        );
    }

    #[test]
    fn kolejny_odczyt_nie_pamieta_poprzedniego_szczytu() {
        // Bez zerowania wskaźnik raz wychylony zostawałby wychylony na
        // zawsze i przestałby cokolwiek diagnozować.
        let mut meter = LevelMeter::new(INTERVAL_MS);
        let mut glosny = vec![0.0; INTERVAL_SAMPLES];
        glosny[0] = 1.0;
        meter.push(&glosny).expect("pierwszy odczyt");

        let level = meter
            .push(&vec![0.0; INTERVAL_SAMPLES])
            .expect("drugi odczyt");
        assert_eq!(level.peak, 0.0);
        assert_eq!(level.rms, 0.0);
    }

    #[test]
    fn cisza_daje_odczyt_zerowy_a_nie_brak_odczytu() {
        // Brak wychyleń to najważniejsza informacja, jaką ten wskaźnik ma do
        // przekazania — nie wolno jej pominąć jako „nic się nie zmieniło".
        let mut meter = LevelMeter::new(INTERVAL_MS);
        let level = meter
            .push(&vec![0.0; INTERVAL_SAMPLES])
            .expect("cisza też jest odczytem");
        assert_eq!(level.rms, 0.0);
        assert_eq!(level.peak, 0.0);
    }
}
