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

mod client;

use client::Client;
use gtk4::prelude::*;
use gtk4::{PolicyType, ScrolledWindow, Stack, TextView, ToggleButton, WrapMode};
use libadwaita::{
    Application, ApplicationWindow, Clamp, HeaderBar, StatusPage, Toast, ToastOverlay,
    ToolbarView, WindowTitle,
};
use std::rc::Rc;

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

    let record_button = ToggleButton::builder()
        .icon_name("audio-input-microphone-symbolic")
        .tooltip_text("Nagrywaj")
        .build();
    let copy_button = gtk4::Button::builder()
        .icon_name("edit-copy-symbolic")
        .tooltip_text("Kopiuj do schowka")
        .build();

    let header = HeaderBar::new();
    header.set_title_widget(Some(&window_title));
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
        // Tekst z daemona wpada tu asynchronicznie i ląduje na końcu bufora.
        let segments = client.segments();
        let buffer_for_segments = text_view.buffer();
        let window_title_for_segments = window_title.clone();
        gtk4::glib::spawn_future_local(async move {
            while let Ok(text) = segments.recv().await {
                let mut end = buffer_for_segments.end_iter();
                if buffer_for_segments.char_count() > 0 {
                    buffer_for_segments.insert(&mut end, " ");
                }
                buffer_for_segments.insert(&mut end, &text);
                window_title_for_segments.set_subtitle("Gotowe");
            }
            window_title_for_segments.set_subtitle("Daemon się rozłączył");
        });

        let window_title_for_button = window_title.clone();
        record_button.connect_toggled(move |button| {
            // Klasa CSS pokazuje stan nagrywania na pierwszy rzut oka —
            // tak jak w systemowych aplikacjach GNOME (np. Nagrywanie
            // dźwięku).
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
            match result {
                Ok(()) if button.is_active() => window_title_for_button.set_subtitle("Nagrywam…"),
                Ok(()) => window_title_for_button.set_subtitle("Transkrybuję…"),
                Err(err) => {
                    window_title_for_button.set_subtitle(&format!("Błąd: {err}"));
                    button.set_active(false);
                }
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
