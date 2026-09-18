//! Model dokumentu: spany, kotwice i log operacji.
//!
//! Zasada, z której wynika cała reszta: **pozycje nigdy nie są wyrażane
//! globalnymi offsetami znaków**. Naiwne „zamień znaki 340–378" psuje się
//! dokładnie w scenariuszu, dla którego ten projekt powstał — użytkownik
//! zaznacza fragment, w tym momencie kończy się zaległy segment i dopisuje
//! sześćdziesiąt znaków, a zanim polecenie zostanie wykonane, offsety znaczą
//! już co innego. Przy mowie szybszej niż przepustowość GPU to nie jest
//! przypadek brzegowy, tylko norma.
//!
//! Dlatego dokument jest listą [`Span`]-ów wskazujących na fragmenty źródeł
//! (wypowiedzi albo tekstu dosłownego), a pozycje to [`Anchor`]-y: `(id
//! spanu, offset wewnątrz spanu)`. Dopisanie na końcu tworzy nowy span i nie
//! dotyka żadnej istniejącej kotwicy. Cięcie spanu ma jedną regułę: **lewa
//! połówka zachowuje `SpanId`, prawa dostaje nowy** — więc kotwice przed
//! punktem cięcia zostają dosłownie takie same.
//!
//! Dokument nie jest przechowywany, tylko **składany z logu operacji**:
//! `dokument = fold(operacje)`. Stąd cofanie dostajemy za darmo — to zdjęcie
//! ostatniej operacji i ponowne złożenie. Identyfikatory spanów nadaje fold,
//! więc powtórzone złożenie tego samego logu daje dosłownie ten sam
//! dokument, z tymi samymi kotwicami.
//!
//! Pełne uzasadnienie: `docs/03-model-dokumentu.md`,
//! reguły sklejania tekstu: `docs/07-zachowanie-dyktowania.md`.

use std::collections::{BTreeMap, HashMap};
use std::ops::Range;

/// Stabilny identyfikator spanu — nigdy nie jest reużywany w obrębie jednego
/// złożenia dokumentu.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SpanId(u64);

/// Skąd pochodzi tekst spanu.
///
/// `docs/03` opisuje spany wskazujące na segmenty. Do tego dochodzi tekst
/// dosłowny: spacja rozdzielająca dokładana przy sklejaniu i tekst
/// wprowadzony klawiaturą. Oba nie pochodzą z żadnej wypowiedzi, a muszą
/// leżeć w tej samej liście spanów, żeby cofanie działało jednym
/// mechanizmem dla wszystkiego.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// Wypowiedź o tym `seq` z logu transkrypcji.
    Segment(u64),
    /// Tekst dosłowny o tym numerze w magazynie dokumentu.
    Literal(usize),
}

/// Fragment źródła widoczny w dokumencie. Tekst dokumentu to konkatenacja
/// spanów w kolejności listy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    pub id: SpanId,
    pub source: Source,
    /// Zakres **znakowy** (nie bajtowy) w tekście źródła.
    pub range: Range<usize>,
}

impl Span {
    fn len(&self) -> usize {
        self.range.end - self.range.start
    }
}

/// Pozycja w dokumencie odporna na napływ nowych wypowiedzi.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Anchor {
    /// Początek dokumentu — jedyna pozycja, jaka istnieje w pustej notatce.
    Start,
    /// `offset` znaków od początku spanu `span`.
    In { span: SpanId, offset: usize },
}

impl Anchor {
    /// Span, w którym leży kotwica; `None` dla początku dokumentu.
    pub fn span_id(&self) -> Option<SpanId> {
        match self {
            Anchor::Start => None,
            Anchor::In { span, .. } => Some(*span),
        }
    }
}

/// Co warstwa widoku ma zrobić z buforem, żeby dogonić model.
///
/// Widok nie przerysowuje całego bufora przy każdej wypowiedzi — zgubiłby
/// przy tym zaznaczenie, pozycję karetki i przewinięcie. Dostaje więc opis
/// jednej, lokalnej zmiany. Globalne offsety znakowe występują wyłącznie
/// tutaj, na granicy z widokiem (`docs/03`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    /// Pozycja znakowa, od której zaczyna się zmiana.
    pub at: usize,
    /// Ile znaków usunąć od `at` w lewo (obcięty wielokropek).
    pub removed: usize,
    /// Co wstawić w miejsce `at` po usunięciu.
    pub inserted: String,
    /// Nowa kotwica wstawiania — tuż za wstawionym tekstem.
    pub anchor: Anchor,
}

/// Reguły sklejania wstawianego tekstu z otoczeniem (`docs/07`).
#[derive(Debug, Clone, Copy)]
pub struct Normalization {
    /// Spacja rozdzielająca na stykach wypowiedzi.
    pub auto_space: bool,
    /// Usuwanie wielokropka kończącego wypowiedź przerwaną pauzą na oddech.
    pub strip_trailing_ellipsis: bool,
}

impl Default for Normalization {
    fn default() -> Self {
        Self {
            auto_space: true,
            strip_trailing_ellipsis: true,
        }
    }
}

/// Znaki, przed którymi nigdy nie stawiamy spacji rozdzielającej.
const NO_SPACE_BEFORE: &[char] = &[',', '.', '!', '?', ':', ';', ')', '”', '»', '…'];
/// Znaki, po których nigdy nie stawiamy spacji rozdzielającej.
const NO_SPACE_AFTER: &[char] = &['(', '„', '«', '“', '\u{201e}'];

/// Zawartość jednego spanu przed nadaniem mu identyfikatora. Identyfikatory
/// nadaje fold, żeby powtórzone złożenie logu dawało ten sam dokument.
#[derive(Debug, Clone, PartialEq, Eq)]
struct SpanContent {
    source: Source,
    range: Range<usize>,
}

/// Operacja w logu. Dokument to `fold` tych operacji na pustej liście spanów.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Op {
    /// Wstawienie w kotwicę. `trim_before` to liczba znaków usuwanych tuż
    /// przed punktem wstawienia (końcowy wielokropek) — jest częścią tej
    /// samej operacji, żeby jedno cofnięcie zdejmowało wstawienie razem
    /// z normalizacją.
    Insert {
        at: Anchor,
        trim_before: usize,
        content: Vec<SpanContent>,
    },
    /// Edycja klawiaturą: zakres zastąpiony tekstem dosłownym.
    SetText {
        from: Anchor,
        to: Anchor,
        literal: usize,
    },
}

/// Notatka: log operacji plus złożony z niego stan.
pub struct Document {
    /// Log operacji — jedyne źródło prawdy.
    ops: Vec<Op>,
    /// Operacje zdjęte przez cofnięcie, czekające na ponowienie.
    redo: Vec<Op>,
    /// Stan złożony z `ops`.
    spans: Vec<Span>,
    next_span_id: u64,
    /// Ślad po cięciach spanów: `stary id -> (id prawej połówki, punkt
    /// cięcia)`. Kotwica o offsecie większym niż skrócony span nie staje się
    /// przez to nieważna — jest przeliczana na `(nowy id, offset − punkt
    /// cięcia)`, czyli wskazuje dokładnie ten sam znak co przed cięciem.
    /// To właśnie ta reguła sprawia, że napływ nowych wypowiedzi nigdy nie
    /// unieważnia trwającego zaznaczenia.
    redirects: HashMap<SpanId, (SpanId, usize)>,
    /// Log transkrypcji: wypowiedzi po `seq`. Append-only — cofnięcie
    /// operacji nie kasuje tego, co faktycznie padło.
    segments: BTreeMap<u64, Vec<char>>,
    /// Magazyn tekstu dosłownego. Też append-only, z tego samego powodu.
    literals: Vec<Vec<char>>,
}

impl Default for Document {
    fn default() -> Self {
        Self::new()
    }
}

impl Document {
    pub fn new() -> Self {
        Self {
            ops: Vec::new(),
            redo: Vec::new(),
            spans: Vec::new(),
            next_span_id: 0,
            redirects: HashMap::new(),
            segments: BTreeMap::new(),
            literals: Vec::new(),
        }
    }

    // --- odczyt ---

    /// Tekst widoczny w oknie: konkatenacja spanów.
    pub fn text(&self) -> String {
        let mut out = String::new();
        for span in &self.spans {
            out.extend(self.span_chars(span));
        }
        out
    }

    /// Długość dokumentu w znakach.
    pub fn char_len(&self) -> usize {
        self.spans.iter().map(Span::len).sum()
    }

    pub fn span_count(&self) -> usize {
        self.spans.len()
    }

    pub fn spans(&self) -> &[Span] {
        &self.spans
    }

    /// Kotwica na `n`-tym znaku dokumentu. `None`, gdy dokument jest krótszy
    /// (w pustym dokumencie nie ma żadnej pozycji poza [`Anchor::Start`]).
    pub fn anchor_at_char(&self, n: usize) -> Option<Anchor> {
        let mut passed = 0;
        for span in &self.spans {
            if n < passed + span.len() {
                return Some(Anchor::In {
                    span: span.id,
                    offset: n - passed,
                });
            }
            passed += span.len();
        }
        // Dokładnie koniec dokumentu — kotwica za ostatnim znakiem.
        if n == passed {
            return self.spans.last().map(|span| Anchor::In {
                span: span.id,
                offset: span.len(),
            });
        }
        None
    }

    /// Kotwica na końcu dokumentu; dla pustej notatki — na jej początku.
    pub fn end_anchor(&self) -> Anchor {
        match self.spans.last() {
            Some(span) => Anchor::In {
                span: span.id,
                offset: span.len(),
            },
            None => Anchor::Start,
        }
    }

    /// Pozycja znakowa kotwicy — jedyne miejsce, w którym globalny offset
    /// wychodzi na zewnątrz modelu. Warstwa widoku potrzebuje go, bo
    /// `GtkTextBuffer` innych pozycji nie zna.
    pub fn char_position(&self, anchor: Anchor) -> usize {
        self.position_of(anchor)
    }

    /// Znak stojący w tym miejscu; `None` na końcu dokumentu.
    pub fn char_at(&self, anchor: Anchor) -> Option<char> {
        let position = self.position_of(anchor);
        self.chars().nth(position)
    }

    /// Tekst między dwiema kotwicami, niezależnie od ich kolejności.
    pub fn text_between(&self, a: Anchor, b: Anchor) -> String {
        let (from, to) = {
            let (x, y) = (self.position_of(a), self.position_of(b));
            (x.min(y), x.max(y))
        };
        self.chars().skip(from).take(to - from).collect()
    }

    // --- zapis ---

    /// Wstawia wypowiedź `seq` w kotwicę, stosując normalizację sklejania
    /// jako **część tej samej operacji**. Zwraca nową kotwicę wstawiania —
    /// tuż za wstawionym tekstem (przed ewentualną spacją dołożoną z tyłu,
    /// bo dalszy ciąg dyktowania należy tam, gdzie skończyła się mowa).
    pub fn insert_segment(
        &mut self,
        seq: u64,
        text: &str,
        at: Anchor,
        normalization: &Normalization,
    ) -> Change {
        let chars: Vec<char> = text.chars().collect();
        let len = chars.len();
        self.segments.insert(seq, chars);

        let trim_before = self.ellipsis_to_trim(at, normalization);
        let position = self.position_of(at);
        let mut inserted = String::new();
        let mut content = Vec::new();
        let mut segment_index = 0;

        if self.needs_space_before(at, trim_before, text, normalization) {
            content.push(self.literal_content(" "));
            inserted.push(' ');
            segment_index = 1;
        }
        content.push(SpanContent {
            source: Source::Segment(seq),
            range: 0..len,
        });
        inserted.push_str(text);
        if self.needs_space_after(at, text, normalization) {
            content.push(self.literal_content(" "));
            inserted.push(' ');
        }

        let created = self.push_op(Op::Insert {
            at,
            trim_before,
            content,
        });
        let anchor = match created.get(segment_index) {
            Some(&id) => Anchor::In {
                span: id,
                offset: len,
            },
            // Pusty segment nie utworzył spanu — kotwica zostaje tam, gdzie była.
            None => at,
        };
        Change {
            at: position - trim_before,
            removed: trim_before,
            inserted,
            anchor,
        }
    }

    /// Edycja klawiaturą: zastępuje zakres tekstem. Trafia do tego samego
    /// logu co dyktowanie, więc `Ctrl+Z` cofa jedno i drugie tym samym
    /// mechanizmem.
    pub fn set_text(&mut self, from: Anchor, to: Anchor, text: &str) {
        let literal = self.push_literal(text);
        self.push_op(Op::SetText { from, to, literal });
    }

    /// Cofa ostatnią operację. `false`, gdy nie było czego cofać.
    pub fn undo(&mut self) -> bool {
        match self.ops.pop() {
            Some(op) => {
                self.redo.push(op);
                self.refold();
                true
            }
            None => false,
        }
    }

    /// Ponawia ostatnio cofniętą operację. `false`, gdy stos ponowień pusty.
    pub fn redo(&mut self) -> bool {
        match self.redo.pop() {
            Some(op) => {
                self.ops.push(op);
                self.refold();
                true
            }
            None => false,
        }
    }

    /// Składa dokument od nowa z logu operacji. Publiczne, bo to jedyna
    /// droga odzyskania spójności po odrzuceniu operacji — i bo na tym
    /// opiera się test własnościowy determinizmu.
    pub fn refold(&mut self) {
        self.spans.clear();
        self.next_span_id = 0;
        self.redirects.clear();
        let ops = std::mem::take(&mut self.ops);
        for op in &ops {
            self.apply(op);
        }
        self.ops = ops;
    }

    // --- wnętrze ---

    /// Dokłada operację do logu, stosuje ją i zwraca identyfikatory
    /// utworzonych spanów. Każda nowa operacja kasuje stos ponowień — po
    /// rozejściu się historii nie ma czego ponawiać.
    fn push_op(&mut self, op: Op) -> Vec<SpanId> {
        self.redo.clear();
        let created = self.apply(&op);
        self.ops.push(op);
        created
    }

    fn apply(&mut self, op: &Op) -> Vec<SpanId> {
        match op {
            Op::Insert {
                at,
                trim_before,
                content,
            } => {
                let mut index = self.split_at(*at);
                index = self.trim_before(index, *trim_before);
                self.insert_spans(index, content)
            }
            Op::SetText { from, to, literal } => {
                // Tniemy najpierw na końcu zakresu: cięcie na pozycji
                // późniejszej nie rusza niczego przed nią, więc kotwica
                // `from` pozostaje ważna. W odwrotnej kolejności `to`
                // wskazywałoby już na span, którego nie ma.
                let end = self.split_at(*to);
                let before = self.spans.len();
                let start = self.split_at(*from);
                // Cięcie na początku zakresu dokłada span przed jego końcem,
                // więc indeks końca trzeba o tyle samo przesunąć.
                let end = end + (self.spans.len() - before);
                let (start, end) = (start.min(end), start.max(end));
                self.spans.drain(start..end);
                let text_len = self.literals[*literal].len();
                if text_len == 0 {
                    return Vec::new();
                }
                self.insert_spans(
                    start,
                    &[SpanContent {
                        source: Source::Literal(*literal),
                        range: 0..text_len,
                    }],
                )
            }
        }
    }

    /// Zamienia kotwicę na indeks w liście spanów, tnąc span, jeśli kotwica
    /// wypada w jego środku. Lewa połówka zachowuje `SpanId`, prawa dostaje
    /// nowy — dzięki temu kotwice przed punktem cięcia zostają nietknięte.
    ///
    /// Kotwica wskazująca span, którego już nie ma (usunęła go wcześniejsza
    /// edycja), jest traktowana jako koniec dokumentu — to sytuacja
    /// niemożliwa przy jednym kliencie, a lepsza od paniki.
    fn split_at(&mut self, anchor: Anchor) -> usize {
        if matches!(anchor, Anchor::Start) {
            return 0;
        }
        let Some((index, offset)) = self.resolve(anchor) else {
            return self.spans.len();
        };
        let span_id = self.spans[index].id;
        let (source, range, len) = {
            let span = &self.spans[index];
            (span.source, span.range.clone(), span.len())
        };
        if offset == 0 {
            return index;
        }
        if offset >= len {
            return index + 1;
        }
        let cut = range.start + offset;
        let right = Span {
            id: self.take_span_id(),
            source,
            range: cut..range.end,
        };
        let right_id = right.id;
        self.spans[index].range.end = cut;
        self.spans.insert(index + 1, right);
        self.redirects.insert(span_id, (right_id, offset));
        index + 1
    }

    /// Usuwa `count` znaków tuż przed pozycją `index`, skracając spany od
    /// końca. Zwraca pozycję po skróceniu (spany, które zniknęły w całości,
    /// przesuwają indeks).
    fn trim_before(&mut self, mut index: usize, mut count: usize) -> usize {
        while count > 0 && index > 0 {
            let len = self.spans[index - 1].len();
            if len <= count {
                count -= len;
                self.spans.remove(index - 1);
                index -= 1;
            } else {
                self.spans[index - 1].range.end -= count;
                count = 0;
            }
        }
        index
    }

    fn insert_spans(&mut self, index: usize, content: &[SpanContent]) -> Vec<SpanId> {
        let mut created = Vec::with_capacity(content.len());
        for (offset, item) in content.iter().enumerate() {
            if item.range.is_empty() {
                continue;
            }
            let id = self.take_span_id();
            created.push(id);
            self.spans.insert(
                index + offset,
                Span {
                    id,
                    source: item.source,
                    range: item.range.clone(),
                },
            );
        }
        created
    }

    fn take_span_id(&mut self) -> SpanId {
        let id = SpanId(self.next_span_id);
        self.next_span_id += 1;
        id
    }

    fn push_literal(&mut self, text: &str) -> usize {
        self.literals.push(text.chars().collect());
        self.literals.len() - 1
    }

    fn literal_content(&mut self, text: &str) -> SpanContent {
        let index = self.push_literal(text);
        let len = self.literals[index].len();
        SpanContent {
            source: Source::Literal(index),
            range: 0..len,
        }
    }

    fn span_chars(&self, span: &Span) -> impl Iterator<Item = char> + '_ {
        let source: &[char] = match span.source {
            Source::Segment(seq) => self.segments.get(&seq).map(Vec::as_slice).unwrap_or(&[]),
            Source::Literal(index) => self.literals[index].as_slice(),
        };
        source[span.range.clone()].iter().copied()
    }

    fn chars(&self) -> impl Iterator<Item = char> + '_ {
        self.spans.iter().flat_map(|span| self.span_chars(span))
    }

    /// Zamienia kotwicę na `(indeks spanu, offset w nim)`, podążając za
    /// śladami cięć. `None`, gdy kotwica wskazuje początek dokumentu albo
    /// span, którego już nie ma.
    fn resolve(&self, anchor: Anchor) -> Option<(usize, usize)> {
        let (mut span_id, mut offset) = match anchor {
            Anchor::Start => return None,
            Anchor::In { span, offset } => (span, offset),
        };
        loop {
            let index = self.spans.iter().position(|span| span.id == span_id)?;
            let len = self.spans[index].len();
            if offset <= len {
                return Some((index, offset));
            }
            // Offset wypada za spanem — to znaczy, że span został przecięty
            // po powstaniu kotwicy. Przeliczamy na prawą połówkę.
            match self.redirects.get(&span_id) {
                Some(&(right, cut)) if offset >= cut => {
                    span_id = right;
                    offset -= cut;
                }
                _ => return Some((index, len)),
            }
        }
    }

    /// Globalna pozycja znakowa kotwicy. Używana wyłącznie wewnątrz
    /// odczytów — na zewnątrz globalne offsety nie istnieją.
    fn position_of(&self, anchor: Anchor) -> usize {
        let Some((index, offset)) = self.resolve(anchor) else {
            return match anchor {
                Anchor::Start => 0,
                // Kotwica po spanie, którego już nie ma — koniec dokumentu.
                Anchor::In { .. } => self.char_len(),
            };
        };
        self.spans[..index].iter().map(Span::len).sum::<usize>() + offset
    }

    /// Ile znaków wielokropka usunąć przed wstawieniem. Tylko na końcu
    /// dokumentu — w środku zdania wielokropek bywa prawdziwą interpunkcją.
    fn ellipsis_to_trim(&self, at: Anchor, normalization: &Normalization) -> usize {
        if !normalization.strip_trailing_ellipsis || self.position_of(at) != self.char_len() {
            return 0;
        }
        let text = self.text();
        if text.ends_with('…') {
            1
        } else if text.ends_with("...") {
            3
        } else {
            0
        }
    }

    fn needs_space_before(
        &self,
        at: Anchor,
        trim_before: usize,
        text: &str,
        normalization: &Normalization,
    ) -> bool {
        if !normalization.auto_space {
            return false;
        }
        if text.starts_with(NO_SPACE_BEFORE) {
            return false;
        }
        let position = self.position_of(at);
        if position < trim_before {
            return false;
        }
        // Znak sprzed punktu wstawienia liczony **po** obcięciu wielokropka —
        // inaczej decydowalibyśmy o spacji na podstawie tekstu, którego już
        // nie będzie.
        match self.chars().take(position - trim_before).last() {
            None => false,
            Some(c) if c.is_whitespace() => false,
            Some(c) if NO_SPACE_AFTER.contains(&c) => false,
            Some(_) => true,
        }
    }

    fn needs_space_after(&self, at: Anchor, text: &str, normalization: &Normalization) -> bool {
        if !normalization.auto_space || text.is_empty() {
            return false;
        }
        if text.ends_with(char::is_whitespace) {
            return false;
        }
        // Spacja z tyłu tylko przy wstawianiu w środek zdania — na końcu
        // dokumentu nie ma czego rozdzielać.
        matches!(self.char_at(at), Some(c) if c.is_alphanumeric())
    }
}
