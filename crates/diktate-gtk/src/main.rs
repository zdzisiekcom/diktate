//! Okno dyktowania.
//!
//! Etap 0: jedno okno, przycisk przełączający nagrywanie, `TextView` na tekst
//! i przycisk „Kopiuj". Bez modelu dokumentu — tekst z `segment.final` jest
//! dopisywany na końcu bufora.
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

use client::{Client, Incoming};
use diktate_proto::SessionState;
use gtk4::glib::SourceId;
use gtk4::{PolicyType, ScrolledWindow, Spinner, Stack, TextView, ToggleButton, WrapMode};
use libadwaita::prelude::*;
use libadwaita::{
    Application, ApplicationWindow, Clamp, HeaderBar, StatusPage, Toast, ToastOverlay,
    ToolbarView, WindowTitle,
};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

const APP_ID: &str = "dev.diktate.Gtk";

/// Nazwy stron w `Stack`-u treści: pusty bufor pokazuje zachętę do
/// dyktowania, niepusty — widok tekstu.
const PAGE_EMPTY: &str = "empty";
const PAGE_TEXT: &str = "text";

/// Odstęp między mignięciami ikony mikrofonu w trakcie nagrywania.
const MIC_PULSE_INTERVAL: Duration = Duration::from_millis(600);

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

/// Uruchamia pulsowanie ikony mikrofonu (naprzemienna przezroczystość).
/// Jeśli jakiś timer już działa, najpierw go zatrzymuje — nigdy nie mamy
/// dwóch nakładających się animacji na tym samym przycisku.
fn start_mic_pulse(button: &ToggleButton, pulse_source: &Rc<RefCell<Option<SourceId>>>) {
    stop_mic_pulse(button, pulse_source);

    let dim = Rc::new(Cell::new(false));
    let button_for_tick = button.clone();
    let id = gtk4::glib::timeout_add_local(MIC_PULSE_INTERVAL, move || {
        let is_dim = dim.get();
        button_for_tick.set_opacity(if is_dim { 1.0 } else { 0.4 });
        dim.set(!is_dim);
        gtk4::glib::ControlFlow::Continue
    });
    *pulse_source.borrow_mut() = Some(id);
}

/// Zatrzymuje pulsowanie i przywraca pełną widoczność ikony. Musi być
/// wywołana przy każdym przejściu poza stan nagrywania i przy rozłączeniu z
/// daemonem — bez tego timer tykałby w nieskończoność mimo zakończonej
/// sesji.
fn stop_mic_pulse(button: &ToggleButton, pulse_source: &Rc<RefCell<Option<SourceId>>>) {
    if let Some(id) = pulse_source.borrow_mut().take() {
        id.remove();
    }
    button.set_opacity(1.0);
}

fn build_ui(app: &Application) {
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
        .default_width(560)
        .default_height(420)
        .content(&toolbar_view)
        .build();

    // Bufor sam decyduje, którą stronę stosu pokazać — obejmuje to zarówno
    // tekst dopisywany z daemona, jak i ręczne usunięcie treści przez
    // użytkownika (wymaganie: powrót do `StatusPage` po wyczyszczeniu).
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
        // Timer pulsowania mikrofonu — dzielony między pętlę odbierającą
        // stan i (pośrednio) zamknięcie okna, żeby zawsze dało się go
        // jednoznacznie zatrzymać.
        let mic_pulse_source: Rc<RefCell<Option<SourceId>>> = Rc::new(RefCell::new(None));
        let previous_state: Rc<Cell<Option<SessionState>>> = Rc::new(Cell::new(None));

        // Wiadomości z daemona wpadają tu asynchronicznie: tekst trafia na
        // koniec bufora, zmiana stanu aktualizuje podtytuł, pulsowanie
        // ikony i widoczność spinnera zaległości.
        let incoming = client.incoming();
        let buffer_for_incoming = text_view.buffer();
        let window_title_for_incoming = window_title.clone();
        let record_button_for_incoming = record_button.clone();
        let backlog_spinner_for_incoming = backlog_spinner.clone();
        let mic_pulse_source_for_incoming = mic_pulse_source.clone();
        let previous_state_for_incoming = previous_state.clone();
        gtk4::glib::spawn_future_local(async move {
            while let Ok(message) = incoming.recv().await {
                match message {
                    Incoming::Segment(text) => {
                        let mut end = buffer_for_incoming.end_iter();
                        if buffer_for_incoming.char_count() > 0 {
                            buffer_for_incoming.insert(&mut end, " ");
                        }
                        buffer_for_incoming.insert(&mut end, &text);
                    }
                    Incoming::State { state, backlog } => {
                        window_title_for_incoming.set_subtitle(&subtitle_for_state(state, backlog));

                        if backlog > 0 {
                            backlog_spinner_for_incoming.set_visible(true);
                            backlog_spinner_for_incoming.start();
                        } else {
                            backlog_spinner_for_incoming.stop();
                            backlog_spinner_for_incoming.set_visible(false);
                        }

                        // Pulsowanie mikrofonu reaguje tylko na faktyczne
                        // przejście stanu, żeby powtarzające się
                        // powiadomienia „Recording" (np. przy zmianie
                        // samego backlogu) nie restartowały timera bez
                        // potrzeby.
                        if previous_state_for_incoming.get() != Some(state) {
                            match state {
                                SessionState::Recording => start_mic_pulse(
                                    &record_button_for_incoming,
                                    &mic_pulse_source_for_incoming,
                                ),
                                SessionState::Idle => stop_mic_pulse(
                                    &record_button_for_incoming,
                                    &mic_pulse_source_for_incoming,
                                ),
                            }
                            previous_state_for_incoming.set(Some(state));
                        }
                    }
                }
            }

            // Kanał się zamknął — daemon się rozłączył. Zatrzymujemy
            // pulsowanie i spinner, żeby nic nie kręciło się bez powodu przy
            // martwym połączeniu.
            stop_mic_pulse(&record_button_for_incoming, &mic_pulse_source_for_incoming);
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

    let buffer_for_copy = text_view.buffer();
    let toast_overlay_for_copy = toast_overlay.clone();
    copy_button.connect_clicked(move |button| {
        let text = buffer_for_copy.text(
            &buffer_for_copy.start_iter(),
            &buffer_for_copy.end_iter(),
            false,
        );
        button.clipboard().set_text(&text);
        toast_overlay_for_copy.add_toast(Toast::new("Skopiowano"));
    });

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
