//! Testy walidacji konfiguracji.
//!
//! Zasada z `docs/04-konfiguracja.md`: błąd walidacji to **odmowa startu
//! z czytelnym komunikatem**, nigdy ciche wartości domyślne. Literówka
//! w nazwie pola jest błędem, a nie polem pominiętym po cichu.

use diktate_config::{DaemonConfig, GtkConfig};

#[test]
fn pusty_plik_daje_same_wartosci_domyslne() {
    let config = DaemonConfig::from_toml("").expect("pusty plik jest poprawny");
    assert_eq!(config.audio.pre_roll_ms, 300);
    assert_eq!(config.vad.threshold, 0.5);
    assert_eq!(config.whisper.language, "pl");
}

#[test]
fn literowka_w_nazwie_pola_jest_bledem_a_nie_cichym_pominieciem() {
    let err = DaemonConfig::from_toml("[vad]\nthreshhold = 0.5\n")
        .expect_err("literówka musi zatrzymać start");
    let message = format!("{err:#}");
    assert!(
        message.contains("threshhold"),
        "komunikat ma wskazywać winne pole, a jest: {message}"
    );
}

#[test]
fn prog_vad_poza_zakresem_jest_odrzucany() {
    let err = DaemonConfig::from_toml("[vad]\nthreshold = 1.5\n")
        .expect_err("próg spoza 0–1 nie ma sensu");
    assert!(format!("{err:#}").contains("threshold"));
}

#[test]
fn segment_krotszy_od_minimalnego_niz_maksymalny_jest_odrzucany() {
    let err = DaemonConfig::from_toml("[vad]\nmin_speech_ms = 20000\nmax_segment_ms = 15000\n")
        .expect_err("minimum większe od maksimum nie ma sensu");
    assert!(format!("{err:#}").contains("min_speech_ms"));
}

#[test]
fn segment_dluzszy_niz_okno_whispera_jest_odrzucany() {
    // Whisper przetwarza zawsze okno 30 s — dłuższy segment i tak zostałby
    // obcięty, tyle że po cichu.
    let err = DaemonConfig::from_toml("[vad]\nmax_segment_ms = 45000\n")
        .expect_err("segment dłuższy od okna whispera nie ma sensu");
    assert!(format!("{err:#}").contains("max_segment_ms"));
}

#[test]
fn wczytana_wartosc_nadpisuje_domyslna_a_reszta_zostaje() {
    let config = DaemonConfig::from_toml("[vad]\nthreshold = 0.7\n").unwrap();
    assert_eq!(config.vad.threshold, 0.7);
    assert_eq!(config.vad.hangover_ms, 700, "reszta sekcji bez zmian");
}

#[test]
fn konfiguracja_gtk_ma_wlasne_wartosci_domyslne() {
    let config = GtkConfig::from_toml("").expect("pusty plik jest poprawny");
    assert!(config.dictation.auto_space);
    assert!(config.dictation.strip_trailing_ellipsis);
    assert!(config.dictation.insert_at_cursor);
    assert_eq!(config.window.width, 620);
}

#[test]
fn konfiguracja_gtk_tez_nie_wybacza_literowek() {
    let err = GtkConfig::from_toml("[dictation]\nauto_spce = true\n")
        .expect_err("literówka musi zatrzymać start");
    assert!(format!("{err:#}").contains("auto_spce"));
}

#[test]
fn skrot_klawiszowy_o_bledny_skladni_jest_odrzucany() {
    let err = GtkConfig::from_toml("[shortcuts]\nundo = \"\"\n")
        .expect_err("pusty skrót nie da się z niczym związać");
    assert!(format!("{err:#}").contains("undo"));
}

#[test]
fn okno_o_zerowej_szerokosci_jest_odrzucane() {
    let err = GtkConfig::from_toml("[window]\nwidth = 0\n").expect_err("okno musi mieć rozmiar");
    assert!(format!("{err:#}").contains("width"));
}
