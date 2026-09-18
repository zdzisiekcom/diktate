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
    segment_seq: u64,
    range: Range<usize>, // zakres znaków w tekście segmentu
}
```

Tekst widoczny w oknie to konkatenacja spanów. Nowy segment = jeden span
dopisany na koniec.

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
    Append   { span: Span },
    Splice   { from: Anchor, to: Anchor, replacement: Vec<Span> },
    Delete   { from: Anchor, to: Anchor },
    SetText  { from: Anchor, to: Anchor, text: String }, // edycja klawiaturą
}
```

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
struct Anchor {
    span: SpanId,
    offset: usize, // offset znakowy WEWNĄTRZ spanu
}
```

`SpanId` jest stabilny i nigdy nie jest reużywany. Dopisanie nowego segmentu
tworzy nowy span na końcu listy i **nie dotyka żadnej istniejącej kotwicy**.
Z tego wynika gwarancja: równoległy napływ segmentów nigdy nie unieważnia
trwającego zaznaczenia.

Kotwica może stać się nieważna tylko wtedy, gdy inna *edycja* usunęła span, na
który wskazuje. Przy jednym oknie klienta to nie zachodzi. Przy wielu klientach
operacja jest odrzucana, a klient przelicza dokument od nowa.

Konwersja w obie strony (offset w `GtkTextBuffer` ↔ kotwica) jest
odpowiedzialnością warstwy widoku i jest jedynym miejscem, gdzie globalne
offsety w ogóle występują.

## Zastępowanie przez ponowne podyktowanie

### Przebieg

1. Użytkownik zaznacza fragment w oknie i naciska `Ctrl+R` (albo przycisk
   „Podyktuj ponownie").
2. Klient zapamiętuje kotwice zaznaczenia oraz `replace_armed_at` — moment
   w osi czasu audio sesji, w którym tryb został włączony.
3. Okno wyróżnia zaznaczenie i pokazuje etykietę „zastępuję…".
4. Użytkownik mówi. VAD zamyka segment po ciszy, whisper go transkrybuje,
   daemon rozgłasza `segment.added`.
5. Klient rozstrzyga na podstawie `audio_start_ms`:
   - `audio_start_ms < replace_armed_at` → **append** (to resztka poprzedniej
     mowy, która czekała w kolejce)
   - `audio_start_ms >= replace_armed_at` → **splice** w zapamiętane kotwice
6. Po wykonaniu splice klient wraca do trybu append.

Krok 5 jest istotą poprawności. Granicę wyznacza **timestamp audio, nie
kolejność przybycia** — dzięki temu zaległości w kolejce nie powodują, że
zastąpieniem stanie się zdanie wypowiedziane sekundę wcześniej.

### Reguła zakończenia

Jedno zastąpienie zjada **dokładnie jedną wypowiedź** — jeden segment zamknięty
ciszą. Rozważana była alternatywa „aż użytkownik naciśnie Enter", ale wymaga
sięgania po klawiaturę w trakcie mówienia i psuje rytm dyktowania. Jeśli
poprawka nie mieści się w jednej wypowiedzi, użytkownik zaznacza nową wersję
i dyktuje ponownie.

`Escape` anuluje tryb zastępowania bez żadnych zmian w dokumencie.

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

- append w trakcie aktywnego zaznaczenia nie przesuwa kotwic
- splice przez granicę dwóch spanów tnie skrajne poprawnie
- splice obejmujący całe spany usuwa je w całości
- segment z `audio_start_ms` sprzed uzbrojenia trybu robi append, nie splice
- undo po splice odtwarza dokument znak w znak
- sekwencja losowych operacji: `fold(ops)` daje ten sam wynik przy każdym
  przeliczeniu (test własnościowy)
