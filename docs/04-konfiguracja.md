# 04 — Konfiguracja

## Zasada

**Żadnej magicznej stałej w kodzie.** Wszystkie progi, opóźnienia, limity
i ścieżki pochodzą z pliku konfiguracyjnego i mają udokumentowaną wartość
domyślną.

Na tym etapie konfiguracja odbywa się **wyłącznie przez pliki TOML**.
Przeniesienie wybranych opcji do interfejsu GUI jest osobną, późniejszą
decyzją — projekt ma być na to gotowy, ale nie płacimy za to teraz.

## Trzy pliki, trzy właściciele

`$XDG_CONFIG_HOME/diktate/` (zwykle `~/.config/diktate/`):

| Plik | Właściciel | Zawartość |
|------|-----------|-----------|
| `daemon.toml` | `diktate-daemon` | Audio, VAD, whisper, modele, journal, wake word |
| `agent.toml` | `diktate-agent` | Reguły komend głosowych |
| `gtk.toml` | `diktate-gtk` | Okno, skróty, wygląd, historia |

Rozdział nie jest kosmetyczny: agent na maszynie bez serwera wyświetlania nie
powinien nawet widzieć `gtk.toml`, a klient GUI nie ma powodu modyfikować
parametrów VAD.

Każdy proces waliduje swój plik przy starcie: błąd walidacji to odmowa startu
z czytelnym komunikatem, nigdy ciche wartości domyślne.

Przeładowanie odbywa się **przy każdym `session.start`**, a nie przez
`inotify`. Skutek dla użytkownika jest ten sam — zmiana progu VAD działa bez
restartu daemona, wystarczy zacząć nagrywać od nowa — a nie wymaga
podglądania katalogu ani wywracania trwającej sesji w połowie wypowiedzi.
Plik niepoprawny nie zabija działającego daemona: zostaje poprzednia
konfiguracja i głośne ostrzeżenie. `inotify` zostaje w zanadrzu na wypadek,
gdyby pojawiły się parametry, których nie da się odłożyć do następnej sesji.

Zapis pliku przez jakiekolwiek narzędzie musi być **atomowy**: zapis do pliku
tymczasowego w tym samym katalogu, potem `rename`. Inaczej inotify potrafi
złapać plik w połowie zapisu.

## `daemon.toml`

```toml
[audio]
device = "default"            # nazwa z `diktate-cli devices`
pre_roll_ms = 300             # audio sprzed detekcji mowy dołączane do segmentu
level_interval_ms = 50        # odstęp odczytów poziomu do wskaźnika (~20 Hz)

[vad]
threshold = 0.5               # próg decyzyjny Silero, 0.0–1.0
hangover_ms = 700             # cisza domykająca segment, gdy mowy już dość
min_speech_ms = 300           # krótsze odrzucane jako trzask
target_speech_ms = 6000       # ile mowy zebrać, zanim domkniemy hangoverem
long_silence_ms = 2000        # cisza domykająca zawsze — wyraźna przerwa
max_segment_ms = 15000        # twarde cięcie (okno whispera to 30 s)

[whisper]
model = ""                    # ścieżka ggml; puste = $DIKTATE_MODEL
threads = 0                   # 0 = decyzja whispera
language = "pl"               # kod ISO albo "auto"
initial_prompt = ""           # terminologia i nazwy własne
beam_size = 1                 # 1 = dekodowanie zachłanne

[queue]
max_pending_segments = 8
```

To jest **całość** dzisiejszego `daemon.toml`. Walidacja odrzuca nieznane
pola, więc sekcje z dalszych etapów — `[whisper.context]`, `[filters]`,
`[journal]`, `[wakeword]`, `[ipc]`, `[log]` i `[profiles.*]` — nie wpisuje
się „na zapas": plik z nimi nie przejdzie walidacji. Dochodzą razem z
etapami, które ich używają. Filtry halucynacji działają dziś na liście
wkompilowanej w `diktate-core`; przeniesienie ich do `[filters]` to część
tej samej pracy co journal.

### Dlaczego progi VAD nazywają się właśnie tak

Pierwsza wersja tego dokumentu opisywała progi w kategoriach „krótka
wypowiedź / długa wypowiedź" i dwóch różnych czasów ciszy. Implementacja
poszła inną drogą, świadomie: whisper przetwarza zawsze okno **30 sekund**,
więc koszt jednego segmentu jest stały niezależnie od jego długości —
dwusekundowy urywek kosztuje tyle samo, co dziesięciosekundowa wypowiedź.
Dlatego segmenter nie tnie na każdej pauzie, tylko **zbiera materiał** do
`target_speech_ms` i dopiero wtedy pozwala zwykłej ciszy (`hangover_ms`)
domknąć segment. Wyraźna przerwa (`long_silence_ms`) domyka zawsze.

Progi powyżej są nazwane tak, jak pokrętła, które faktycznie istnieją
w `SegmenterConfig` — jedna nazwa, jedno miejsce.

### Profile językowe (etap 3)

Nadpisują wybrane pola `[whisper]` i `[filters]`. Przełączane z tray-a,
`diktate-cli profile set <nazwa>` albo `session.start { profile }`.

```toml
[profiles.pl]
language = "pl"
initial_prompt = "Transkrypcja notatek technicznych. Terminy: Kubernetes, PostgreSQL, Wayland, systemd."

[profiles.en]
language = "en"
initial_prompt = "Technical dictation. Terms: Kubernetes, PostgreSQL, Wayland, systemd."

[profiles.szybki]
model = "base"
backend = "cpu"
beam_size = 1
```

## `gtk.toml`

```toml
[window]
width = 620
height = 420
show_level_meter = true

[dictation]
insert_at_cursor = true        # segment wchodzi w miejscu karetki, nie na końcu
auto_space = true              # spacja rozdzielająca na stykach
strip_trailing_ellipsis = true # usuwanie „...” na końcu przerwanej wypowiedzi
follow_caret = true            # przewijanie do punktu wstawiania, gdy był widoczny

[shortcuts]
copy = "<Control>c"
undo = "<Control>z"
redo = "<Control><Shift>z"
toggle_recording = "<Control>space"   # wewnątrz okna, nie globalny

[tray]
enabled = true
show_backlog = true
```

Pola `[replace]`, `[history]`, `implicit_replace`, `snap_to_word_boundary`,
`auto_case` i `show_confidence` dochodzą razem z etapem 2 — walidacja
odrzuca nieznane pola, więc nie wpisuje się ich „na zapas".

Skrót globalny (uruchamiający nagrywanie spoza okna) **nie jest tutaj** —
rejestruje go środowisko graficzne. Patrz [06-gui.md](06-gui.md#skrót-globalny).

## `agent.toml`

Opisany w całości w [05-agent.md](05-agent.md).

Sekcja `[wakeword]` powyżej jest opisana w [10-wake-word.md](10-wake-word.md).
Sekcje `[dictation]`, `[replace]` oraz progi `[vad]` i `[filters]` związane
z rytmem dyktowania mają uzasadnienie w
[07-zachowanie-dyktowania.md](07-zachowanie-dyktowania.md).

## Kolejność rozstrzygania

1. Wartości domyślne wkompilowane w binarkę
2. `$XDG_CONFIG_HOME/diktate/<plik>.toml`
3. Aktywny profil (tylko `daemon.toml`)
4. Argumenty wiersza poleceń (`--model`, `--language`, `--backend`, …)
5. Parametry pojedynczego wywołania (`session.start { profile }`)

## Walidacja

Każdy plik ma odpowiadającą mu strukturę `serde` z `#[serde(deny_unknown_fields)]`
— literówka w nazwie pola jest błędem, nie cichym pominięciem. Dodatkowo
sprawdzane są zakresy wartości (`threshold` w 0–1, `min_segment_ms <
max_segment_ms`, istnienie modelu, istnienie urządzenia audio).

`diktate-cli config check` waliduje wszystkie trzy pliki i wypisuje efektywną
konfigurację po rozstrzygnięciu profili i argumentów.
