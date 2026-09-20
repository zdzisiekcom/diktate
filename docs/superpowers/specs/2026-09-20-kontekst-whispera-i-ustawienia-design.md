# Kontekst dla whispera i okno ustawień

Data: 2026-09-20. Stan: design zatwierdzony do spisania planu.

## Problem

Whisper dostaje dziś jeden prompt na całe życie daemona:
`whisper.initial_prompt` z `daemon.toml`, wczytany raz przy budowaniu
`Transcriber`. Każdy segment jest przez to transkrybowany tak, jakby był
pierwszym zdaniem dnia — model nie wie, że trzy sekundy wcześniej padło
„Wayland”, więc za czwartym razem napisze „waylend”, a nazwisko rozpoznane
raz poprawnie potrafi za chwilę rozpaść się na trzy warianty.

Specyfikacja przewiduje na to lekarstwo od początku i opisuje je w trzech
miejscach — `docs/01#kontekst-między-segmentami`,
`docs/02#kontekst-transkrypcji`, `docs/07#kontekst-dla-whispera` — ale nic
z tego nie jest zaimplementowane: metody `session.context` nie ma w
`diktate-proto`, sekcji `[whisper.context]` nie ma w `diktate-config`,
strażnika pętli nie ma nigdzie. Ten dokument domyka rozjazd między
specyfikacją a kodem i dokłada do niego drugą połowę, której specyfikacja nie
miała: miejsce, w którym użytkownik **widzi i edytuje** prompt, zamiast
szukać go w pliku TOML.

Prompt jest przy tym główną dźwignią jakości polskiej transkrypcji
(`docs/01#prompt`), a dźwignia zakopana w pliku konfiguracyjnym daemona to
dźwignia, której się nie rusza.

## Zakres

- `session.context` w protokole: klient przysyła gotowy prompt, daemon używa
  go przy kolejnych segmentach.
- Kontekst budowany po stronie klienta z **tekstu dokumentu przed punktem
  wstawiania**, nie z logu transkrypcji.
- Strażnik pętli w daemonie: segment będący powtórzeniem promptu liczony
  ponownie bez kontekstu.
- Okno ustawień w `diktate-gtk` (`adw::PreferencesDialog`) z dwiema
  stronami: transkrypcja (szablon promptu, długość kontekstu) i korekta LLM.
- Zapis `gtk.toml` z poziomu GUI — a więc przeniesienie istniejącego pisarza
  konfiguracji z `diktate-cli` do `diktate-config`.

Poza zakresem: wybór urządzenia audio i pobieranie modeli z okna ustawień
(wymagają `model.list`/`model.pull` i pisania po `daemon.toml` — osobna
robota), zakładka „Akcje głosowe” (`docs/05`), `whisper.context.source =
"transcript"` (nie ma journalu, z którego miałby brać ogon).

## Zmiany w specyfikacji

Trzy świadome odstępstwa od dzisiejszych dokumentów. Wszystkie trzy wchodzą
do `docs/` razem z wdrożeniem, nie zostają w tym pliku.

**Kontekst nadpisuje `initial_prompt`, nie dokleja się do niego.**
`docs/01#kontekst-między-segmentami` mówi dziś, że ogon poprzedniej
transkrypcji jest doklejany do `initial_prompt`. Skoro jednak szablon
w oknie ustawień pokazuje **cały** prompt razem z miejscem na kontekst, to
co użytkownik widzi w polu, ma być dokładnie tym, co dostaje model. Doklejanie
z przodu niewidocznego zdania z `daemon.toml` łamałoby tę obietnicę.
`whisper.initial_prompt` zostaje więc promptem dla klientów, którzy kontekstu
nie przysyłają — `diktate-cli record`, `--example bench-model`, cudze klienty.

**`whisper.context.chars` żyje w `gtk.toml`, nie w `daemon.toml`.**
`docs/01` i `docs/02` umieszczają to pole po stronie daemona. Ogon dokumentu
tnie jednak ten, kto dokument zna, czyli klient — daemon nie wie nawet, gdzie
w przysłanym tekście kończy się szablon, a zaczyna kontekst. Po stronie
daemona zostaje tylko awaryjny twardy limit długości całego promptu.

**Okno ustawień to `adw::PreferencesDialog`.**
`docs/06#konwencje-widoków` wymaga dziś od **każdego** widoku
`adw::ApplicationWindow` z `ToolbarView` i `HeaderBar`. `PreferencesDialog`
niesie ten układ wewnątrz siebie i jest natywnym wzorcem GNOME dla ustawień,
więc konwencja dostaje jedno zdanie wyjątku. Reszta konwencji — grupy przez
`PreferencesGroup`, wiersze przez `ActionRow`/`SpinRow`, ikony symboliczne,
teksty po polsku — obowiązuje bez zmian.

## Architektura

Kontekst wędruje w jedną stronę i nigdy w drugą:

```
dokument w oknie ─┐
                  ├─► szablon z {kontekst} ─► session.context ─► slot promptu
szablon z gtk.toml┘        (klient)              (IPC)            (daemon)
                                                                     │
                                          segment audio ─► transcribe(próbki, prompt)
```

Daemon nie zna dokumentu i nie próbuje go poznać — dostaje ciąg znaków.
Klient nie wie, co daemon z nim zrobi — wysyła podpowiedź, nie zobowiązanie.

### `diktate-core`: prompt jako argument

`Transcribe::transcribe` dostaje drugi argument:

```rust
fn transcribe(&self, samples: &[f32], prompt: &str) -> Result<String>;
```

`Transcriber` (whisper) podaje go jako `initial_prompt` zamiast pola
`self.initial_prompt`; pole zostaje jako wartość używana, gdy przekazany
prompt jest pusty. `ParakeetTranscriber` argument **ignoruje** i mówi o tym
w komentarzu — model nie przyjmuje promptu i nie ma udawać, że przyjmuje.

Nowy moduł `crates/diktate-core/src/prompt.rs` — cała logika promptu,
bez wejścia/wyjścia, testowalna bez mikrofonu i bez modelu:

- `render(template: &str, context: &str) -> String` — podstawienie
  `{kontekst}`. Brak symbolu w szablonie znaczy „prompt bez kontekstu”
  i jest poprawnym stanem, nie błędem.
- `tail(text: &str, chars: usize) -> &str` — ogon po granicy znaku, nie
  bajtu, docięty do najbliższej granicy słowa od lewej, żeby model nie
  dostawał urwanej sylaby na starcie.
- `clamp(prompt: &str, max_chars: usize) -> &str` — awaryjne obcięcie
  od początku (bliżej kursora jest ważniejsze).
- `looks_like_echo(segment: &str, prompt: &str, threshold: f32) -> bool` —
  podobieństwo Jaccarda na trigramach słów.

### Slot promptu w sesji

`Session::start` dostaje `Arc<PromptSlot>` (`Mutex<String>` opakowany
w `prompt.rs`). `run_transcriber` czyta slot przed każdym segmentem —
kontekst przysłany w trakcie mówienia działa od następnej wypowiedzi, bez
restartu sesji.

Slot należy do **połączenia**, nie do sesji: przeżywa `session.stop`, więc
kontekst wysłany przed pierwszym `session.start` obowiązuje od pierwszej
wypowiedzi. Każde połączenie ma własny slot — dwa klienty nie mieszają sobie
promptów.

### Strażnik pętli

W `run_transcriber`, po transkrypcji, przed filtrami: jeśli
`context.repetition_guard` jest włączony i `looks_like_echo(tekst, prompt,
próg)`, segment leci drugi raz z pustym promptem, a wynik drugiego przebiegu
jest tym, co idzie dalej. Koszt to jeden dodatkowy przebieg modelu na
wpadkę — tanio wobec whispera zapętlonego na powtarzaniu kontekstu.

Zdarzenie idzie do logu (`eprintln!`), bo cicha podwójna transkrypcja
wygląda w pomiarach czasu jak nagłe spowolnienie bez powodu.

## Protokół

Nowa metoda, dokładnie jak w `docs/02#kontekst-transkrypcji`:

| Metoda | Parametry | Wynik |
|--------|-----------|-------|
| `session.context` | `{ text: string }` | `{ chars }` |

`Request::params` przestaje być `Option<Subscription>` i staje się
`Option<RequestParams>`:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RequestParams {
    Subscribe(Subscription),
    Context(ContextParams),
}
```

Pułapka, którą trzeba zamknąć testem: `Subscription` ma wszystkie pola
z `default`, więc przy `untagged` dopasowałoby się do **pustego obiektu**,
a tym samym do `{"text": "…"}`, i kontekst zniknąłby po cichu jako
subskrypcja bez poziomów. Oba warianty dostają więc `deny_unknown_fields`,
a test sprawdza, że `{"text":"abc"}` deserializuje się jako `Context`
i że `{"levels":true}` nadal deserializuje się jako `Subscribe`.

`scripts/smoke-ipc.py` dostaje krok: `session.context` przed
`session.start`, z asercją na `chars` w odpowiedzi.

Metoda jest podpowiedzią, nie zobowiązaniem — daemon bez kontekstu
transkrybuje normalnie, a pusty `text` czyści slot.

## Konfiguracja

`daemon.toml` — sekcja zarezerwowana w `docs/04:79` przestaje być
zarezerwowana:

```toml
[whisper.context]
max_prompt_chars = 900     # twardy limit długości promptu (whisper mieści ~224 tokeny)
repetition_guard = true    # segment będący echem promptu licz ponownie bez kontekstu
repetition_threshold = 0.8 # podobieństwo, od którego segment jest echem
```

`gtk.toml` — nowa sekcja po stronie klienta:

```toml
[whisper]
prompt = ""          # szablon; {kontekst} to miejsce na ogon dokumentu
context_chars = 200  # ile znaków przed kursorem wchodzi w {kontekst}
```

Pusty `prompt` znaczy „nie wysyłaj kontekstu w ogóle” — daemon zostaje wtedy
przy `whisper.initial_prompt`, czyli zachowaniu sprzed tej zmiany. To jest
wartość domyślna, więc aktualizacja niczego nikomu nie zmienia, dopóki sam
nie wejdzie w ustawienia.

Walidacja: `context_chars` ≤ `max_prompt_chars` nie jest sprawdzane między
plikami (klient nie czyta `daemon.toml`) — od tego jest awaryjne obcięcie
po stronie daemona.

### Jeden pisarz konfiguracji

`diktate-cli/src/setup/write.rs` już umie wyrenderować `gtk.toml`
z komentarzami i zapisać go atomowo z kopią `.bak`. GTK nie może zależeć od
binarki CLI, a druga ścieżka zapisu oznaczałaby dwie listy pól do
utrzymania — więc renderowanie i zapis przenoszą się do `diktate-config`
jako `GtkConfig::save()` i `DaemonConfig::save()`, a `diktate setup` woła to
samo. `write.rs` zostaje przy tym, co jest kreatorem: dialogiem i `.bak`.

Konsekwencja, którą trzeba nazwać: zapis z okna ustawień **kasuje ręczne
komentarze** w `gtk.toml`, tak samo jak dziś robi to kreator. Poprzednia
wersja zostaje w `gtk.toml.bak`.

## Klient GTK

### Skąd bierze się kontekst

`DocumentView::context_before(chars) -> String` — ogon tekstu przed punktem
wstawiania, a przy uzbrojonym zastąpieniu przed **początkiem zaznaczenia**
(`docs/07`: model ma widzieć to, co poprzedza miejsce, w które trafi nowy
tekst). Widok zna już `target()` i `mark()`, więc to kilka linii nad
istniejącym `TextBuffer`.

### Kiedy leci do daemona

Po każdej zmianie bufora, z odbiciem 400 ms, i tylko gdy sklejony prompt
różni się od ostatnio wysłanego. Odbicie jest konieczne, bo pisanie
z klawiatury zmienia bufor przy każdym znaku, a pusty warunek „różni się”
odcina powtórki przy ruchu karetki bez edycji.

Ta sama droga obsługuje segmenty od daemona i ręczne poprawki — i o to
chodzi: po poprawieniu źle rozpoznanego nazwiska model widzi wersję
poprawioną, a nie własną pomyłkę sprzed minuty.

Wysyłka jest bezpieczna przy braku daemona: błąd gniazda jest logowany
i porzucany, bo kontekst to podpowiedź, a nie akcja użytkownika.

### Okno ustawień

`crates/diktate-gtk/src/settings.rs`, `adw::PreferencesDialog`, dwie strony.

**Transkrypcja** (`audio-input-microphone-symbolic`):

- grupa „Prompt whispera” — wielolinijkowy `TextView` w karcie; opis grupy
  tłumaczy, że `{kontekst}` jest miejscem, w które wchodzi tekst sprzed
  kursora, a jego brak znaczy „prompt bez kontekstu”,
- grupa „Kontekst” — `SpinRow` „Znaków przed kursorem”, 0–1000, krok 50,
- grupa „Pliki” — `ActionRow` otwierające `gtk.toml` i `daemon.toml`
  w domyślnym edytorze.

**Korekta** (`text-editor-symbolic`) — pola, które i tak żyją w `gtk.toml`:
`enabled`, `endpoint`, `model`, `system_prompt`, `redaction_system_prompt`,
`max_change_ratio_short`, `max_change_ratio_long`, `min_changed_words`.
`api_key`, `temperature`, `max_tokens`, `timeout_ms` i szablony
`user_template` zostają w pliku — to strojenie, nie ustawienia.

Zapis przy zamknięciu okna. Po zapisie: `Toast` w oknie głównym
(„Zapisano ustawienia”), przeliczenie promptu i natychmiastowa wysyłka
nowego kontekstu — zmiana ma być widoczna w następnej wypowiedzi, bez
restartu czegokolwiek.

Przycisk otwierający: `pack_start` zaraz za przyciskiem mikrofonu (prawa
strona paska jest zajęta przez sześć akcji i siódma się tam nie mieści),
ikona `emblem-system-symbolic`, tooltip „Ustawienia”, skrót `Ctrl+,`.

## Obsługa błędów

| Sytuacja | Zachowanie |
|----------|------------|
| Prompt dłuższy niż `max_prompt_chars` | Daemon obcina od początku i pisze o tym w logu; transkrypcja idzie dalej |
| Segment jest echem promptu | Drugi przebieg bez promptu, wpis w logu |
| `session.context` bez daemona | Błąd logowany i porzucony — kontekst to podpowiedź |
| Nieudany zapis `gtk.toml` | `Toast` z treścią błędu; okno zostaje otwarte, ustawienia niezapisane |
| Szablon bez `{kontekst}` | Poprawny stan: prompt stały, kontekst nie wchodzi |
| Pusty szablon | Kontekst nie jest wysyłany wcale; daemon używa `whisper.initial_prompt` |

## Testy

Bez mikrofonu, bez modelu, bez GTK:

- `prompt.rs`: podstawienie `{kontekst}`, szablon bez symbolu, ogon po
  granicy znaku (polskie diakrytyki), ogon docięty do granicy słowa,
  awaryjne obcięcie od początku, podobieństwo — a przede wszystkim test
  tego, czego strażnik ruszyć **nie może**: prawdziwe zdanie powtarzające
  kilka słów z kontekstu nie jest echem (`CLAUDE.md`: filtr zjadający
  prawdziwy tekst jest gorszy od przepuszczonej halucynacji).
- `diktate-proto`: `{"text":…}` → `Context`, `{"levels":true}` → `Subscribe`,
  obie w obie strony.
- `diktate-config`: wartości domyślne nowych pól, `gtk.toml` z sekcją
  `[whisper]` w obie strony, odrzucenie nieznanego pola.
- `diktate-core/session`: slot czytany przed każdym segmentem; zmiana slotu
  w trakcie działa od następnego segmentu (atrapa `Transcribe` zapisująca
  otrzymane prompty).
- `diktate-gtk/view`: `context_before` przy karetce i przy zaznaczeniu,
  krótszy bufor niż limit.
- `diktate-gtk/settings`: mapowanie konfiguracja↔pola w obie strony jako
  czyste funkcje, bez tworzenia widżetów.

Ręcznie, bo tego nie da się zautomatyzować: podyktowanie akapitu
z terminologią przy pustym i przy wypełnionym szablonie — i sprawdzenie,
że nazwy własne trzymają jedną pisownię.

## Dokumenty do poprawienia razem z kodem

- `docs/01` — kontekst nadpisuje prompt; `chars` po stronie klienta.
- `docs/02` — `session.context` przestaje być planem; `RequestParams`.
- `docs/04` — `[whisper.context]` w `daemon.toml`, `[whisper]` w `gtk.toml`,
  skreślenie sekcji z listy „zarezerwowanych”.
- `docs/06` — okno ustawień: zakres i wyjątek na `PreferencesDialog`;
  przycisk w pasku; skrót `Ctrl+,`.
- `docs/07` — kontekst i strażnik pętli jako stan faktyczny.
- `README` — wzmianka o oknie ustawień w opisie klienta GTK (listy skrótów
  README nie prowadzi, więc `Ctrl+,` zostaje w `docs/06`).
