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
    assert_eq!(
        config.vad.silence_ms_to_end, 1_500,
        "reszta sekcji bez zmian"
    );
}

#[test]
fn krotszy_prog_ciszy_dluzszy_od_zwyklego_jest_odrzucany() {
    // Adaptacyjny próg ma rytm skracać, nie wydłużać (docs/07). Odwrotne
    // ustawienie znaczy, że długa wypowiedź czekałaby na tekst dłużej niż
    // krótka — to zawsze pomyłka w pliku, nie zamiar.
    let err =
        DaemonConfig::from_toml("[vad]\nsilence_ms_to_end = 300\nsilence_ms_to_end_long = 1500\n")
            .expect_err("krótszy próg dłuższy od zwykłego nie ma sensu");
    assert!(format!("{err:#}").contains("silence_ms_to_end_long"));
}

#[test]
fn prog_dlugiej_wypowiedzi_powyzej_twardego_limitu_jest_odrzucany() {
    // Gdyby long_segment_ms przekraczał max_segment_ms, krótki próg nigdy
    // by się nie włączył — adaptacja byłaby martwym kodem.
    let err = DaemonConfig::from_toml("[vad]\nlong_segment_ms = 20000\nmax_segment_ms = 15000\n")
        .expect_err("próg długości powyżej twardego limitu nigdy by nie zadziałał");
    assert!(format!("{err:#}").contains("long_segment_ms"));
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

// --- korekta LLM (docs/11) ---

#[test]
fn korekta_ma_domyslny_prompt_z_przykladami_napraw() {
    // Prompt bez przykładów daje model bierny — to główny wniosek pomiaru
    // z `docs/11`, więc domyślna wartość nie może go zgubić.
    let config = GtkConfig::from_toml("").expect("pusty plik jest poprawny");
    assert!(config.correction.enabled);
    assert!(config.correction.system_prompt.contains("ten wis per"));
    assert!(config.correction.user_template.contains("{text}"));
}

#[test]
fn prompt_uzytkownika_bez_miejsca_na_tekst_jest_bledem() {
    let err = GtkConfig::from_toml("[correction]\nuser_template = \"Popraw to zdanie\"\n")
        .expect_err("szablon bez {text} nigdy nie przekaże notatki");
    assert!(format!("{err:#}").contains("user_template"));
}

#[test]
fn prog_zmian_spoza_zakresu_jest_odrzucany() {
    let err = GtkConfig::from_toml("[correction]\nmax_change_ratio_long = 1.4\n")
        .expect_err("odsetek większy od 1.0 nie ma sensu");
    assert!(format!("{err:#}").contains("max_change_ratio_long"));
}

#[test]
fn pusty_prompt_przy_wlaczonej_korekcie_zatrzymuje_start() {
    let err = GtkConfig::from_toml("[correction]\nsystem_prompt = \"\"\n")
        .expect_err("bez instrukcji model redaguje zamiast poprawiać");
    assert!(format!("{err:#}").contains("system_prompt"));
}

#[test]
fn wylaczona_korekta_nie_wymaga_adresu_ale_liczby_sprawdza_dalej() {
    let config = GtkConfig::from_toml("[correction]\nenabled = false\nendpoint = \"\"\n")
        .expect("wyłączonej korekty nie ma po co konfigurować");
    assert!(!config.correction.enabled);

    let err = GtkConfig::from_toml("[correction]\nenabled = false\nmax_tokens = 0\n")
        .expect_err("literówka w liczbie ma się odezwać od razu");
    assert!(format!("{err:#}").contains("max_tokens"));
}

#[test]
fn prog_zmian_zalezy_od_dlugosci_notatki() {
    // Jedna liczba nie wystarcza: na zdaniu bezpieczne jest ~10%, na
    // akapicie model rutynowo przekracza 30% (`docs/11`).
    let config = GtkConfig::from_toml("").unwrap().correction;
    assert_eq!(config.max_change_ratio(5), config.max_change_ratio_short);
    assert_eq!(config.max_change_ratio(500), config.max_change_ratio_long);
    assert_eq!(
        config.max_change_ratio(config.short_text_words),
        config.max_change_ratio_short,
        "granica należy do notatki krótkiej"
    );
}

// --- redakcja poleceniem głosowym (docs/11) ---

#[test]
fn redakcja_ma_domyslny_prompt_z_miejscem_na_polecenie_i_tekst() {
    // Redakcja odwraca umowę korekty: to polecenie użytkownika jest
    // instrukcją, więc szablon musi nieść oba pola naraz.
    let config = GtkConfig::from_toml("").expect("pusty plik jest poprawny");
    assert!(config
        .correction
        .redaction_user_template
        .contains("{instruction}"));
    assert!(config.correction.redaction_user_template.contains("{text}"));
    assert!(!config.correction.redaction_system_prompt.trim().is_empty());
}

#[test]
fn szablon_redakcji_bez_polecenia_jest_bledem() {
    let err =
        GtkConfig::from_toml("[correction]\nredaction_user_template = \"Zredaguj:\\n\\n{text}\"\n")
            .expect_err("szablon bez {instruction} gubi polecenie użytkownika");
    assert!(format!("{err:#}").contains("redaction_user_template"));
}

#[test]
fn szablon_redakcji_bez_tekstu_jest_bledem() {
    let err = GtkConfig::from_toml(
        "[correction]\nredaction_user_template = \"Zrób tak: {instruction}\"\n",
    )
    .expect_err("szablon bez {text} nie przekaże modelowi, co redagować");
    assert!(format!("{err:#}").contains("redaction_user_template"));
}
