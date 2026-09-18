//! Okno dyktowania.
//!
//! Etap 0: jedno okno, przycisk przełączający nagrywanie, `TextView` na tekst
//! i przycisk „Kopiuj". Bez modelu dokumentu — tekst z `segment.final` jest
//! dopisywany na końcu bufora.

mod client;

use client::Client;
use gtk4::prelude::*;
use gtk4::{
    Align, Application, ApplicationWindow, Box as GtkBox, Button, HeaderBar, Label, Orientation,
    PolicyType, ScrolledWindow, TextView, ToggleButton, WrapMode,
};
use std::rc::Rc;

const APP_ID: &str = "dev.diktate.Gtk";

fn main() -> gtk4::glib::ExitCode {
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

    let status = Label::builder()
        .label("Łączę z daemonem…")
        .halign(Align::Start)
        .margin_start(12)
        .margin_bottom(6)
        .build();

    let record_button = ToggleButton::with_label("Nagrywaj");
    let copy_button = Button::with_label("Kopiuj");

    let header = HeaderBar::new();
    header.pack_start(&record_button);
    header.pack_end(&copy_button);

    let content = GtkBox::new(Orientation::Vertical, 0);
    content.append(&scroller);
    content.append(&status);

    let window = ApplicationWindow::builder()
        .application(app)
        .title("Diktate")
        .default_width(560)
        .default_height(420)
        .child(&content)
        .build();
    window.set_titlebar(Some(&header));

    let client = match Client::connect() {
        Ok(client) => {
            status.set_text("Gotowe.");
            Some(Rc::new(client))
        }
        Err(err) => {
            record_button.set_sensitive(false);
            status.set_text(&format!("Brak daemona: {err}"));
            None
        }
    };

    if let Some(client) = client {
        // Tekst z daemona wpada tu asynchronicznie i ląduje na końcu bufora.
        let segments = client.segments();
        let buffer = text_view.buffer();
        let status_for_segments = status.clone();
        gtk4::glib::spawn_future_local(async move {
            while let Ok(text) = segments.recv().await {
                let mut end = buffer.end_iter();
                if buffer.char_count() > 0 {
                    buffer.insert(&mut end, " ");
                }
                buffer.insert(&mut end, &text);
                status_for_segments.set_text("Gotowe.");
            }
            status_for_segments.set_text("Daemon się rozłączył.");
        });

        let status_for_button = status.clone();
        record_button.connect_toggled(move |button| {
            let result = if button.is_active() {
                client.start()
            } else {
                client.stop()
            };
            match result {
                Ok(()) if button.is_active() => status_for_button.set_text("Nagrywam…"),
                Ok(()) => status_for_button.set_text("Transkrybuję…"),
                Err(err) => {
                    status_for_button.set_text(&format!("Błąd: {err}"));
                    button.set_active(false);
                }
            }
        });
    }

    let buffer_for_copy = text_view.buffer();
    copy_button.connect_clicked(move |button| {
        let text = buffer_for_copy.text(
            &buffer_for_copy.start_iter(),
            &buffer_for_copy.end_iter(),
            false,
        );
        button.clipboard().set_text(&text);
    });

    window.present();
}
