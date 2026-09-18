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

use std::cell::{Cell, RefCell};
use std::rc::Rc;

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
    normalization: Normalization,
}

impl DocumentView {
    /// Podpina model pod widok tekstu i podłącza sygnały edycji.
    pub fn attach(view: &TextView, normalization: Normalization) -> Rc<Self> {
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
            normalization,
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

        this
    }

    /// Wstawia wypowiedź w kotwicę wstawiania i nakłada wynikłą zmianę na
    /// bufor.
    pub fn insert_segment(&self, segment: &SegmentAdded) {
        let change = self.document.borrow_mut().insert_segment(
            segment.seq,
            &segment.text,
            self.insert_anchor.get(),
            &self.normalization,
        );
        self.insert_anchor.set(change.anchor);

        let follow = self.insertion_point_visible(change.at);
        self.applying.set(true);
        let mut at = self.buffer.iter_at_offset(change.at as i32);
        if change.removed > 0 {
            let mut end = self
                .buffer
                .iter_at_offset((change.at + change.removed) as i32);
            self.buffer.delete(&mut at, &mut end);
        }
        self.buffer.insert(&mut at, &change.inserted);
        // Karetka idzie tam, gdzie stanęła kotwica — dyktowanie płynie dalej
        // od tego miejsca.
        let caret = self
            .buffer
            .iter_at_offset(self.document.borrow().char_position(change.anchor) as i32);
        self.buffer.place_cursor(&caret);
        self.applying.set(false);

        if follow {
            self.view
                .scroll_to_mark(&self.buffer.get_insert(), 0.0, false, 0.0, 0.0);
        }
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

    /// Cały tekst notatki — do schowka.
    pub fn text(&self) -> String {
        self.document.borrow().text()
    }

    // --- wnętrze ---

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
