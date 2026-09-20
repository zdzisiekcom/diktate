//! Kontekst dla whispera: co klient wysyła daemonowi i jak często.
//!
//! Prompt powstaje z szablonu użytkownika i ogona tekstu sprzed kursora
//! (`docs/07#kontekst-dla-whispera`). Wysyłka jest odbita w czasie i
//! pomijana, gdy prompt się nie zmienił — pisanie z klawiatury zmienia
//! bufor przy każdym znaku, a gniazdo nie ma tego oglądać.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use diktate_config::GtkWhisperConfig;
use diktate_core::prompt;

use crate::view::DocumentView;

/// Odstęp między ostatnią zmianą bufora a wysyłką kontekstu.
const DEBOUNCE: Duration = Duration::from_millis(400);

/// Skleja prompt z szablonu i ogona dokumentu. Ogon jest już obcięty do
/// `context_chars` przez wywołującego; tutaj zostaje samo podstawienie.
pub fn compose(config: &GtkWhisperConfig, before: &str) -> String {
    if config.context_chars == 0 {
        return prompt::render(&config.prompt, "");
    }
    prompt::render(&config.prompt, before)
}

/// Wysyłka kontekstu: pamięta ostatnio wysłany prompt i odbija zmiany
/// w czasie.
pub struct ContextSender {
    config: RefCell<GtkWhisperConfig>,
    last: RefCell<String>,
    pending: std::cell::Cell<bool>,
    /// Klient dociera później niż okno: połączenie staje w osobnym wątku
    /// i trafia do `Ui::attach`. Do tego czasu kontekst nie ma dokąd iść.
    client: RefCell<Option<Rc<crate::client::Client>>>,
}

impl ContextSender {
    pub fn new(config: GtkWhisperConfig) -> Rc<Self> {
        Rc::new(Self {
            config: RefCell::new(config),
            last: RefCell::new(String::new()),
            pending: std::cell::Cell::new(false),
            client: RefCell::new(None),
        })
    }

    /// Nowe ustawienia z okna ustawień. Kasuje pamięć ostatnio wysłanego
    /// promptu — zmiana szablonu ma dojść do daemona, choćby dokument
    /// się nie ruszył.
    ///
    /// Okno ustawień zacznie ją wołać dopiero w kolejnym zadaniu — stąd
    /// `allow` niżej.
    #[allow(dead_code)]
    pub fn set_config(&self, config: GtkWhisperConfig) {
        *self.config.borrow_mut() = config;
        self.last.borrow_mut().clear();
    }

    /// Podłączony klient. Wołane raz, z `Ui::attach`.
    pub fn set_client(&self, client: Rc<crate::client::Client>) {
        *self.client.borrow_mut() = Some(client);
    }

    /// Wysyłka po odbiciu: kolejne zmiany w trakcie oczekiwania nie dokładają
    /// kolejnych timerów, tylko korzystają z tego, który już biegnie.
    pub fn schedule(self: &Rc<Self>, document: &Rc<DocumentView>) {
        if self.pending.get() {
            return;
        }
        self.pending.set(true);
        let sender = Rc::clone(self);
        let document = Rc::clone(document);
        gtk4::glib::timeout_add_local_once(DEBOUNCE, move || {
            sender.pending.set(false);
            sender.send_now(&document);
        });
    }

    /// Wysyłka natychmiastowa — po zapisaniu ustawień i przy starcie sesji.
    pub fn send_now(&self, document: &Rc<DocumentView>) {
        let Some(client) = self.client.borrow().clone() else {
            return;
        };
        let config = self.config.borrow().clone();
        if config.prompt.trim().is_empty() {
            return;
        }
        let before = document.context_before(config.context_chars);
        let composed = compose(&config, &before);
        if composed == *self.last.borrow() {
            return;
        }
        // Kontekst jest podpowiedzią, nie czynnością użytkownika: nieudana
        // wysyłka trafia do logu i tyle. Okno nie ma o niej krzyczeć.
        match client.send_context(&composed) {
            Ok(()) => *self.last.borrow_mut() = composed,
            Err(err) => eprintln!("diktate-gtk: nie udało się wysłać kontekstu: {err:#}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(prompt: &str, chars: usize) -> GtkWhisperConfig {
        GtkWhisperConfig {
            prompt: prompt.to_string(),
            context_chars: chars,
        }
    }

    #[test]
    fn szablon_dostaje_ogon_dokumentu() {
        let out = compose(&config("Terminy: ggml.\n{kontekst}", 200), "Ostatnie zdanie.");
        assert_eq!(out, "Terminy: ggml.\nOstatnie zdanie.");
    }

    #[test]
    fn pusty_szablon_nie_wysyla_niczego() {
        assert_eq!(compose(&config("", 200), "Ostatnie zdanie."), "");
    }

    #[test]
    fn szablon_bez_symbolu_nie_niesie_dokumentu() {
        let out = compose(&config("Terminy: ggml.", 200), "Ostatnie zdanie.");
        assert_eq!(out, "Terminy: ggml.");
    }

    #[test]
    fn zerowa_dlugosc_kontekstu_daje_sam_szablon() {
        let out = compose(&config("Terminy: ggml.\n{kontekst}", 0), "Ostatnie zdanie.");
        assert_eq!(out, "Terminy: ggml.");
    }
}
