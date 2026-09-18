//! Okno dyktowania.
//!
//! Jedno okno: przycisk przełączający nagrywanie, `TextView` na tekst
//! i przycisk „Kopiuj". Tekst nie jest dopisywany na koniec bufora, tylko
//! wstawiany **w kotwicę wstawiania** modelu dokumentu — patrz [`view`]
//! i `docs/07-zachowanie-dyktowania.md`. Dopisanie na koniec jest tylko
//! przypadkiem szczególnym tej jednej ścieżki.
//!
//! Warstwa widoku zbudowana jest na libadwaicie, żeby okno wyglądało i
//! zachowywało się jak natywna aplikacja GNOME: pasek nagłówka zintegrowany
//! z treścią (bez osobnego paska tytułowego), podążanie za systemowym
//! schematem kolorów oraz `StatusPage` widoczna, dopóki bufor jest pusty.
//!
//! **Stan sesji w podtytule.** Podtytuł `WindowTitle` jest jedynym miejscem,
//! gdzie pokazujemy stan — zgodnie z konwencją widoków w `docs/06-gui.md` nie
//! ma osobnej etykiety statusu pod treścią. Źródłem prawdy o stanie jest
//! wyłącznie powiadomienie `state.changed` z daemona (`Incoming::State`) —
//! kliknięcie przycisku nagrywania nie zgaduje tekstu podtytułu, tylko czeka
//! na potwierdzenie. Ikona mikrofonu pulsuje w trakcie nagrywania, a `Spinner`
//! obok tytułu pokazuje się, dopóki jest cokolwiek w kolejce transkrypcji.

mod client;
mod view;

use client::{Client, Incoming};
use diktate_config::GtkConfig;
use diktate_proto::SessionState;
use gtk4::{PolicyType, ScrolledWindow, Spinner, Stack, TextView, ToggleButton, WrapMode};
use libadwaita::prelude::*;
use libadwaita::{
    Application, ApplicationWindow, Clamp, HeaderBar, StatusPage, Toast, ToastOverlay, ToolbarView,
    WindowTitle,
};
use std::rc::Rc;
use view::DocumentView;

const APP_ID: &str = "dev.diktate.Gtk";

/// Nazwy stron w `Stack`-u treści: pusty bufor pokazuje zachętę do
/// dyktowania, niepusty — widok tekstu.
const PAGE_EMPTY: &str = "empty";
const PAGE_TEXT: &str = "text";

fn main() -> gtk4::glib::ExitCode {
    // Aplikacja libadwaity sama inicjalizuje bibliotekę i podłącza się pod
    // `adw::StyleManager` z domyślnym `ColorScheme::Default`, czyli
    // podążaniem za systemowym jasnym/ciemnym motywem — nic tu nie trzeba
    // wymuszać.
    let app = Application::builder().application_id(APP_ID).build();
    app.connect_activate(build_ui);
    app.run()
}

/// Forma liczebnika odpowiadająca polskiej gramatyce dla rzeczowników
/// policzalnych: pojedyncza dla 1, „kilka" dla końcówek 2–4 (poza 12–14),
/// „wiele" dla reszty (dopełniacz liczby mnogiej).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PluralForm {
    One,
    Few,
    Many,
}

fn polish_plural_form(n: u32) -> PluralForm {
    if n == 1 {
        return PluralForm::One;
    }
    let last_digit = n % 10;
    let last_two_digits = n % 100;
    if (2..=4).contains(&last_digit) && !(12..=14).contains(&last_two_digits) {
        PluralForm::Few
    } else {
        PluralForm::Many
    }
}

/// Odmiana słowa „wypowiedź" dla danej liczby. Formy „kilka" i „wiele"
/// pokrywają się dla tego rzeczownika („2 wypowiedzi", „5 wypowiedzi",
/// „22 wypowiedzi") — jedyną formą różną jest liczba pojedyncza.
fn wypowiedzi_word(n: u32) -> &'static str {
    match polish_plural_form(n) {
        PluralForm::One => "wypowiedź",
        PluralForm::Few | PluralForm::Many => "wypowiedzi",
    }
}

fn backlog_phrase(n: u32) -> String {
    format!("{n} {}", wypowiedzi_word(n))
}

/// Tekst podtytułu na podstawie stanu sesji i liczby zaległości. Wywoływana
/// wyłącznie z obsługi `Incoming::State` — podtytuł nigdy nie jest zgadywany
/// na podstawie samego kliknięcia przycisku.
fn subtitle_for_state(state: SessionState, backlog: u32) -> String {
    match (state, backlog) {
        (SessionState::Idle, 0) => "Gotowe".to_string(),
        (SessionState::Idle, n) => format!("Przetwarzam {}…", backlog_phrase(n)),
        (SessionState::Recording, 0) => "Nagrywam".to_string(),
        (SessionState::Recording, n) => format!("Nagrywam · przetwarzam {}", backlog_phrase(n)),
    }
}

fn build_ui(app: &Application) {
    // Błąd w gtk.toml zatrzymuje start z czytelnym komunikatem, a nie cofa
    // się po cichu do wartości domyślnych (docs/04).
    let config = match GtkConfig::load() {
        Ok(config) => config,
        Err(err) => {
            eprintln!("diktate-gtk: {err:#}");
            std::process::exit(1);
        }
    };

    let text_view = TextView::builder()
        .wrap_mode(WrapMode::WordChar)
        .left_margin(12)
        .right_margin(12)
        .top_margin(12)
        .bottom_margin(12)
        .build();

    let scroller = ScrolledWindow::builder()
        .hscrollbar_policy(PolicyType::Never)
        .vexpand(true)
        .child(&text_view)
        .build();

    // `Clamp` ogranicza szerokość tekstu w dużym oknie, tak jak robią to
    // aplikacje GNOME (np. Ustawienia, Teksty).
    let clamp = Clamp::builder().child(&scroller).maximum_size(720).build();

    let status_page = StatusPage::builder()
        .icon_name("audio-input-microphone-symbolic")
        .title("Gotowy do dyktowania")
        .description("Naciśnij przycisk nagrywania, aby rozpocząć dyktowanie.")
        .vexpand(true)
        .build();

    let stack = Stack::new();
    stack.add_named(&status_page, Some(PAGE_EMPTY));
    stack.add_named(&clamp, Some(PAGE_TEXT));
    stack.set_visible_child_name(PAGE_EMPTY);

    let toast_overlay = ToastOverlay::new();
    toast_overlay.set_child(Some(&stack));

    let window_title = WindowTitle::new("Diktate", "Łączę z daemonem…");

    // Spinner obok tytułu — widoczny wyłącznie, gdy w kolejce transkrypcji
    // jest cokolwiek do przetworzenia (backlog > 0).
    let backlog_spinner = Spinner::builder().visible(false).build();

    let title_box = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Horizontal)
        .spacing(6)
        .halign(gtk4::Align::Center)
        .build();
    title_box.append(&window_title);
    title_box.append(&backlog_spinner);

    let record_button = ToggleButton::builder()
        .icon_name("audio-input-microphone-symbolic")
        .tooltip_text("Nagrywaj")
        .build();
    let copy_button = gtk4::Button::builder()
        .icon_name("edit-copy-symbolic")
        .tooltip_text("Kopiuj do schowka")
        .build();

    let header = HeaderBar::new();
    header.set_title_widget(Some(&title_box));
    header.pack_start(&record_button);
    header.pack_end(&copy_button);

    let toolbar_view = ToolbarView::new();
    toolbar_view.add_top_bar(&header);
    toolbar_view.set_content(Some(&toast_overlay));

    let window = ApplicationWindow::builder()
        .application(app)
        .title("Diktate")
        .default_width(config.window.width)
        .default_height(config.window.height)
        .content(&toolbar_view)
        .build();

    // Bufor sam decyduje, którą stronę stosu pokazać — obejmuje to zarówno
    // tekst dopisywany z daemona, jak i ręczne usunięcie treści przez
    // użytkownika (wymaganie: powrót do `StatusPage` po wyczyszczeniu).
    // Model dokumentu podpięty pod bufor. Od tej chwili bufor jest widokiem,
    // a nie źródłem prawdy.
    let document = DocumentView::attach(&text_view, config.dictation.clone());

    let buffer = text_view.buffer();
    let stack_for_buffer = stack.clone();
    buffer.connect_changed(move |buffer| {
        let page = if buffer.char_count() == 0 {
            PAGE_EMPTY
        } else {
            PAGE_TEXT
        };
        stack_for_buffer.set_visible_child_name(page);
    });

    let client = match Client::connect() {
        Ok(client) => {
            window_title.set_subtitle("Gotowe");
            Some(Rc::new(client))
        }
        Err(err) => {
            record_button.set_sensitive(false);
            window_title.set_subtitle(&format!("Brak daemona: {err}"));
            None
        }
    };

    if let Some(client) = client {
        // Wiadomości z daemona wpadają tu asynchronicznie: tekst trafia na
        // koniec bufora, a zmiana stanu aktualizuje podtytuł i widoczność
        // spinnera zaległości.
        let incoming = client.incoming();
        let document_for_incoming = Rc::clone(&document);
        let window_title_for_incoming = window_title.clone();
        let backlog_spinner_for_incoming = backlog_spinner.clone();
        gtk4::glib::spawn_future_local(async move {
            while let Ok(message) = incoming.recv().await {
                match message {
                    Incoming::Segment(segment) => {
                        document_for_incoming.insert_segment(&segment);
                    }
                    // Wskaźnik poziomu jeszcze nie istnieje — odczyty
                    // pochłaniamy, żeby kanał nie rósł.
                    Incoming::Level { .. } => {}
                    Incoming::State { state, backlog } => {
                        window_title_for_incoming.set_subtitle(&subtitle_for_state(state, backlog));

                        if backlog > 0 {
                            backlog_spinner_for_incoming.set_visible(true);
                            backlog_spinner_for_incoming.start();
                        } else {
                            backlog_spinner_for_incoming.stop();
                            backlog_spinner_for_incoming.set_visible(false);
                        }
                    }
                }
            }

            // Kanał się zamknął — daemon się rozłączył. Zatrzymujemy
            // spinner, żeby nic nie kręciło się bez powodu przy martwym
            // połączeniu.
            backlog_spinner_for_incoming.stop();
            backlog_spinner_for_incoming.set_visible(false);
            window_title_for_incoming.set_subtitle("Daemon się rozłączył");
        });

        record_button.connect_toggled(move |button| {
            // Klasa CSS pokazuje stan nagrywania na pierwszy rzut oka —
            // tak jak w systemowych aplikacjach GNOME (np. Nagrywanie
            // dźwięku). Sam tekst podtytułu nie jest tu zgadywany — czeka
            // na potwierdzenie przez `state.changed`.
            if button.is_active() {
                button.add_css_class("destructive-action");
            } else {
                button.remove_css_class("destructive-action");
            }

            let result = if button.is_active() {
                client.start()
            } else {
                client.stop()
            };
            if let Err(err) = result {
                window_title.set_subtitle(&format!("Błąd: {err}"));
                button.set_active(false);
            }
        });
    }

    let document_for_copy = Rc::clone(&document);
    let toast_overlay_for_copy = toast_overlay.clone();
    copy_button.connect_clicked(move |button| {
        button.clipboard().set_text(&document_for_copy.text());
        toast_overlay_for_copy.add_toast(Toast::new("Skopiowano"));
    });

    // Cofanie i ponawianie obejmuje dyktowanie i pisanie razem — obie drogi
    // zapisują się w tym samym logu operacji.
    let shortcuts = gtk4::EventControllerKey::new();
    let document_for_keys = Rc::clone(&document);
    shortcuts.connect_key_pressed(move |_, key, _, state| {
        let ctrl = state.contains(gtk4::gdk::ModifierType::CONTROL_MASK);
        let shift = state.contains(gtk4::gdk::ModifierType::SHIFT_MASK);
        match key {
            gtk4::gdk::Key::z if ctrl && !shift => {
                document_for_keys.undo();
                gtk4::glib::Propagation::Stop
            }
            gtk4::gdk::Key::Z | gtk4::gdk::Key::z if ctrl && shift => {
                document_for_keys.redo();
                gtk4::glib::Propagation::Stop
            }
            _ => gtk4::glib::Propagation::Proceed,
        }
    });
    window.add_controller(shortcuts);

    window.present();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn liczba_pojedyncza_dla_jednej_wypowiedzi() {
        assert_eq!(polish_plural_form(1), PluralForm::One);
        assert_eq!(wypowiedzi_word(1), "wypowiedź");
        assert_eq!(backlog_phrase(1), "1 wypowiedź");
    }

    #[test]
    fn forma_kilka_dla_koncowek_2_do_4() {
        for n in [2, 3, 4, 22, 23, 24, 32, 33, 34] {
            assert_eq!(polish_plural_form(n), PluralForm::Few, "n = {n}");
        }
    }

    #[test]
    fn forma_wiele_dla_piatki_i_wiecej_oraz_nastolatkow() {
        for n in [5, 6, 9, 10, 11, 12, 13, 14, 15, 20, 21, 25] {
            assert_eq!(polish_plural_form(n), PluralForm::Many, "n = {n}");
        }
    }

    #[test]
    fn odmiana_slowa_wypowiedz_pokrywa_sie_dla_kilku_i_wielu() {
        // Dla rzeczownika „wypowiedź" forma „kilka" i „wiele" brzmią tak samo
        // („wypowiedzi"), więc wynikowy tekst jest identyczny niezależnie od
        // tego, do której grupy trafia liczba — sprawdzamy oba przypadki z
        // treści zadania wprost.
        assert_eq!(wypowiedzi_word(2), "wypowiedzi");
        assert_eq!(wypowiedzi_word(5), "wypowiedzi");
        assert_eq!(wypowiedzi_word(12), "wypowiedzi");
        assert_eq!(wypowiedzi_word(22), "wypowiedzi");
        assert_eq!(wypowiedzi_word(23), "wypowiedzi");
        assert_eq!(wypowiedzi_word(25), "wypowiedzi");

        assert_eq!(backlog_phrase(2), "2 wypowiedzi");
        assert_eq!(backlog_phrase(5), "5 wypowiedzi");
        assert_eq!(backlog_phrase(12), "12 wypowiedzi");
        assert_eq!(backlog_phrase(22), "22 wypowiedzi");
        assert_eq!(backlog_phrase(23), "23 wypowiedzi");
        assert_eq!(backlog_phrase(25), "25 wypowiedzi");
    }

    #[test]
    fn podtytul_gotowe_gdy_bezczynnie_i_pusto() {
        assert_eq!(subtitle_for_state(SessionState::Idle, 0), "Gotowe");
    }

    #[test]
    fn podtytul_nagrywam_gdy_pusta_kolejka() {
        assert_eq!(subtitle_for_state(SessionState::Recording, 0), "Nagrywam");
    }

    #[test]
    fn podtytul_nagrywam_z_kolejka() {
        assert_eq!(
            subtitle_for_state(SessionState::Recording, 1),
            "Nagrywam · przetwarzam 1 wypowiedź"
        );
        assert_eq!(
            subtitle_for_state(SessionState::Recording, 2),
            "Nagrywam · przetwarzam 2 wypowiedzi"
        );
        assert_eq!(
            subtitle_for_state(SessionState::Recording, 5),
            "Nagrywam · przetwarzam 5 wypowiedzi"
        );
    }

    #[test]
    fn podtytul_przetwarzam_po_zatrzymaniu_sesji() {
        assert_eq!(
            subtitle_for_state(SessionState::Idle, 3),
            "Przetwarzam 3 wypowiedzi…"
        );
    }
}
