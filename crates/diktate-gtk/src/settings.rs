//! Okno ustawień.
//!
//! Pokrywa to, czego nie da się sensownie zrobić edytorem tekstu: szablon
//! promptu whispera, z którego użytkownik musi **widzieć**, gdzie wchodzi
//! kontekst, oraz ustawienia korekty LLM-em. Reszta konfiguracji zostaje
//! w plikach TOML (`docs/06#okno-ustawień`).
//!
//! Widok jest `adw::PreferencesDialog` — jedyny wyjątek od pary
//! `ApplicationWindow`/`ToolbarView` z konwencji widoków: ten typ niesie
//! tamten układ w sobie i jest wzorcem GNOME dla ustawień.
//!
//! Moduł dzieli się na dwie warstwy: mapowanie pól ↔ konfiguracji
//! ([`Fields`], [`fields_from`], [`apply`]) to czyste funkcje testowane bez
//! GTK-a, a budowanie widżetów ([`present`] i pomocnicze funkcje niżej)
//! dotyka wyłącznie ich wyników.
//!
//! Nic jeszcze nie woła [`present`] — przycisk w pasku nagłówka i skrót
//! `Ctrl+,` dochodzą w kolejnym zadaniu. Stąd atrybut niżej zamiast ośmiu
//! kopii `#[allow(dead_code)]` przy każdej funkcji modułu; znika, gdy
//! `main.rs` zacznie wywoływać `settings::present`.
#![allow(dead_code)]

use diktate_config::{DaemonConfig, GtkConfig};
use gtk4::prelude::*;
use gtk4::{gio, Frame, TextView, WrapMode};
use libadwaita::prelude::*;
use libadwaita::{
    ActionRow, ApplicationWindow, EntryRow, PreferencesDialog, PreferencesGroup, PreferencesPage,
    SpinRow, SwitchRow,
};

/// Pola pokazywane w oknie ustawień — jedyne, które `apply` wolno mu dotknąć.
///
/// Reszta `CorrectionConfig` (`api_key`, `temperature`, `max_tokens`,
/// `timeout_ms`, szablony wiadomości użytkownika) to strojenie, nie
/// ustawienia — okno ma go nie ruszać, żeby jego otwarcie nigdy nie
/// skasowało klucza API.
#[derive(Debug, Clone, PartialEq)]
pub struct Fields {
    pub prompt: String,
    pub context_chars: usize,
    pub correction_enabled: bool,
    pub endpoint: String,
    pub model: String,
    pub system_prompt: String,
    pub redaction_system_prompt: String,
    pub max_change_ratio_short: f32,
    pub max_change_ratio_long: f32,
    pub min_changed_words: usize,
}

/// Czyta pola okna z aktualnej konfiguracji.
pub fn fields_from(config: &GtkConfig) -> Fields {
    Fields {
        prompt: config.whisper.prompt.clone(),
        context_chars: config.whisper.context_chars,
        correction_enabled: config.correction.enabled,
        endpoint: config.correction.endpoint.clone(),
        model: config.correction.model.clone(),
        system_prompt: config.correction.system_prompt.clone(),
        redaction_system_prompt: config.correction.redaction_system_prompt.clone(),
        max_change_ratio_short: config.correction.max_change_ratio_short,
        max_change_ratio_long: config.correction.max_change_ratio_long,
        min_changed_words: config.correction.min_changed_words,
    }
}

/// Zapisuje pola okna z powrotem do konfiguracji. Dotyka **wyłącznie** pól
/// z [`Fields`] — nic ponadto.
pub fn apply(config: &mut GtkConfig, fields: &Fields) {
    config.whisper.prompt = fields.prompt.clone();
    config.whisper.context_chars = fields.context_chars;
    config.correction.enabled = fields.correction_enabled;
    config.correction.endpoint = fields.endpoint.clone();
    config.correction.model = fields.model.clone();
    config.correction.system_prompt = fields.system_prompt.clone();
    config.correction.redaction_system_prompt = fields.redaction_system_prompt.clone();
    config.correction.max_change_ratio_short = fields.max_change_ratio_short;
    config.correction.max_change_ratio_long = fields.max_change_ratio_long;
    config.correction.min_changed_words = fields.min_changed_words;
}

// --- budowanie widżetów ---

/// Wielolinijkowe pole tekstowe w karcie — `EntryRow` jest jednolinijkowy,
/// a szablon promptu i instrukcje korekty muszą być czytelne na kilku
/// wierszach naraz.
fn text_card(initial: &str) -> (Frame, TextView) {
    let text_view = TextView::builder()
        .wrap_mode(WrapMode::WordChar)
        .top_margin(8)
        .bottom_margin(8)
        .left_margin(8)
        .right_margin(8)
        .build();
    text_view.buffer().set_text(initial);
    // Minimum 8 wierszy — poniżej tego prompt korektora nie mieści się bez
    // przewijania nawet w spokojnym czytaniu.
    text_view.set_size_request(-1, 160);

    let frame = Frame::new(None);
    frame.add_css_class("card");
    frame.set_child(Some(&text_view));
    (frame, text_view)
}

/// To samo co [`text_card`], z etykietą nad kartą — dla grup, w których
/// więcej niż jedno pole tekstowe potrzebuje własnego opisu.
fn labeled_text_card(label: &str, initial: &str) -> (gtk4::Box, TextView) {
    let heading = gtk4::Label::builder()
        .label(label)
        .halign(gtk4::Align::Start)
        .build();
    heading.add_css_class("heading");

    let (frame, text_view) = text_card(initial);

    let container = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Vertical)
        .spacing(6)
        .build();
    container.append(&heading);
    container.append(&frame);
    (container, text_view)
}

/// Odczytuje bieżącą treść pola wielolinijkowego.
fn text_view_value(view: &TextView) -> String {
    let buffer = view.buffer();
    buffer
        .text(&buffer.start_iter(), &buffer.end_iter(), false)
        .to_string()
}

/// Wiersz „Otwórz …” — ikona plus otwarcie pliku domyślnym programem.
/// Nieudane otwarcie (np. brak zarejestrowanego programu dla `.toml`) trafia
/// do logu — to nie jest akcja, dla której otwarte okno ustawień ma się
/// czymkolwiek tłumaczyć.
fn open_file_row(title: &str, path: std::path::PathBuf) -> ActionRow {
    let row = ActionRow::builder()
        .title(title)
        .activatable(true)
        .build();
    row.add_prefix(&gtk4::Image::from_icon_name("document-open-symbolic"));
    let uri = format!("file://{}", path.display());
    row.connect_activated(move |_| {
        if let Err(err) = gio::AppInfo::launch_default_for_uri(&uri, gio::AppLaunchContext::NONE) {
            eprintln!("diktate-gtk: nie udało się otworzyć {uri}: {err}");
        }
    });
    row
}

/// Otwiera okno ustawień. Zapis następuje przy zamknięciu (`connect_closed`):
/// pola widżetów schodzą do [`Fields`], [`apply`] nakłada je na kopię
/// przekazanej konfiguracji, a `config.save()` zapisuje ją na dysk. Dopiero
/// udany zapis woła `on_saved` — nieudany zostaje w logu, a okno wywołujące
/// nie dostaje nowej konfiguracji, więc nie może udawać, że zapis się udał.
pub fn present(
    parent: &ApplicationWindow,
    config: &GtkConfig,
    on_saved: impl Fn(GtkConfig) + 'static,
) {
    let fields = fields_from(config);
    let base_config = config.clone();

    let dialog = PreferencesDialog::new();
    dialog.set_title("Ustawienia");

    // --- strona „Transkrypcja" ---

    let transcription_page = PreferencesPage::builder()
        .title("Transkrypcja")
        .icon_name("audio-input-microphone-symbolic")
        .build();

    let prompt_group = PreferencesGroup::builder()
        .title("Prompt whispera")
        .description(
            "Szablon podpowiedzi dla modelu. {kontekst} to miejsce, w które wchodzi \
             tekst sprzed kursora — bez tego symbolu prompt jest stały.",
        )
        .build();
    let (prompt_frame, prompt_view) = text_card(&fields.prompt);
    prompt_group.add(&prompt_frame);
    transcription_page.add(&prompt_group);

    let context_group = PreferencesGroup::builder().title("Kontekst").build();
    let context_row = SpinRow::with_range(0.0, 1000.0, 50.0);
    context_row.set_title("Znaków przed kursorem");
    context_row.set_value(fields.context_chars as f64);
    context_group.add(&context_row);
    transcription_page.add(&context_group);

    let files_group = PreferencesGroup::builder().title("Pliki").build();
    files_group.add(&open_file_row("Otwórz gtk.toml", GtkConfig::path()));
    files_group.add(&open_file_row("Otwórz daemon.toml", DaemonConfig::path()));
    transcription_page.add(&files_group);

    dialog.add(&transcription_page);

    // --- strona „Korekta" ---

    let correction_page = PreferencesPage::builder()
        .title("Korekta")
        .icon_name("text-editor-symbolic")
        .build();

    let server_group = PreferencesGroup::builder().title("Serwer").build();
    let enabled_row = SwitchRow::builder()
        .title("Korekta włączona")
        .active(fields.correction_enabled)
        .build();
    server_group.add(&enabled_row);
    let endpoint_row = EntryRow::builder().title("Adres").build();
    endpoint_row.set_text(&fields.endpoint);
    server_group.add(&endpoint_row);
    let model_row = EntryRow::builder().title("Model").build();
    model_row.set_text(&fields.model);
    server_group.add(&model_row);
    correction_page.add(&server_group);

    let instructions_group = PreferencesGroup::builder().title("Instrukcje").build();
    let (system_box, system_view) = labeled_text_card("Prompt korektora", &fields.system_prompt);
    instructions_group.add(&system_box);
    let (redaction_box, redaction_view) =
        labeled_text_card("Prompt redaktora", &fields.redaction_system_prompt);
    instructions_group.add(&redaction_box);
    correction_page.add(&instructions_group);

    let thresholds_group = PreferencesGroup::builder().title("Progi").build();
    let short_ratio_row = SpinRow::with_range(0.0, 1.0, 0.05);
    short_ratio_row.set_title("Próg zmian — krótka notatka");
    short_ratio_row.set_digits(2);
    short_ratio_row.set_value(fields.max_change_ratio_short as f64);
    thresholds_group.add(&short_ratio_row);

    let long_ratio_row = SpinRow::with_range(0.0, 1.0, 0.05);
    long_ratio_row.set_title("Próg zmian — długa notatka");
    long_ratio_row.set_digits(2);
    long_ratio_row.set_value(fields.max_change_ratio_long as f64);
    thresholds_group.add(&long_ratio_row);

    let min_changed_row = SpinRow::with_range(0.0, 20.0, 1.0);
    min_changed_row.set_title("Zmienionych słów bez pytania");
    min_changed_row.set_value(fields.min_changed_words as f64);
    thresholds_group.add(&min_changed_row);
    correction_page.add(&thresholds_group);

    dialog.add(&correction_page);

    dialog.connect_closed(move |_| {
        let fields = Fields {
            prompt: text_view_value(&prompt_view),
            context_chars: context_row.value() as usize,
            correction_enabled: enabled_row.is_active(),
            endpoint: endpoint_row.text().to_string(),
            model: model_row.text().to_string(),
            system_prompt: text_view_value(&system_view),
            redaction_system_prompt: text_view_value(&redaction_view),
            max_change_ratio_short: short_ratio_row.value() as f32,
            max_change_ratio_long: long_ratio_row.value() as f32,
            min_changed_words: min_changed_row.value() as usize,
        };
        let mut new_config = base_config.clone();
        apply(&mut new_config, &fields);
        match new_config.save() {
            Ok(()) => on_saved(new_config),
            Err(err) => eprintln!("diktate-gtk: nie udało się zapisać ustawień: {err:#}"),
        }
    });

    dialog.present(Some(parent));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pola_biora_wartosci_z_konfiguracji() {
        let mut config = GtkConfig::default();
        config.whisper.prompt = "Terminy: ggml.\n{kontekst}".to_string();
        config.whisper.context_chars = 300;
        config.correction.model = "bielik".to_string();
        let fields = fields_from(&config);
        assert_eq!(fields.prompt, "Terminy: ggml.\n{kontekst}");
        assert_eq!(fields.context_chars, 300);
        assert_eq!(fields.model, "bielik");
    }

    #[test]
    fn zapis_pol_wraca_do_konfiguracji() {
        let mut config = GtkConfig::default();
        let fields = Fields {
            prompt: "nowy szablon {kontekst}".to_string(),
            context_chars: 120,
            correction_enabled: false,
            endpoint: "http://localhost:8080/v1/chat/completions".to_string(),
            model: "bielik-11b-v3".to_string(),
            system_prompt: "Jesteś korektorem.".to_string(),
            redaction_system_prompt: "Jesteś redaktorem.".to_string(),
            max_change_ratio_short: 0.4,
            max_change_ratio_long: 0.6,
            min_changed_words: 3,
        };
        apply(&mut config, &fields);
        assert_eq!(config.whisper.prompt, "nowy szablon {kontekst}");
        assert_eq!(config.whisper.context_chars, 120);
        assert!(!config.correction.enabled);
        assert_eq!(config.correction.model, "bielik-11b-v3");
        assert_eq!(config.correction.min_changed_words, 3);
    }

    #[test]
    fn mapowanie_w_obie_strony_nic_nie_gubi() {
        let mut config = GtkConfig::default();
        config.whisper.prompt = "szablon".to_string();
        let mut kopia = GtkConfig::default();
        apply(&mut kopia, &fields_from(&config));
        assert_eq!(kopia.whisper.prompt, config.whisper.prompt);
        assert_eq!(kopia.correction.system_prompt, config.correction.system_prompt);
    }

    // Pola, których okno nie pokazuje, mają przetrwać zapis — inaczej
    // ustawienia zjadłyby klucz API i limity czasu przy pierwszym otwarciu.
    #[test]
    fn pola_spoza_okna_zostaja_nietkniete() {
        let mut config = GtkConfig::default();
        config.correction.api_key = "tajne".to_string();
        config.correction.timeout_ms = 12_345;
        let fields = fields_from(&config);
        apply(&mut config, &fields);
        assert_eq!(config.correction.api_key, "tajne");
        assert_eq!(config.correction.timeout_ms, 12_345);
    }
}
