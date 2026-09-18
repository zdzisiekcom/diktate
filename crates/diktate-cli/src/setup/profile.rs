//! Rytm segmentacji jako jeden wybór zamiast trzech liczb.
//!
//! Progi VAD w oderwaniu od siebie nie znaczą nic: skrócenie pauzy nie
//! przyspiesza niczego, dopóki próg długości segmentu zostaje wysoki
//! (`docs/04`, „Dlaczego progi VAD nazywają się właśnie tak”). Dlatego krótka
//! ścieżka kreatora pyta o profil, a nie o pola.

use diktate_config::{DaemonConfig, VadConfig};

pub struct Rhythm {
    /// Na razie czytane tylko przez testy i przyszły dialog kreatora (to
    /// zadanie dostarcza jądro bez interfejsu).
    #[allow(dead_code)]
    pub name: &'static str,
    pub long_segment_ms: u32,
    pub silence_ms_to_end_long: u32,
    pub silence_ms_to_end: u32,
    /// Na razie wyświetlane tylko przez przyszły dialog kreatora (to
    /// zadanie dostarcza jądro bez interfejsu).
    #[allow(dead_code)]
    pub opis: &'static str,
}

pub const RHYTHMS: &[Rhythm] = &[
    Rhythm {
        name: "szybki",
        long_segment_ms: 2_000,
        silence_ms_to_end_long: 200,
        silence_ms_to_end: 1_200,
        opis: "tekst pojawia się najszybciej; wymaga szybkiego modelu",
    },
    Rhythm {
        name: "zrównoważony",
        long_segment_ms: 2_500,
        silence_ms_to_end_long: 200,
        silence_ms_to_end: 1_500,
        opis: "domyślny — sprawdzony dla small-q5_1",
    },
    Rhythm {
        name: "dokładny",
        long_segment_ms: 4_000,
        silence_ms_to_end_long: 400,
        silence_ms_to_end: 2_000,
        opis: "dłuższe segmenty, więcej kontekstu dla modelu",
    },
];

/// Na razie wołane tylko przez testy i przyszły dialog kreatora (to
/// zadanie dostarcza jądro bez interfejsu).
#[allow(dead_code)]
pub fn apply(vad: &mut VadConfig, rhythm: &Rhythm) {
    vad.long_segment_ms = rhythm.long_segment_ms;
    vad.silence_ms_to_end_long = rhythm.silence_ms_to_end_long;
    vad.silence_ms_to_end = rhythm.silence_ms_to_end;
}

/// Który profil odpowiada tym progom — `None` znaczy „ustawione ręcznie”
/// i jest odpowiedzią pełnoprawną, nie błędem.
pub fn detect(vad: &VadConfig) -> Option<&'static Rhythm> {
    RHYTHMS.iter().find(|r| {
        r.long_segment_ms == vad.long_segment_ms
            && r.silence_ms_to_end_long == vad.silence_ms_to_end_long
            && r.silence_ms_to_end == vad.silence_ms_to_end
    })
}

/// Koszt jednego segmentu to około 62% czasu przebiegu mierzonego na 20 s
/// materiału: dla `small-q5_1` pomiar daje 4,5 s przebiegu i ~2,8 s na
/// segment (`CLAUDE.md`, „Wydajność transkrypcji”). Whisper liczy zawsze
/// pełne okno 30 s, więc koszt prawie nie zależy od długości segmentu.
const COST_SHARE_PERCENT: u64 = 62;

/// O tyle segment jest dłuższy niż sam próg: dochodzi pauza domykająca
/// i pre-roll. Próg 2500 ms daje w pomiarach segmenty ~3,4 s (`docs/04`).
const SEGMENT_OVERHEAD_MS: u32 = 700;

/// Granica przepustowości: powyżej niej kolejka rośnie, a tekst pojawia się
/// później, nie wcześniej.
const THROUGHPUT_LIMIT_PERCENT: u64 = 85;

/// Czy przy modelu o takim czasie przebiegu ten profil przewróci kolejkę.
///
/// `run_ms` to czas przebiegu na 20 s materiału z katalogu modeli, nie
/// koszt pojedynczego segmentu — te dwie liczby różnią się (`COST_SHARE_PERCENT`
/// przelicza jedną w drugą). Rzeczywisty czas trwania segmentu jest też
/// dłuższy niż sam `long_segment_ms` — dochodzi pauza domykająca i pre-roll
/// (`SEGMENT_OVERHEAD_MS`). Kolejka rośnie, gdy koszt zbliża się do
/// rzeczywistej długości segmentu; `THROUGHPUT_LIMIT_PERCENT` to granica
/// z `CLAUDE.md` („Dobieranie `long_segment_ms`”).
///
/// Na razie wołane tylko przez testy i przyszły dialog kreatora.
#[allow(dead_code)]
pub fn too_fast_for(rhythm: &Rhythm, run_ms: u32) -> bool {
    let segment_ms =
        rhythm.long_segment_ms + rhythm.silence_ms_to_end_long + SEGMENT_OVERHEAD_MS;
    let cost_ms = run_ms as u64 * COST_SHARE_PERCENT / 100;
    cost_ms * 100 > segment_ms as u64 * THROUGHPUT_LIMIT_PERCENT
}

/// Wygodny skrót dla dialogu: profil ustawiony w całej konfiguracji.
///
/// Na razie używane tylko przez przyszły dialog kreatora (to zadanie
/// dostarcza jądro bez interfejsu) — stąd `#[allow(dead_code)]`, tak jak
/// przy `model::catalog::recommended`.
#[allow(dead_code)]
pub fn current(cfg: &DaemonConfig) -> Option<&'static Rhythm> {
    detect(&cfg.vad)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn każdy_profil_przechodzi_walidację_daemona() {
        // Kreator nie może zaproponować ustawień, których daemon nie przyjmie.
        for rhythm in RHYTHMS {
            let mut cfg = DaemonConfig::default();
            apply(&mut cfg.vad, rhythm);
            let toml = toml::to_string(&cfg).unwrap();
            DaemonConfig::from_toml(&toml)
                .unwrap_or_else(|e| panic!("profil {} odrzucony: {e}", rhythm.name));
        }
    }

    #[test]
    fn rozpoznaje_własny_profil_po_progach() {
        let mut vad = VadConfig::default();
        apply(&mut vad, &RHYTHMS[2]);
        assert_eq!(detect(&vad).map(|r| r.name), Some(RHYTHMS[2].name));
    }

    #[test]
    fn ręcznie_ustawione_progi_to_profil_własny() {
        // Plik napisany ręcznie nie może zostać po cichu podmieniony
        // na najbliższy profil.
        let vad = VadConfig {
            long_segment_ms: 3_333,
            ..VadConfig::default()
        };
        assert!(detect(&vad).is_none());
    }

    #[test]
    fn szybki_profil_jest_za_szybki_dla_wolnego_modelu() {
        // small-q5_1 liczy segment ~2,8 s; przy progu 2000 ms segment ma
        // ~2,9 s, więc kolejka zaczyna rosnąć (CLAUDE.md, „Wydajność”).
        let szybki = &RHYTHMS[0];
        assert!(too_fast_for(szybki, 4_500));
        assert!(!too_fast_for(szybki, 1_200));
    }

    #[test]
    fn zalecany_zestaw_nie_budzi_ostrzeżenia() {
        // small-q5_1 (4,5 s przebiegu na 20 s materiału) z profilem
        // domyślnym „zrównoważony” to zestaw, który `docs/04` opisuje jako
        // sprawdzony — kreator nie może ostrzegać przed własnym zaleceniem.
        assert!(!too_fast_for(&RHYTHMS[1], 4_500));
    }

    #[test]
    fn duży_model_ostrzega_nawet_przy_dokładnym_rytmie() {
        // large-v3-turbo-q5_0 (14–20 s przebiegu na 20 s materiału) jest za
        // wolny nawet dla najbardziej wyrozumiałego profilu.
        assert!(too_fast_for(&RHYTHMS[2], 15_500));
    }
}
