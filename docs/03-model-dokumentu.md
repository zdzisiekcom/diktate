# 03 — Model dokumentu

Dotyczy crate'u `diktate-document`, używanego przez `diktate-gtk` i
`diktate-cli`. Daemon nie zna tego modelu i nie ma do niego dostępu.

## Problem

Wymaganie brzmi: w trakcie dyktowania, gdy pojawi się segment, który nie
odpowiada użytkownikowi, musi być możliwe zaznaczenie fragmentu tekstu
i podyktowanie go od nowa — tak, żeby nowy tekst zastąpił zaznaczony fragment.
Ma to działać spójnie także wtedy, gdy w tle wciąż napływają kolejne segmenty.

## Dwie warstwy

### Warstwa 1 — log transkrypcji

Append-only, nigdy nie modyfikowany. Każdy segment ma `seq`, zakres czasowy
w nagraniu i surowy tekst z whispera. To zapis tego, co faktycznie padło.
Poprawki go nie dotykają.

### Warstwa 2 — dokument

Uporządkowana lista **spanów**. Span wskazuje na fragment segmentu:

```rust
struct Span {
    id: SpanId,          // stabilne, nigdy nie reużywane
    source: Source,      // z czego pochodzi tekst — niżej
    range: Range<usize>, // zakres znaków w tekście źródła
}
```

Tekst widoczny w oknie to konkatenacja spanów. Nowy segment = jeden span
wstawiony **w punkcie wstawiania** (zwykle, ale nie zawsze, na końcu listy —
patrz [07](07-zachowanie-dyktowania.md#punkt-wstawiania)).

Nie każdy span pochodzi z wypowiedzi: spacja rozdzielająca dokładana przy
sklejaniu ([07](07-zachowanie-dyktowania.md#sklejanie-z-otoczeniem)) i tekst
wprowadzony klawiaturą nie mają `segment_seq`. Dlatego span wskazuje na
**źródło**, którym jest albo wypowiedź, albo tekst dosłowny z magazynu
dokumentu:

```rust
enum Source {
    Segment(u64),   // seq wypowiedzi z logu transkrypcji
    Literal(usize), // tekst dosłowny: spacja sklejająca, wpis z klawiatury
}
```

Oba magazyny są append-only — cofnięcie operacji nie kasuje tego, co
faktycznie padło ani co zostało napisane.

Edycja **nie zmienia segmentów** — zmienia listę spanów. Zastąpienie
zaznaczenia to operacja `splice`: wytnij spany pokrywające zaznaczenie (tnąc
skrajne na pół), wstaw w to miejsce span wskazujący na nowo podyktowany
segment.

## Operacje

Operacje trafiają do własnego append-only logu, więc:

```
dokument = fold(operacje) na pustej liście spanów
```

```rust
enum Op {
    Insert   { at: Anchor, spans: Vec<Span> },
    Splice   { from: Anchor, to: Anchor, replacement: Vec<Span> },
    Delete   { from: Anchor, to: Anchor },
    SetText  { from: Anchor, to: Anchor, text: String }, // edycja klawiaturą
}
```

Nie ma osobnego `Append` — dopisanie na koniec to `Insert` z kotwicą na końcu
dokumentu. Jedna ścieżka zamiast dwóch oznacza, że wstawianie w środek notatki
nie jest przypadkiem szczególnym, którego nikt nie przetestował.

`spans` jest listą, bo normalizacja tekstu segmentu
([07](07-zachowanie-dyktowania.md#sklejanie-z-otoczeniem)) potrafi dołożyć do
wstawianego tekstu spację rozdzielającą, a usunięcie końcowego wielokropka
przycina span poprzedzający. Jedno wstawienie z punktu widzenia użytkownika
jest **jedną** operacją w logu — inaczej `Ctrl+Z` cofałby dodaną spację
osobno od tekstu, do którego ona należy.

Undo to zdjęcie ostatniej operacji i ponowne złożenie dokumentu — dostajemy je
za darmo, nie implementujemy osobno. Redo to odłożony stos zdjętych operacji.

## Kotwice — dlaczego to jest spójne

Naiwna implementacja wysyłałaby „zamień znaki 340–378". To się psuje
dokładnie w scenariuszu docelowym: użytkownik zaznacza tekst, w tym momencie
whisper kończy zaległy segment i dopisuje 60 znaków, a zanim polecenie zostanie
wykonane — offsety znaczą już co innego. Przy zaległościach w kolejce (mowa
szybsza niż przepustowość GPU) to nie jest przypadek brzegowy, tylko norma.

Dlatego **pozycje nigdy nie są wyrażane globalnymi offsetami znaków**:

```rust
enum Anchor {
    Start,                              // początek dokumentu
    In { span: SpanId, offset: usize }, // offset znakowy WEWNĄTRZ spanu
}
```

Wariant `Start` istnieje, bo w pustej notatce nie ma jeszcze żadnego spanu,
na którym można by zaczepić kotwicę.

`SpanId` jest stabilny i nigdy nie jest reużywany. Dopisanie segmentu na końcu
dokumentu tworzy nowy span i **nie dotyka żadnej istniejącej kotwicy**.

Wstawienie w środek dokumentu ([07](07-zachowanie-dyktowania.md#punkt-wstawiania))
trafia albo na granicę spanów — i wtedy również nie dotyka niczego — albo do
wnętrza spanu, który trzeba przeciąć. Cięcie ma jedną regułę:

> **Lewa połówka zachowuje `SpanId`, prawa dostaje nowy.**

Kotwice o offsecie mniejszym od punktu cięcia pozostają dosłownie takie same.
Kotwice za punktem cięcia są przeliczane przy odczycie na `(nowy id, offset −
punkt cięcia)` — `fold` zostawia po każdym cięciu ślad `stary id → (id prawej
połówki, punkt cięcia)`, więc kotwica trzymana przez widok od dawna wskazuje
wciąż ten sam znak zamiast stać się nieważna. To przekształcenie
deterministyczne i lokalne, wykonywane na spanie, którego zaznaczenie i tak nie obejmuje (punkt wstawiania leży poza
zaznaczeniem, bo przy uzbrojonym zastąpieniu kotwica wstawiania się nie
porusza).

Z tego wynika gwarancja, o którą w tym wszystkim chodzi: równoległy napływ
segmentów nigdy nie unieważnia trwającego zaznaczenia.

Kotwica może stać się nieważna tylko wtedy, gdy inna *edycja* usunęła span, na
który wskazuje. Przy jednym oknie klienta to nie zachodzi. Przy wielu klientach
operacja jest odrzucana, a klient przelicza dokument od nowa.

Konwersja w obie strony (offset w `GtkTextBuffer` ↔ kotwica) jest
odpowiedzialnością warstwy widoku i jest jedynym miejscem, gdzie globalne
offsety w ogóle występują.

## Zastępowanie przez ponowne podyktowanie

### Przebieg

1. Użytkownik zaznacza fragment w oknie. **Samo zaznaczenie uzbraja
   zastąpienie** — nie ma osobnego trybu do włączenia. `Ctrl+R` (albo przycisk
   „Podyktuj ponownie") jest równoważnym uzbrojeniem jawnym, potrzebnym przy
   `gtk.dictation.implicit_replace = false` i w trybie `until_confirm`.
2. Klient zapamiętuje kotwice zaznaczenia oraz `replace_armed_at` — moment
   w osi czasu audio sesji, w którym zaznaczenie powstało. Kotwica wstawiania
   zostaje tam, gdzie była.
3. Okno wyróżnia zaznaczenie i pokazuje etykietę „zastępuję…".
4. Użytkownik mówi. VAD zamyka segment po ciszy, whisper go transkrybuje,
   daemon rozgłasza `segment.added`.
5. Klient rozstrzyga na podstawie `audio_start_ms`:
   - `audio_start_ms < replace_armed_at` → **`Insert`** w kotwicę wstawiania
     (to resztka poprzedniej mowy, która czekała w kolejce); zaznaczenie
     pozostaje uzbrojone
   - `audio_start_ms >= replace_armed_at` → **`Splice`** w zapamiętane kotwice
6. Po wykonaniu `Splice` zaznaczenie znika, kotwica wstawiania ląduje za nowym
   tekstem, klient wraca do zwykłego wstawiania — i dyktowanie płynie dalej od
   poprawionego miejsca.

Krok 5 jest istotą poprawności. Granicę wyznacza **timestamp audio, nie
kolejność przybycia** — dzięki temu zaległości w kolejce nie powodują, że
zastąpieniem stanie się zdanie wypowiedziane sekundę wcześniej. Prototyp
`gadajj` tego nie rozróżniał i właśnie tam się psuł
([07](07-zachowanie-dyktowania.md#rozstrzyganie-po-osi-czasu-audio)).

Normalizacja brzegów (rozszerzenie zaznaczenia do granicy słowa, spacje,
interpunkcja za zaznaczeniem) jest opisana w
[07](07-zachowanie-dyktowania.md#granice-zaznaczenia) i wchodzi w skład tej
samej operacji `Splice`.

### Reguła zakończenia

Jedno zastąpienie zjada **dokładnie jedną wypowiedź** — jeden segment zamknięty
ciszą. Rozważana była alternatywa „aż użytkownik naciśnie Enter", ale wymaga
sięgania po klawiaturę w trakcie mówienia i psuje rytm dyktowania. Jeśli
poprawka nie mieści się w jednej wypowiedzi, użytkownik zaznacza nową wersję
i dyktuje ponownie.

`Escape` znosi zaznaczenie i anuluje zastępowanie bez żadnych zmian
w dokumencie.

Parametr konfiguracyjny `gtk.replace.mode = "one_utterance" | "until_confirm"`
zostawia furtkę, gdyby praktyka pokazała coś innego.

### Co z audio zastąpionego fragmentu

Nic mu się nie dzieje. Log transkrypcji zawiera oba segmenty — zastąpiony
i zastępujący. Dokument pokazuje tylko ten drugi. Pełna proweniencja jest
zachowana i odwracalna.

## Historia

Zapisywana przez klienta do `$XDG_DATA_HOME/diktate/notes/<data>-<id>/`:

| Plik | Zawartość |
|------|-----------|
| `raw.ndjson` | Log transkrypcji — segmenty tak, jak przyszły z daemona |
| `ops.ndjson` | Log operacji na dokumencie |
| `note.md` | Tekst wynikowy, materializowany dla wygody |

`note.md` jest **pochodną**, nie źródłem prawdy — da się go odtworzyć z dwóch
pozostałych plików. Istnieje po to, żeby notatki dało się przeszukiwać
`grep`em i otwierać w dowolnym edytorze, bez narzędzi projektu.

## Punkt zaczepienia dla LLM-a

Redakcja tekstu lokalnym LLM-em (etap poza pierwszą wersją) wpina się tutaj
bez zmian w modelu: LLM produkuje kolejne operacje `Splice`/`SetText`, widoczne
i odwracalne dokładnie tak samo, jak poprawki użytkownika. `Ctrl+Z` cofa
redakcję LLM-a tym samym mechanizmem, co wszystko inne.

## Testy

`diktate-document` jest testowalny w całości bez audio, bez daemona i bez GTK
— to zwykła biblioteka nad strukturami danych. Przypadki obowiązkowe:

- wstawienie na końcu w trakcie aktywnego zaznaczenia nie przesuwa kotwic
- wstawienie w środek spanu tnie go tak, że lewa połówka zachowuje `SpanId`,
  a kotwica za punktem cięcia wskazuje ten sam znak co przed operacją
- splice przez granicę dwóch spanów tnie skrajne poprawnie
- splice obejmujący całe spany usuwa je w całości
- segment z `audio_start_ms` sprzed uzbrojenia trafia w kotwicę wstawiania,
  a zaznaczenie zostaje uzbrojone dla następnego segmentu
- normalizacja (spacja rozdzielająca, usunięty wielokropek) jest częścią jednej
  operacji — jedno `Ctrl+Z` cofa wstawienie razem z nią
- undo po splice odtwarza dokument znak w znak
- sekwencja losowych operacji: `fold(ops)` daje ten sam wynik przy każdym
  przeliczeniu (test własnościowy)
