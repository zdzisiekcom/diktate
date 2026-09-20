//! Most między modelem dokumentu a `GtkTextBuffer`.
//!
//! Źródłem prawdy jest [`Document`], a bufor jest **widokiem** — to tu, i
//! tylko tu, kotwice zamieniają się na globalne offsety znakowe, których
//! reszta systemu nie zna (`docs/03-model-dokumentu.md`).
//!
//! Ruch idzie w obie strony. Wypowiedź z daemona wchodzi do modelu, a model
//! zwraca opis jednej lokalnej zmiany, którą nakładamy na bufor — nie
//! przerysowujemy go w całości, bo zgubiłoby to zaznaczenie, karetkę
//! i przewinięcie. Odwrotnie: każda edycja klawiaturą jest przechwytywana
//! *przed* wykonaniem i zapisywana w tym samym logu operacji co dyktowanie.
//! Dzięki temu `Ctrl+Z` cofa jedno i drugie jednym mechanizmem, zamiast
//! walczyć z osobnym cofaniem `GtkTextBuffer` (dlatego je wyłączamy).
//!
//! Kotwica wstawiania podąża za karetką zgodnie z regułami z
//! `docs/07-zachowanie-dyktowania.md`: przesuwa się za wstawionym tekstem
//! i za karetką przesuniętą przez użytkownika, ale **nie rusza się przy
//! niepustym zaznaczeniu** — zaznaczenie pamięta miejsce sprzed siebie.
//!
//! Niepuste zaznaczenie znaczy „następną wypowiedzią zastąp to" i uzbraja się
//! samo. O tym, czy przychodzący segment zastępuje, czy tylko dopływa,
//! rozstrzyga **oś czasu audio**, nie kolejność przybycia: wypowiedź, która
//! padła przed uzbrojeniem, a dotarła po nim (bo GPU nie nadążał), wchodzi
//! w kotwicę wstawiania i zostawia zaznaczenie uzbrojone. Bez tej reguły
//! poprawka bywa zjadana przez zdanie sprzed niej — błąd prototypu opisany
//! w `docs/07#rozstrzyganie-po-osi-czasu-audio`.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Instant;

use diktate_config::DictationConfig;
use diktate_document::{Anchor, Document, Normalization};
use diktate_proto::SegmentAdded;
use gtk4::prelude::*;
use gtk4::{TextBuffer, TextView};

/// Model dokumentu wraz z buforem, na który jest rzutowany.
pub struct DocumentView {
    document: RefCell<Document>,
    buffer: TextBuffer,
    view: TextView,
    /// Kotwica wstawiania — miejsce, w którym ląduje następna wypowiedź.
    insert_anchor: Cell<Anchor>,
    /// Podniesiona, gdy to my zmieniamy bufor. Bez tego nasza własna zmiana
    /// wróciłaby przez sygnały edycji i zapisała się w logu drugi raz.
    applying: Cell<bool>,
    /// Uzbrojone zastąpienie — zapamiętane zaznaczenie wraz z momentem
    /// powstania na osi czasu audio.
    replace: RefCell<Option<Armed>>,
    /// Początek bieżącej sesji audio w czasie ściennym. Daemon liczy
    /// `audio_start_ms` od startu nagrywania, więc tylko stąd wiemy, którą
    /// chwilę osi czasu audio widzi użytkownik, gdy zaznacza tekst.
    audio_origin: Cell<Option<Instant>>,
    /// Numer sesji audio. Każde nagranie zaczyna oś czasu od zera, więc
    /// samo `at_ms` nie wystarcza do porównania między sesjami.
    session_epoch: Cell<u64>,
    /// Powiadomienie dla okna: zastąpienie uzbrojone albo zwolnione.
    on_armed: RefCell<Option<ArmedNotify>>,
    normalization: Normalization,
    config: DictationConfig,
}

/// Powiadomienie okna o zmianie uzbrojenia zastąpienia.
type ArmedNotify = Box<dyn Fn(bool)>;

/// Zakres, na którym ma zadziałać redakcja poleceniem głosowym.
///
/// `whole` nie da się wyliczyć z samych kotwic po fakcie — zaznaczenie
/// obejmujące całą notatkę to co innego niż jej brak, choć zakres wychodzi
/// ten sam. Okno mówi o tym użytkownikowi, więc różnica musi przetrwać.
pub struct Target {
    from: Anchor,
    to: Anchor,
    /// Tekst celu w chwili naciśnięcia przycisku — to on idzie do modelu
    /// i on decyduje, czy podmiana jest jeszcze aktualna.
    pub text: String,
    /// Czy celem jest cała notatka (brak zaznaczenia).
    pub whole: bool,
}

/// Zapamiętane zaznaczenie czekające na wypowiedź, która je zastąpi.
#[derive(Debug, Clone, Copy)]
struct Armed {
    from: Anchor,
    to: Anchor,
    /// Sesja audio, w której zaznaczenie powstało.
    epoch: u64,
    /// `replace_armed_at` — moment uzbrojenia na osi czasu audio sesji.
    at_ms: u64,
}

impl DocumentView {
    /// Podpina model pod widok tekstu i podłącza sygnały edycji.
    pub fn attach(view: &TextView, config: DictationConfig) -> Rc<Self> {
        let normalization = Normalization {
            auto_space: config.auto_space,
            strip_trailing_ellipsis: config.strip_trailing_ellipsis,
        };
        let buffer = view.buffer();
        // Własne cofanie `GtkTextBuffer` musi zniknąć: historia dokumentu
        // jest jedna i leży w logu operacji.
        buffer.set_enable_undo(false);

        let this = Rc::new(Self {
            document: RefCell::new(Document::new()),
            buffer: buffer.clone(),
            view: view.clone(),
            insert_anchor: Cell::new(Anchor::Start),
            applying: Cell::new(false),
            replace: RefCell::new(None),
            audio_origin: Cell::new(None),
            session_epoch: Cell::new(0),
            on_armed: RefCell::new(None),
            normalization,
            config,
        });

        let for_insert = Rc::clone(&this);
        buffer.connect_insert_text(move |_, iter, text| {
            for_insert.mirror_keyboard_insert(iter.offset() as usize, text);
        });

        let for_delete = Rc::clone(&this);
        buffer.connect_delete_range(move |_, start, end| {
            for_delete.mirror_keyboard_delete(start.offset() as usize, end.offset() as usize);
        });

        let for_cursor = Rc::clone(&this);
        buffer.connect_cursor_position_notify(move |_| {
            for_cursor.follow_caret();
        });

        // Zaznaczenie uzbraja zastąpienie samo, więc musimy widzieć każdy ruch
        // obu znaczników — `has-selection` nie drgnie przy zmianie zakresu
        // w trakcie przeciągania, a to ono wyznacza moment uzbrojenia.
        let for_selection = Rc::clone(&this);
        buffer.connect_mark_set(move |buffer, _, mark| {
            let name = mark.name();
            let name = name.as_deref();
            if name == Some("insert") || name == Some("selection_bound") {
                let _ = buffer;
                for_selection.sync_selection();
            }
        });

        this
    }

    /// Przyjmuje wypowiedź: zastępuje nią uzbrojone zaznaczenie albo wstawia
    /// ją w kotwicę wstawiania. Rozstrzyga oś czasu audio, nie kolejność
    /// przybycia (`docs/07`).
    pub fn insert_segment(&self, segment: &SegmentAdded) {
        match self.armed_for(segment.audio_start_ms) {
            Some(armed) => self.apply_splice(segment, armed),
            None => self.apply_insert(segment),
        }
    }

    /// Zaczyna nową sesję audio — jej oś czasu rusza od zera. Wywoływane, gdy
    /// daemon potwierdzi start nagrywania.
    pub fn begin_audio_session(&self) {
        self.session_epoch.set(self.session_epoch.get() + 1);
        self.audio_origin.set(Some(Instant::now()));
    }

    /// Podpina powiadomienie o uzbrojeniu zastąpienia — okno pokazuje wtedy
    /// „zastępuję…" i wyróżnia zaznaczenie.
    pub fn connect_armed<F: Fn(bool) + 'static>(&self, notify: F) {
        *self.on_armed.borrow_mut() = Some(Box::new(notify));
    }

    /// Czy jakieś zaznaczenie czeka na zastąpienie.
    pub fn is_armed(&self) -> bool {
        self.replace.borrow().is_some()
    }

    /// `Escape`: znosi zaznaczenie i wraca do zwykłego wstawiania, bez żadnej
    /// zmiany w dokumencie. `false`, gdy nie było czego znosić.
    pub fn cancel_replace(&self) -> bool {
        if !self.is_armed() {
            return false;
        }
        // Samo zdjęcie zaznaczenia w buforze przejdzie przez `sync_selection`
        // i rozbroi zastąpienie — nie robimy tego drugi raz ręcznie.
        let caret = self.buffer.iter_at_offset(self.buffer.cursor_position());
        self.buffer.place_cursor(&caret);
        true
    }

    /// Cofa ostatnią operację — dyktowanie i pisanie jednym mechanizmem.
    pub fn undo(&self) -> bool {
        if !self.document.borrow_mut().undo() {
            return false;
        }
        self.rebuild();
        true
    }

    pub fn redo(&self) -> bool {
        if !self.document.borrow_mut().redo() {
            return false;
        }
        self.rebuild();
        true
    }

    /// Czyści całą notatkę. Idzie tą samą drogą co kasowanie z klawiatury,
    /// więc operacja trafia do logu i `Ctrl+Z` przywraca tekst. `false`, gdy
    /// nie było czego czyścić.
    pub fn clear(&self) -> bool {
        let (mut start, mut end) = self.buffer.bounds();
        if start == end {
            return false;
        }
        self.buffer.delete(&mut start, &mut end);
        true
    }

    /// Cały tekst notatki — do schowka.
    pub fn text(&self) -> String {
        self.document.borrow().text()
    }

    /// Podmienia **całą** notatkę jedną operacją logu — tą drogą wchodzi
    /// korekta LLM-em (`docs/11-korekta-llm.md`). Jedna operacja, a nie
    /// kasowanie plus wstawienie, bo `Ctrl+Z` ma cofać korektę w całości,
    /// jednym naciśnięciem, a nie zostawiać po drodze pustej notatki.
    pub fn replace_all(&self, text: &str) {
        let to = self.document.borrow().end_anchor();
        self.document.borrow_mut().set_text(Anchor::Start, to, text);
        self.rebuild();
        // Karetka na końcu: dyktowanie po korekcie płynie dalej stamtąd,
        // gdzie tekst się kończy. `rebuild` ustawił tam już kotwicę.
        self.place_caret(self.insert_anchor.get());
    }

    /// Cel redakcji poleceniem głosowym: zaznaczenie, a gdy go nie ma — cała
    /// notatka (`docs/11#redakcja-poleceniem-głosowym`).
    ///
    /// Zakres zapamiętujemy **kotwicami**, nie offsetami w buforze: między
    /// naciśnięciem przycisku a odpowiedzią modelu mija kilka sekund, w czasie
    /// których do notatki może dopłynąć zaległa wypowiedź i przesunąć każdy
    /// offset. Kotwica to przeżyje, liczba nie.
    pub fn target(&self) -> Target {
        let document = self.document.borrow();
        let zaznaczenie = self.buffer.selection_bounds().and_then(|(start, end)| {
            let from = document.anchor_at_char(start.offset() as usize)?;
            let to = document.anchor_at_char(end.offset() as usize)?;
            Some((from, to))
        });
        match zaznaczenie {
            Some((from, to)) => Target {
                from,
                to,
                text: document.text_between(from, to),
                whole: false,
            },
            None => Target {
                from: Anchor::Start,
                to: document.end_anchor(),
                text: document.text(),
                whole: true,
            },
        }
    }

    /// Podmienia zakres celu jedną operacją logu — tą drogą wchodzi redakcja
    /// poleceniem. Zwraca `false`, gdy tekst celu zmienił się w międzyczasie:
    /// odpowiedź modelu dotyczy wtedy czegoś innego, niż stoi w notatce,
    /// a podmiana zjadłaby wypowiedź, która dopłynęła z transkrypcji.
    pub fn replace_range(&self, target: &Target, text: &str) -> bool {
        if self.document.borrow().text_between(target.from, target.to) != target.text {
            return false;
        }
        self.document
            .borrow_mut()
            .set_text(target.from, target.to, text);
        self.rebuild();
        self.place_caret(self.insert_anchor.get());
        true
    }

    /// Znacznik „od teraz": chwila, od której wypowiedzi są poleceniem
    /// redakcyjnym, a nie treścią notatki.
    pub fn mark(&self) -> Mark {
        Mark {
            epoch: self.session_epoch.get(),
            at_ms: self.audio_now_ms(),
        }
    }

    /// Czy ta wypowiedź padła po znaczniku. Zaległość sprzed niego należy do
    /// notatki — okno polecenia nie ma prawa jej zjeść.
    pub fn spoken_after(&self, mark: Mark, audio_start_ms: u64) -> bool {
        spoken_after(mark, self.session_epoch.get(), audio_start_ms)
    }

    // --- wnętrze ---

    /// Zaznaczenie uzbrojone dla tej wypowiedzi — albo `None`, gdy segment
    /// ma tylko dopłynąć w kotwicę wstawiania.
    fn armed_for(&self, audio_start_ms: u64) -> Option<Armed> {
        let armed = (*self.replace.borrow())?;
        replaces(
            armed.epoch,
            armed.at_ms,
            self.session_epoch.get(),
            audio_start_ms,
        )
        .then_some(armed)
    }

    /// Chwila na osi czasu audio bieżącej sesji. Poza nagrywaniem to zero —
    /// sesja i tak zacznie się od nowa, a `epoch` to wyłapie.
    fn audio_now_ms(&self) -> u64 {
        match self.audio_origin.get() {
            Some(origin) => origin.elapsed().as_millis() as u64,
            None => 0,
        }
    }

    /// Wstawienie w kotwicę. Gdy zastąpienie czeka uzbrojone, wypowiedź
    /// jest zaległością sprzed zaznaczenia — po nałożeniu zmiany odtwarzamy
    /// zaznaczenie z kotwic, bo offsety w buforze właśnie się przesunęły.
    fn apply_insert(&self, segment: &SegmentAdded) {
        // `insert_at_cursor = false` przywraca zachowanie „zawsze na koniec"
        // — ta sama ścieżka kodu, tylko inna kotwica.
        let at = if self.config.insert_at_cursor {
            self.insert_anchor.get()
        } else {
            self.document.borrow().end_anchor()
        };
        let change = self.document.borrow_mut().insert_segment(
            segment.seq,
            &segment.text,
            at,
            &self.normalization,
        );
        self.insert_anchor.set(change.anchor);

        let armed = *self.replace.borrow();
        let follow = self.insertion_point_visible(change.at);
        self.applying.set(true);
        self.splice_buffer(&change);
        match armed {
            // Zaznaczenie zostaje uzbrojone — zaległy segment go nie
            // unieważnia (`docs/07`), a karetka nie ma prawa go zerwać.
            Some(armed) => self.select(armed.from, armed.to),
            None => self.place_caret(change.anchor),
        }
        self.applying.set(false);

        if follow && self.config.follow_caret {
            self.view
                .scroll_to_mark(&self.buffer.get_insert(), 0.0, false, 0.0, 0.0);
        }
    }

    /// Zastąpienie zaznaczenia. Jedna wypowiedź zjada dokładnie jedno
    /// zaznaczenie — po nim tryb wraca do wstawiania, a dyktowanie płynie
    /// dalej od poprawionego miejsca (`gtk.replace.mode = "one_utterance"`).
    fn apply_splice(&self, segment: &SegmentAdded, armed: Armed) {
        let change = self.document.borrow_mut().splice_segment(
            segment.seq,
            &segment.text,
            armed.from,
            armed.to,
            &self.normalization,
        );
        self.insert_anchor.set(change.anchor);

        let follow = self.insertion_point_visible(change.at);
        self.applying.set(true);
        self.splice_buffer(&change);
        self.place_caret(change.anchor);
        self.applying.set(false);
        self.disarm();

        if follow && self.config.follow_caret {
            self.view
                .scroll_to_mark(&self.buffer.get_insert(), 0.0, false, 0.0, 0.0);
        }
    }

    /// Nakłada jedną lokalną zmianę na bufor. Wołane wyłącznie z podniesioną
    /// flagą `applying`.
    fn splice_buffer(&self, change: &diktate_document::Change) {
        let mut at = self.buffer.iter_at_offset(change.at as i32);
        if change.removed > 0 {
            let mut end = self
                .buffer
                .iter_at_offset((change.at + change.removed) as i32);
            self.buffer.delete(&mut at, &mut end);
        }
        self.buffer.insert(&mut at, &change.inserted);
    }

    /// Karetka idzie tam, gdzie stanęła kotwica — dyktowanie płynie dalej od
    /// tego miejsca.
    fn place_caret(&self, anchor: Anchor) {
        let position = self.document.borrow().char_position(anchor);
        let caret = self.buffer.iter_at_offset(position as i32);
        self.buffer.place_cursor(&caret);
    }

    fn select(&self, from: Anchor, to: Anchor) {
        let (start, end) = {
            let document = self.document.borrow();
            (
                document.char_position(from) as i32,
                document.char_position(to) as i32,
            )
        };
        self.buffer.select_range(
            &self.buffer.iter_at_offset(end),
            &self.buffer.iter_at_offset(start),
        );
    }

    /// Zapamiętuje zaznaczenie jako uzbrojone zastąpienie albo je zwalnia.
    /// Moment uzbrojenia to `replace_armed_at` — chwila, w której powstało
    /// **to** zaznaczenie, także po poprawieniu zakresu myszą.
    fn sync_selection(&self) {
        if self.applying.get() {
            return;
        }
        let armed = self.buffer.selection_bounds().and_then(|(start, end)| {
            let document = self.document.borrow();
            let from = document.anchor_at_char(start.offset() as usize)?;
            let to = document.anchor_at_char(end.offset() as usize)?;
            Some(Armed {
                from,
                to,
                epoch: self.session_epoch.get(),
                at_ms: self.audio_now_ms(),
            })
        });
        let was = self.is_armed();
        *self.replace.borrow_mut() = armed;
        let is = self.is_armed();
        if was != is {
            self.notify_armed(is);
        }
    }

    fn disarm(&self) {
        if self.replace.borrow_mut().take().is_some() {
            self.notify_armed(false);
        }
    }

    fn notify_armed(&self, armed: bool) {
        if let Some(notify) = self.on_armed.borrow().as_ref() {
            notify(armed);
        }
    }

    /// Po cofnięciu bufor odtwarzamy w całości: cofnięcie jest rzadkie,
    /// a jego wynik może sięgać dowolnie daleko w tekście.
    fn rebuild(&self) {
        let (text, anchor) = {
            let document = self.document.borrow();
            let anchor = document
                .anchor_at_char(document.char_len())
                .unwrap_or(Anchor::Start);
            (document.text(), anchor)
        };
        self.applying.set(true);
        self.buffer.set_text(&text);
        self.applying.set(false);
        self.insert_anchor.set(anchor);
        // Bufor przerysowany w całości nie ma już zaznaczenia, a zapamiętane
        // kotwice wskazują stan sprzed cofnięcia.
        self.disarm();
    }

    fn mirror_keyboard_insert(&self, offset: usize, text: &str) {
        if self.applying.get() {
            return;
        }
        let mut document = self.document.borrow_mut();
        let Some(at) = document.anchor_at_char(offset) else {
            document.set_text(Anchor::Start, Anchor::Start, text);
            return;
        };
        document.set_text(at, at, text);
    }

    fn mirror_keyboard_delete(&self, start: usize, end: usize) {
        if self.applying.get() {
            return;
        }
        let mut document = self.document.borrow_mut();
        let (Some(from), Some(to)) = (document.anchor_at_char(start), document.anchor_at_char(end))
        else {
            return;
        };
        document.set_text(from, to, "");
    }

    /// Reguły 3 i 5 z `docs/07`: karetka przesunięta przez użytkownika
    /// przenosi punkt wstawiania, ale niepuste zaznaczenie go **nie rusza** —
    /// zaległe wypowiedzi mają dokąd trafić, gdy zaznaczenie czeka na
    /// zastąpienie.
    fn follow_caret(&self) {
        if self.applying.get() || self.buffer.has_selection() {
            return;
        }
        let offset = self.buffer.cursor_position() as usize;
        let document = self.document.borrow();
        if let Some(anchor) = document.anchor_at_char(offset) {
            self.insert_anchor.set(anchor);
        }
    }

    /// Czy punkt wstawiania był widoczny przed zmianą. Użytkownik, który
    /// przewinął w górę, żeby coś przeczytać, nie zostaje z tego wyrwany
    /// (`dictation.follow_caret`).
    fn insertion_point_visible(&self, offset: usize) -> bool {
        let iter = self.buffer.iter_at_offset(offset as i32);
        let location = self.view.iter_location(&iter);
        let visible = self.view.visible_rect();
        location.y() + location.height() >= visible.y()
            && location.y() <= visible.y() + visible.height()
    }
}

/// Chwila na osi czasu dyktowania, zapamiętana po to, żeby rozstrzygnąć,
/// które wypowiedzi padły **po** niej.
///
/// Sesja audio liczy czas od zera przy każdym nagraniu, więc sama liczba
/// milisekund nic nie znaczy bez numeru sesji — stąd oba pola razem.
#[derive(Debug, Clone, Copy)]
pub struct Mark {
    epoch: u64,
    at_ms: u64,
}

impl Mark {
    /// Znacznik złożony z gotowych liczb — wyłącznie do testów
    /// rozgałęziania wypowiedzi w [`crate::redact`], które nie mają jak
    /// uruchomić sesji audio.
    #[cfg(test)]
    pub fn at(epoch: u64, at_ms: u64) -> Self {
        Self { epoch, at_ms }
    }
}

/// Czy wypowiedź zaczynająca się w `audio_start_ms` padła po znaczniku.
///
/// Ta sama reguła, co przy zastępowaniu zaznaczenia (`docs/07`), użyta do
/// czego innego: rozstrzyga, czy wypowiedź jest poleceniem redakcyjnym, czy
/// zaległą treścią notatki, która wyszła z transkrypcji dopiero teraz.
fn spoken_after(mark: Mark, session_epoch: u64, audio_start_ms: u64) -> bool {
    replaces(mark.epoch, mark.at_ms, session_epoch, audio_start_ms)
}

/// Czy wypowiedź zaczynająca się w `audio_start_ms` zastępuje zaznaczenie
/// uzbrojone w `armed_at_ms` — reguła rozstrzygania z `docs/07`.
///
/// Zaznaczenie z wcześniejszej sesji audio zastępuje zawsze: nowa sesja liczy
/// oś czasu od zera, więc każda jej wypowiedź padła po uzbrojeniu, choćby jej
/// `audio_start_ms` był mniejszy od zapamiętanego.
fn replaces(armed_epoch: u64, armed_at_ms: u64, session_epoch: u64, audio_start_ms: u64) -> bool {
    let threshold = if armed_epoch < session_epoch {
        0
    } else {
        armed_at_ms
    };
    audio_start_ms >= threshold
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wypowiedz_po_uzbrojeniu_zastepuje_zaznaczenie() {
        assert!(replaces(1, 5_000, 1, 5_200));
    }

    #[test]
    fn zaleglosc_sprzed_uzbrojenia_nie_zjada_zaznaczenia() {
        // Sedno reguły: segment padł dziesięć sekund temu, a dotarł dopiero
        // teraz. Prototyp zjadał nim zaznaczenie — my go wstawiamy.
        assert!(!replaces(1, 12_000, 1, 2_000));
    }

    #[test]
    fn wypowiedz_dokladnie_w_momencie_uzbrojenia_zastepuje() {
        // Granica należy do zastępowania: mowa zaczęta w tej samej chwili
        // jest już poprawką, a nie zaległością.
        assert!(replaces(1, 5_000, 1, 5_000));
    }

    #[test]
    fn wypowiedz_sprzed_otwarcia_okna_polecenia_nie_jest_poleceniem() {
        // Ta sama reguła co przy zastępowaniu, użyta do czego innego:
        // zaległość, która padła przed otwarciem okna, jest treścią notatki,
        // a nie poleceniem redakcyjnym.
        let znacznik = Mark {
            epoch: 1,
            at_ms: 4_000,
        };
        assert!(
            !spoken_after(znacznik, 1, 1_200),
            "zaległość idzie do notatki"
        );
        assert!(
            spoken_after(znacznik, 1, 4_500),
            "wypowiedź po otwarciu to polecenie"
        );
    }

    #[test]
    fn zaznaczenie_sprzed_nowego_nagrania_zastepuje_pierwsza_wypowiedzia() {
        // Użytkownik zaznaczył tekst po zatrzymaniu nagrywania i zaczął mówić
        // od nowa. Nowa sesja liczy oś czasu od zera — zapamiętane 12 s nie
        // ma z nią nic wspólnego.
        assert!(replaces(1, 12_000, 2, 0));
    }

    // --- sklejka z buforem ---
    //
    // Reguła rozstrzygania i sama operacja `Splice` mają testy bez GTK, ale
    // nie pokrywają tego, co jest między nimi: czy zaznaczenie zrobione
    // w buforze uzbraja zastąpienie i czy zaległa wypowiedź nie zrywa go przy
    // okazji przesunięcia karetki. Błąd siedział dokładnie w tej warstwie,
    // więc ma własne testy — wymagają sesji graficznej, stąd `#[ignore]`:
    // `cargo test -p diktate-gtk -- --ignored`.

    fn segment(seq: u64, text: &str, audio_start_ms: u64) -> SegmentAdded {
        SegmentAdded {
            seq,
            text: text.to_string(),
            audio_start_ms,
            audio_end_ms: audio_start_ms + 1_000,
        }
    }

    fn przygotuj() -> Rc<DocumentView> {
        let view = TextView::new();
        DocumentView::attach(&view, DictationConfig::default())
    }

    /// Zaznacza fragment, upewniając się wpierw, że oś czasu audio zdążyła
    /// ruszyć. `replace_armed_at` bierze się z realnego zegara, a bez tej
    /// zwłoki zaległość z `audio_start_ms = 0` wypadałaby „po uzbrojeniu"
    /// i test mierzyłby szybkość maszyny, nie regułę.
    fn zaznacz(document: &DocumentView, od: i32, do_: i32) {
        std::thread::sleep(std::time::Duration::from_millis(30));
        let buffer = &document.buffer;
        buffer.select_range(&buffer.iter_at_offset(do_), &buffer.iter_at_offset(od));
    }

    fn tekst(document: &DocumentView) -> String {
        let buffer = &document.buffer;
        buffer
            .text(&buffer.start_iter(), &buffer.end_iter(), false)
            .to_string()
    }

    /// Trzy scenariusze w jednym teście, bo GTK wolno zainicjować tylko raz
    /// i tylko z jednego wątku, a harness dla każdego testu tworzy nowy.
    #[test]
    #[ignore = "wymaga sesji graficznej"]
    fn zastepowanie_zaznaczenia_w_buforze() {
        gtk4::init().expect("sesja graficzna");

        // Zaznaczenie uzbraja zastąpienie, wypowiedź je wymienia.
        let document = przygotuj();
        document.begin_audio_session();
        document.insert_segment(&segment(0, "Ala ma kota", 0));
        assert_eq!(tekst(&document), "Ala ma kota");
        zaznacz(&document, 7, 11);
        assert!(document.is_armed(), "zaznaczenie ma uzbrajać zastąpienie");
        document.insert_segment(&segment(1, "psa", 5_000));
        assert_eq!(tekst(&document), "Ala ma psa");
        assert!(
            !document.is_armed(),
            "po zastąpieniu tryb wraca do wstawiania"
        );

        // Zaległość sprzed uzbrojenia dopisuje się i zostawia zaznaczenie.
        let document = przygotuj();
        document.begin_audio_session();
        document.insert_segment(&segment(0, "Ala ma kota", 0));
        zaznacz(&document, 7, 11);
        document.insert_segment(&segment(1, "i psa", 0));
        assert_eq!(tekst(&document), "Ala ma kota i psa");
        assert!(
            document.is_armed(),
            "zaległość nie ma prawa unieważnić zaznaczenia"
        );
        document.insert_segment(&segment(2, "chomika", 5_000));
        assert_eq!(tekst(&document), "Ala ma chomika i psa");

        // Escape znosi uzbrojenie, dokument zostaje nietknięty.
        let document = przygotuj();
        document.begin_audio_session();
        document.insert_segment(&segment(0, "Ala ma kota", 0));
        zaznacz(&document, 7, 11);
        assert!(document.cancel_replace());
        assert!(!document.is_armed());
        assert_eq!(tekst(&document), "Ala ma kota");

        // Korekta LLM-em podmienia całość, a jedno cofnięcie przywraca tekst
        // sprzed niej — nie zostawia po drodze pustej notatki (`docs/11`).
        let document = przygotuj();
        document.begin_audio_session();
        document.insert_segment(&segment(0, "ala ma kota", 0));
        document.insert_segment(&segment(1, "i psa", 1_000));
        let przed = tekst(&document);
        document.replace_all("Ala ma kota i psa.");
        assert_eq!(tekst(&document), "Ala ma kota i psa.");
        assert!(document.undo(), "korekta ma być odwracalna");
        assert_eq!(tekst(&document), przed, "jedno cofnięcie wystarcza");

        // Redakcja poleceniem: celem jest zaznaczenie, a podmiana wchodzi
        // jedną operacją logu — tak samo odwracalną jak korekta.
        let document = przygotuj();
        document.begin_audio_session();
        document.insert_segment(&segment(0, "Ala ma kota i psa", 0));
        zaznacz(&document, 7, 11);
        let cel = document.target();
        assert_eq!(cel.text, "kota");
        assert!(!cel.whole, "zaznaczenie nie jest całą notatką");
        assert!(document.replace_range(&cel, "kotkę"));
        assert_eq!(tekst(&document), "Ala ma kotkę i psa");
        assert!(document.undo(), "redakcja ma być odwracalna");
        assert_eq!(tekst(&document), "Ala ma kota i psa");

        // Bez zaznaczenia celem jest cała notatka.
        let document = przygotuj();
        document.begin_audio_session();
        document.insert_segment(&segment(0, "Ala ma kota", 0));
        let cel = document.target();
        assert!(cel.whole, "brak zaznaczenia to cała notatka");
        assert_eq!(cel.text, "Ala ma kota");

        // Cel, który zmienił się w trakcie czekania na model, nie jest
        // podmieniany — wypowiedź z transkrypcji zostałaby zjedzona bez śladu.
        let document = przygotuj();
        document.begin_audio_session();
        document.insert_segment(&segment(0, "Ala ma kota", 0));
        zaznacz(&document, 7, 11);
        let cel = document.target();
        document.insert_segment(&segment(1, "i psa", 5_000));
        assert!(
            !document.replace_range(&cel, "kotkę"),
            "zmieniony cel ma zatrzymać redakcję"
        );

        // Dyktowanie po korekcie płynie dalej od końca poprawionego tekstu.
        let document = przygotuj();
        document.begin_audio_session();
        document.insert_segment(&segment(0, "ala ma kota", 0));
        document.replace_all("Ala ma kota.");
        document.insert_segment(&segment(1, "I psa.", 1_000));
        assert_eq!(tekst(&document), "Ala ma kota. I psa.");
    }
}
