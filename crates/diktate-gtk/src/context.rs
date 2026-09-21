//! Kontekst dla whispera: co klient wysyła daemonowi i jak często.
//!
//! Prompt powstaje z szablonu użytkownika i ogona tekstu sprzed kursora
//! (`docs/07#kontekst-dla-whispera`). Wysyłka jest odbita w czasie i
//! pomijana, gdy prompt się nie zmienił — pisanie z klawiatury zmienia
//! bufor przy każdym znaku, a gniazdo nie ma tego oglądać. Zaznaczenie
//! tekstu odbija się tą samą drogą (`main.rs`, `connect_mark_set` na
//! `insert`/`selection_bound`): to ono przesuwa punkt, od którego liczy się
//! ogon (`DocumentView::context_before`), więc musi wywoływać tę samą
//! wysyłkę co zmiana bufora.
//!
//! Pusty szablon nie znaczy „nic nie rób" — znaczy „wyślij pusty prompt",
//! żeby daemon wrócił do własnego `whisper.initial_prompt` zamiast zostać
//! z ogonem dokumentu sprzed chwili, gdy ktoś skasował pole w ustawieniach.
//! Wyjątek jest jeden: stan startowy, w którym nic jeszcze nie wysłano —
//! czyszczenie slotu, który i tak jest pusty, byłoby wysyłką bez powodu.

use std::cell::{Cell, RefCell};
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
    /// Ostatni prompt potwierdzony wysłany do daemona. `None` znaczy „slot
    /// uznajemy za pusty" — tak samo stan startowy (nic jeszcze nie
    /// poszło), jak i stan po udanym wysłaniu pustego promptu; oba mają
    /// wychodzić z dedupu tak samo, bez rozróżniania.
    last: RefCell<Option<String>>,
    /// Wymusza najbliższą wysyłkę niezależnie od `last` — ustawiane przez
    /// [`Self::set_config`]. Bez tego wyczyszczenie szablonu w ustawieniach
    /// nigdy by nie doszło do daemona: nowy `composed` bywa pusty tak samo
    /// jak stan startowy, więc zwykłe porównanie z `last == None` by je
    /// pomyliło.
    force: Cell<bool>,
    pending: Cell<bool>,
    /// Klient dociera później niż okno: połączenie staje w osobnym wątku
    /// i trafia do `Ui::attach`. Do tego czasu kontekst nie ma dokąd iść.
    client: RefCell<Option<Rc<crate::client::Client>>>,
}

impl ContextSender {
    pub fn new(config: GtkWhisperConfig) -> Rc<Self> {
        Rc::new(Self {
            config: RefCell::new(config),
            last: RefCell::new(None),
            force: Cell::new(false),
            pending: Cell::new(false),
            client: RefCell::new(None),
        })
    }

    /// Nowe ustawienia z okna ustawień. Następna wysyłka ma polecieć zawsze,
    /// choćby dokument się nie ruszył i choćby nowy szablon dawał pusty
    /// prompt — inaczej wyczyszczenie pola w ustawieniach nigdy nie
    /// dotarłoby do daemona (usterka: pusty szablon zostawiał daemona
    /// z nieaktualnym kontekstem na zawsze, do restartu klienta).
    pub fn set_config(&self, config: GtkWhisperConfig) {
        *self.config.borrow_mut() = config;
        self.force.set(true);
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

    /// Wysyłka natychmiastowa — po zapisaniu ustawień, przy starcie sesji
    /// i po odbiciu z [`Self::schedule`].
    pub fn send_now(&self, document: &Rc<DocumentView>) {
        let Some(client) = self.client.borrow().clone() else {
            return;
        };
        let config = self.config.borrow().clone();
        let before = document.context_before(config.context_chars);
        let composed = compose(&config, &before);

        // `force` jest konsumowany niezależnie od tego, czy w ogóle doszło
        // do próby wysłania — inaczej ustawienia zapisane bez połączonego
        // klienta zgubiłyby wymuszenie na starcie tej metody.
        let force = self.force.take();
        if !decide(&self.last.borrow(), force, &composed) {
            return;
        }

        // Kontekst jest podpowiedzią, nie czynnością użytkownika: nieudana
        // wysyłka trafia do logu i tyle. Okno nie ma o niej krzyczeć.
        match client.send_context(&composed) {
            Ok(()) => {
                *self.last.borrow_mut() = if composed.is_empty() {
                    None
                } else {
                    Some(composed)
                };
            }
            Err(err) => eprintln!("diktate-gtk: nie udało się wysłać kontekstu: {err:#}"),
        }
    }
}

/// Rozstrzyga, czy `send_now` ma faktycznie wywołać `send_context` —
/// wydzielone z ciała metody, żeby dało się to sprawdzić bez gniazda,
/// klienta i GTK-a.
///
/// `force` (ustawiane przez [`ContextSender::set_config`]) omija dedup
/// całkowicie: zmiana w oknie ustawień ma dojść do daemona zawsze, także
/// gdy nowy `composed` jest pusty tak samo jak stan startowy. Bez `force`
/// wysyłka leci, gdy `composed` różni się od ostatnio potwierdzonego —
/// a przy `last == None` (nic jeszcze nie wysłano) tylko wtedy, gdy
/// `composed` nie jest puste, bo czyszczenie pustego slotu byłoby wysyłką
/// bez powodu.
fn decide(last: &Option<String>, force: bool, composed: &str) -> bool {
    if force {
        return true;
    }
    match last {
        Some(prev) => prev != composed,
        None => !composed.is_empty(),
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

    // --- decide: usterka „wyczyszczenie szablonu nie czyści slotu" ---

    #[test]
    fn stan_startowy_z_pustym_promptem_nie_wysyla_niczego() {
        // Nic jeszcze nie wysłano, a nowy prompt też jest pusty — nie ma
        // czego czyścić w slocie, który i tak jest pusty.
        assert!(!decide(&None, false, ""));
    }

    #[test]
    fn pierwsza_wysylka_niepustego_promptu_leci() {
        assert!(decide(&None, false, "Terminy: ggml."));
    }

    #[test]
    fn wyczyszczenie_szablonu_po_niepustym_promptcie_wysyla_puste() {
        // Sedno usterki: był wysłany niepusty prompt, teraz szablon jest
        // pusty — musi polecieć czyszczenie, a nie cisza.
        assert!(decide(&Some("Terminy: ggml.".to_string()), false, ""));
    }

    #[test]
    fn niezmieniony_prompt_nie_wysyla_sie_ponownie() {
        assert!(!decide(
            &Some("Terminy: ggml.".to_string()),
            false,
            "Terminy: ggml."
        ));
    }

    #[test]
    fn wymuszenie_po_ustawieniach_wysyla_mimo_pustego_i_niezmienionego() {
        // `force` (ustawienia zapisane) omija dedup całkowicie — nawet gdy
        // stan startowy sugerowałby ciszę, a nawet gdy prompt się nie
        // zmienił.
        assert!(decide(&None, true, ""));
        assert!(decide(&Some("Terminy: ggml.".to_string()), true, "Terminy: ggml."));
    }
}
