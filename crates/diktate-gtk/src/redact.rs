//! Redakcja notatki poleceniem podyktowanym głosem.
//!
//! Korekta z [`crate::correct`] odpowiada na pytanie „co whisper przekręcił".
//! Redakcja odpowiada na inne: „zrób z tym tekstem to, o co proszę". Dlatego
//! instrukcją nie jest prompt wpisany w konfigurację, tylko zdanie
//! użytkownika — „zmień na bardziej formalny", „usuń powtórzenia", „przepisz
//! w czasie przyszłym" — a celem jest zaznaczenie albo, gdy go nie ma, cała
//! notatka (`docs/11-korekta-llm.md#redakcja-poleceniem-głosowym`).
//!
//! Polecenie podaje się **głosem**, tą samą drogą co dyktowanie: okno
//! polecenia rejestruje się w [`Sink`] jako odbiorca wypowiedzi i przez ten
//! czas transkrypcja nie dopływa do notatki, tylko do pola polecenia. Nie
//! wymagało to zmiany protokołu — daemon nadal nie wie nic o redakcji, a
//! rozgałęzienie mieszka w oknie. Pole jest edytowalne, bo przekręcone
//! polecenie poprawia się szybciej palcem niż drugim podejściem.
//!
//! Zabezpieczenia są celowo inne niż przy korekcie. Progi odsetka zmian tu
//! nie obowiązują ([`correct::review_redaction`]) — polecenie „zmień na
//! formalny" **ma** zmienić dużo. Broni za to jedna operacja logu, którą
//! `Ctrl+Z` cofa w całości, walidacja znakowa i sprawdzenie, czy tekst celu
//! nie zmienił się w trakcie czekania na model: zaległa wypowiedź, która
//! dopłynęła z transkrypcji, nie może zniknąć pod podmianą.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use diktate_config::CorrectionConfig;
use libadwaita::prelude::*;
use libadwaita::{
    ApplicationWindow, Clamp, EntryRow, HeaderBar, PreferencesGroup, Toast, ToastOverlay,
    ToolbarView, Window, WindowTitle,
};

use crate::correct;
use crate::view::{DocumentView, Mark, Target};

/// Odbiorca wypowiedzi na czas dyktowania polecenia.
///
/// Jedno miejsce, w którym okno rozstrzyga, dokąd idzie tekst z transkrypcji:
/// do notatki czy do polecenia. Bez tego rozgałęzienia okno polecenia
/// musiałoby zakładać własną sesję z daemonem, a dwie sesje naraz to dwa
/// mikrofony i dwa razy więcej stanu do pilnowania.
#[derive(Default)]
pub struct Sink {
    handler: RefCell<Option<Listener>>,
    /// Chwila otwarcia okna polecenia. Wypowiedzi sprzed niej należą do
    /// notatki, choćby wyszły z transkrypcji dopiero teraz.
    mark: Cell<Option<Mark>>,
}

/// Nasłuchujący wypowiedzi — pole polecenia otwartego okna redakcji.
type Listener = Rc<dyn Fn(&str)>;

impl Sink {
    pub fn new() -> Rc<Self> {
        Rc::new(Self::default())
    }

    /// Podaje wypowiedź nasłuchującemu. `true` znaczy „przechwycone" — tekst
    /// nie ma już trafić do notatki.
    ///
    /// `spoken_after` rozstrzyga, czy wypowiedź padła po otwarciu okna
    /// polecenia. Zaległość sprzed niego **nie jest poleceniem**: wyszła
    /// z transkrypcji dopiero teraz, ale powiedziana została do notatki,
    /// a zjedzenie jej byłoby stratą bez śladu (reguła z `docs/07`).
    pub fn deliver(&self, text: &str, spoken_after: impl FnOnce(Mark) -> bool) -> bool {
        let handler = self.handler.borrow().clone();
        let (Some(handler), Some(mark)) = (handler, self.mark.get()) else {
            return false;
        };
        if !spoken_after(mark) {
            return false;
        }
        handler(text);
        true
    }

    fn listen(&self, mark: Mark, handler: Listener) {
        self.mark.set(Some(mark));
        *self.handler.borrow_mut() = Some(handler);
    }

    fn silence(&self) {
        self.mark.set(None);
        self.handler.borrow_mut().take();
    }
}

/// Wszystko, czego redakcja potrzebuje od okna dyktowania.
///
/// `busy` jest wspólne z korektą: obie czynności chodzą do tego samego
/// modelu i obie podmieniają tekst, więc druga w trakcie pierwszej nie
/// miałaby czego redagować.
pub struct Redaction {
    pub window: ApplicationWindow,
    pub toasts: ToastOverlay,
    pub document: Rc<DocumentView>,
    pub subtitle: Rc<crate::Subtitle>,
    pub busy: Rc<Cell<bool>>,
    pub sink: Rc<Sink>,
    pub record_button: gtk4::ToggleButton,
    pub button: gtk4::Button,
    pub config: Arc<CorrectionConfig>,
}

impl Redaction {
    /// Podpina przycisk redakcji do okna polecenia.
    pub fn wire(self: &Rc<Self>) {
        let this = Rc::clone(self);
        self.button
            .connect_clicked(move |_| Rc::clone(&this).open());
        self.refresh_button();
    }

    /// Ustawia czynność przycisku zgodnie ze stanem okna.
    ///
    /// Dyktowania **nie** bierze pod uwagę: redakcja przydaje się właśnie
    /// w trakcie mówienia, a wypowiedzi sprzed otwarcia okna polecenia
    /// odsiewa znacznik w [`Sink`], nie zablokowany przycisk.
    pub fn refresh_button(&self) {
        let ma_tekst = !self.document.text().trim().is_empty();
        self.button.set_sensitive(ma_tekst && !self.busy.get());
    }

    fn open(self: Rc<Self>) {
        if self.busy.get() || self.document.text().trim().is_empty() {
            return;
        }
        let target = Rc::new(self.document.target());

        let title = WindowTitle::new("Polecenie", "Mów, co zmienić…");
        let entry = EntryRow::builder().title("Polecenie").build();
        let group = PreferencesGroup::builder()
            .title(if target.whole {
                "Cała notatka"
            } else {
                "Zaznaczony fragment"
            })
            .description(preview(&target.text))
            .build();
        group.add(&entry);

        let clamp = Clamp::builder()
            .child(&group)
            .maximum_size(480)
            .margin_top(12)
            .margin_bottom(12)
            .margin_start(12)
            .margin_end(12)
            .build();

        let cancel = gtk4::Button::with_label("Anuluj");
        let run = gtk4::Button::with_label("Wykonaj");
        run.add_css_class("suggested-action");

        let header = HeaderBar::new();
        header.set_title_widget(Some(&title));
        header.set_show_end_title_buttons(false);
        header.set_show_start_title_buttons(false);
        header.pack_start(&cancel);
        header.pack_end(&run);

        let toolbar = ToolbarView::new();
        toolbar.add_top_bar(&header);
        toolbar.set_content(Some(&clamp));

        let dialog = Window::builder()
            .transient_for(&self.window)
            .modal(true)
            .title("Polecenie")
            .default_width(460)
            .default_height(200)
            .content(&toolbar)
            .build();

        // Wypowiedzi z transkrypcji idą od teraz do pola polecenia. Kolejne
        // doklejają się spacją — polecenie bywa dwuczęściowe („usuń
        // powtórzenia", „i skróć").
        let entry_for_sink = entry.clone();
        self.sink.listen(
            self.document.mark(),
            Rc::new(move |text: &str| {
                let text = text.trim();
                if text.is_empty() {
                    return;
                }
                let mut polecenie = entry_for_sink.text().to_string();
                if !polecenie.is_empty() {
                    polecenie.push(' ');
                }
                polecenie.push_str(text);
                entry_for_sink.set_text(&polecenie);
            }),
        );

        // Mikrofon idzie tą samą drogą co kliknięcie przycisku nagrywania:
        // jeden tor stanu, jeden wskaźnik poziomu, żadnej drugiej sesji.
        // Gdy dyktowanie już trwa, nie ruszamy go wcale — i tak samo nie
        // zatrzymamy go przy zamknięciu okna.
        let nagrywało = self.record_button.is_active();
        if !nagrywało {
            self.record_button.set_active(true);
        }

        // Zamknięcie okna zawsze sprząta po sobie — wszystko jedno, którą
        // drogą wyszliśmy: przyciskiem, Escape czy krzyżykiem.
        let sink_for_close = Rc::clone(&self.sink);
        let record_for_close = self.record_button.clone();
        let this_for_close = Rc::clone(&self);
        dialog.connect_close_request(move |_| {
            sink_for_close.silence();
            // Mikrofon gasimy tylko wtedy, gdy sami go włączyliśmy —
            // przerwane w pół słowa dyktowanie byłoby gorsze od
            // niedokończonego polecenia.
            if !nagrywało && record_for_close.is_active() {
                record_for_close.set_active(false);
            }
            this_for_close.refresh_button();
            gtk4::glib::Propagation::Proceed
        });

        let escape = gtk4::EventControllerKey::new();
        let dialog_for_escape = dialog.clone();
        escape.connect_key_pressed(move |_, key, _, _| match key {
            gtk4::gdk::Key::Escape => {
                dialog_for_escape.close();
                gtk4::glib::Propagation::Stop
            }
            _ => gtk4::glib::Propagation::Proceed,
        });
        dialog.add_controller(escape);

        let dialog_for_cancel = dialog.clone();
        cancel.connect_clicked(move |_| dialog_for_cancel.close());

        let start = {
            let this = Rc::clone(&self);
            let dialog = dialog.clone();
            let entry = entry.clone();
            let title = title.clone();
            let target = Rc::clone(&target);
            move || {
                let instruction = entry.text().trim().to_string();
                if instruction.is_empty() {
                    title.set_subtitle("Polecenie jest puste — powiedz, co zmienić");
                    return;
                }
                dialog.close();
                Rc::clone(&this).execute(Rc::clone(&target), instruction);
            }
        };
        let start_for_entry = start.clone();
        entry.connect_entry_activated(move |_| start_for_entry());
        run.connect_clicked(move |_| start());

        self.refresh_button();
        dialog.present();
    }

    /// Wysyła polecenie do modelu osobnym wątkiem. Odpowiedź to sekundy,
    /// a pętla GTK nie ma prawa na nią czekać.
    fn execute(self: Rc<Self>, target: Rc<Target>, instruction: String) {
        // Cel „cała notatka" bierzemy na nowo: dyktowanie mogło w trakcie
        // dyktowania polecenia dosłać zaległe zdanie, a „całość" znaczy
        // całość z chwili wydania polecenia, nie sprzed niego. Zaznaczenie
        // zostaje takie, jakie było — po to je zapamiętaliśmy kotwicami.
        let target = if target.whole {
            Rc::new(self.document.target())
        } else {
            target
        };
        self.busy.set(true);
        self.subtitle.set_busy(Some(crate::BUSY_REDACTING));
        self.refresh_button();

        let (tx, rx) = async_channel::bounded::<anyhow::Result<String>>(1);
        let config = Arc::clone(&self.config);
        let text = target.text.clone();
        std::thread::spawn(move || {
            let _ = tx.send_blocking(correct::ask_redaction(&config, &instruction, &text));
        });

        let this = Rc::clone(&self);
        gtk4::glib::spawn_future_local(async move {
            let result = rx.recv().await;
            this.busy.set(false);
            this.subtitle.set_busy(None);
            this.refresh_button();

            match result {
                Ok(Ok(after)) => this.apply(&target, &after),
                Ok(Err(err)) => this.toast(&format!("Redakcja: {err:#}")),
                // Wątek zniknął bez odpowiedzi — nie ma czego pokazywać poza
                // tym, że nic z tego nie wyszło.
                Err(_) => this.toast("Redakcja przerwana"),
            }
        });
    }

    /// Wpisuje zredagowany tekst w cel. Jedna operacja logu, więc `Ctrl+Z`
    /// cofa redakcję w całości — i o tym właśnie mówi powiadomienie.
    fn apply(&self, target: &Target, after: &str) {
        match correct::review_redaction(&self.config, &target.text, after) {
            correct::Verdict::Reject { reason } => {
                self.toast(&format!("Redakcja odrzucona: {reason}"))
            }
            correct::Verdict::Accept { ratio } | correct::Verdict::Confirm { ratio, .. } => {
                if self.document.replace_range(target, after) {
                    self.toast(&format!(
                        "Zredagowano — zmienionych słów: {}%. Ctrl+Z cofa.",
                        correct::percent(ratio)
                    ));
                } else {
                    self.toast("Tekst zmienił się w trakcie redakcji — nic nie podmieniam");
                }
            }
        }
    }

    fn toast(&self, text: &str) {
        self.toasts.add_toast(Toast::new(text));
    }
}

/// Skrócony podgląd celu do opisu grupy: użytkownik ma widzieć, czego
/// dotyczy polecenie, zanim je wypowie.
fn preview(text: &str) -> String {
    const LIMIT: usize = 120;
    let jedną_linią = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if jedną_linią.chars().count() <= LIMIT {
        return jedną_linią;
    }
    let skrót: String = jedną_linią.chars().take(LIMIT).collect();
    format!("{}…", skrót.trim_end())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn podglad_celu_miesci_sie_w_jednej_linii() {
        let cel = "Dobra,\n  zaczynamy   pracę.";
        assert_eq!(preview(cel), "Dobra, zaczynamy pracę.");
    }

    #[test]
    fn dlugi_cel_jest_przycinany_z_wielokropkiem() {
        let cel = "słowo ".repeat(100);
        let podgląd = preview(&cel);
        assert!(podgląd.ends_with('…'), "a jest {podgląd}");
        assert!(podgląd.chars().count() <= 121, "a ma {}", podgląd.len());
    }

    #[test]
    fn wypowiedz_bez_nasluchujacego_zostaje_nieprzechwycona() {
        // To jest warunek, pod którym dyktowanie działa jak przedtem:
        // dopóki okno polecenia nie jest otwarte, tekst idzie do notatki.
        let sink = Sink::new();
        assert!(!sink.deliver("Ala ma kota", |_| true));
    }

    #[test]
    fn nasluchujacy_przechwytuje_wypowiedz() {
        let sink = Sink::new();
        let usłyszane = Rc::new(RefCell::new(String::new()));
        let cel = Rc::clone(&usłyszane);
        sink.listen(
            Mark::at(1, 1_000),
            Rc::new(move |text: &str| cel.borrow_mut().push_str(text)),
        );
        assert!(sink.deliver("zmień na formalny", |_| true));
        assert_eq!(usłyszane.borrow().as_str(), "zmień na formalny");

        sink.silence();
        assert!(
            !sink.deliver("i skróć", |_| true),
            "po zamknięciu okna tekst wraca do notatki"
        );
        assert_eq!(usłyszane.borrow().as_str(), "zmień na formalny");
    }

    #[test]
    fn zaleglosc_sprzed_otwarcia_okna_nie_jest_poleceniem() {
        // Najważniejszy przypadek całego rozgałęzienia: wypowiedź, która
        // padła przed otwarciem okna, a wyszła z transkrypcji dopiero teraz,
        // jest treścią notatki. Zjedzenie jej byłoby stratą bez śladu.
        let sink = Sink::new();
        let usłyszane = Rc::new(RefCell::new(String::new()));
        let cel = Rc::clone(&usłyszane);
        sink.listen(
            Mark::at(1, 4_000),
            Rc::new(move |text: &str| cel.borrow_mut().push_str(text)),
        );
        assert!(!sink.deliver("to jest zdanie notatki", |_| false));
        assert!(usłyszane.borrow().is_empty());
    }
}
