//! Skala wskaźnika poziomu sygnału.
//!
//! Wskaźnik pełni funkcję **diagnostyczną** (`docs/06-gui.md`): brak wychyleń
//! ma natychmiast pokazać, że wybrane urządzenie wejściowe jest nie to, co
//! trzeba — zanim ktoś przedyktuje minutę w ciszę.
//!
//! Dlatego skala jest decybelowa, a nie liniowa. Mowa przy normalnym
//! nagrywaniu ma wartość skuteczną rzędu 0,02–0,2, więc liniowy pasek stałby
//! przez cały czas przy lewej krawędzi i nie odróżniałby cichej mowy od
//! wyłączonego mikrofonu — czyli nie robiłby dokładnie tego jednego, do
//! czego służy.

/// Poniżej tego poziomu uznajemy, że nic nie słychać. −60 dBFS to szum
/// własny typowego mikrofonu USB.
const FLOOR_DB: f32 = -60.0;

/// Zamienia wartość skuteczną (0.0–1.0) na wychylenie wskaźnika (0.0–1.0)
/// w skali decybelowej.
pub fn meter_value(rms: f32) -> f64 {
    if rms <= 0.0 {
        return 0.0;
    }
    let db = 20.0 * rms.log10();
    let value = (db - FLOOR_DB) / -FLOOR_DB;
    value.clamp(0.0, 1.0) as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cisza_daje_zerowe_wychylenie() {
        assert_eq!(meter_value(0.0), 0.0);
        // Poniżej progu szumu też zero, a nie wartość ujemna.
        assert_eq!(meter_value(0.0001), 0.0);
    }

    #[test]
    fn peleny_sygnal_daje_pelne_wychylenie() {
        assert!((meter_value(1.0) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn cicha_mowa_jest_wyraznie_odrozniana_od_ciszy() {
        // Sedno sprawy: przy skali liniowej cicha mowa (rms 0,02) dałaby
        // 2% wychylenia, czyli wzrokowo to samo co wyłączony mikrofon.
        let cicha_mowa = meter_value(0.02);
        assert!(
            cicha_mowa > 0.25,
            "cicha mowa musi być widoczna, a wychylenie to {cicha_mowa}"
        );
    }

    #[test]
    fn glosniejszy_sygnal_zawsze_wychyla_bardziej() {
        let mut poprzednie = 0.0;
        for rms in [0.001, 0.01, 0.05, 0.2, 0.5, 1.0] {
            let wartosc = meter_value(rms);
            assert!(
                wartosc >= poprzednie,
                "wychylenie zmalało przy rms {rms}: {wartosc} < {poprzednie}"
            );
            poprzednie = wartosc;
        }
    }

    #[test]
    fn wychylenie_nigdy_nie_wychodzi_poza_zakres_paska() {
        for rms in [0.0, 1e-9, 0.5, 1.0, 2.0] {
            let wartosc = meter_value(rms);
            assert!((0.0..=1.0).contains(&wartosc), "rms {rms} dało {wartosc}");
        }
    }
}
