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
silence_ms_to_end = 1500      # cisza domykająca krótką wypowiedź
silence_ms_to_end_long = 200  # cisza domykająca wypowiedź już rozpędzoną
long_segment_ms = 2500        # od tej długości obowiązuje krótszy próg
min_speech_ms = 300           # krótsze odrzucane jako trzask
max_segment_ms = 15000        # twarde cięcie (okno whispera to 30 s)

[whisper]
model = ""                    # ścieżka ggml; puste = $DIKTATE_MODEL,
                              # potem ~/.diktate/models/ggml-small-q5_1.bin
threads = 0                   # 0 = decyzja whispera
language = "pl"               # kod ISO albo "auto"
initial_prompt = ""           # terminologia i nazwy własne
beam_size = 1                 # 1 = dekodowanie zachłanne

[queue]
max_pending_segments = 16
```

To jest **całość** dzisiejszego `daemon.toml`. Walidacja odrzuca nieznane
pola, więc sekcje z dalszych etapów — `[whisper.context]`, `[filters]`,
`[journal]`, `[wakeword]`, `[ipc]`, `[log]` i `[profiles.*]` — nie wpisuje
się „na zapas": plik z nimi nie przejdzie walidacji. Dochodzą razem z
etapami, które ich używają. Filtry halucynacji działają dziś na liście
wkompilowanej w `diktate-core`; przeniesienie ich do `[filters]` to część
tej samej pracy co journal.

### Dlaczego progi VAD nazywają się właśnie tak

Próg ciszy jest **adaptacyjny**: dopóki wypowiedź trwa krócej niż
`long_segment_ms`, domyka ją dopiero `silence_ms_to_end` — użytkownik ma czas
na zastanowienie przed trudnym słowem. Gdy mówi już dłużej, obowiązuje
`silence_ms_to_end_long` i każda pauza między zdaniami natychmiast wypycha
tekst na ekran. Uzasadnienie rytmu:
[07-zachowanie-dyktowania.md](07-zachowanie-dyktowania.md#rytm-adaptacyjny-próg-ciszy).

O tempie pojawiania się tekstu decyduje w praktyce `long_segment_ms`, a nie
`silence_ms_to_end_long`. Po każdym cięciu nowy segment startuje od zera i musi
znów urosnąć do tego progu, zanim krótka cisza zacznie obowiązywać — skracanie
samej pauzy niczego więc nie przyspiesza, dopóki próg długości zostaje wysoki
(zmierzone na `testdata/jfk.wav`: 300 ms → 150 ms nie zmienia liczby segmentów,
5000 ms → 2500 ms zmienia ją z dwóch na trzy).

Ta segmentacja zakłada, że transkrypcja nadąża za mową, i przy `long_segment_ms
= 2500` margines jest cienki: segment kosztuje ~2,8 s (`small-q5_1`, Vulkan)
przy średniej długości ~3,4 s, czyli ~85% przepustowości. Dalsze obniżanie
progu przewraca tę proporcję — kolejka zacznie rosnąć, a tekst pojawiać się
**później**, nie wcześniej. Pomiary dla poszczególnych modeli i narzędzie,
którym się je robi (`--example bench-model`), opisuje `CLAUDE.md`, sekcja
„Wydajność transkrypcji".

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

[daemon]
autostart = true               # brak daemona = klient sam go uruchamia
start_timeout_ms = 30000       # ile czekać na gniazdo (mieści wczytanie modelu)

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

[correction]                   # korekta tekstu LLM-em, patrz docs/11
enabled = true                 # false ukrywa przycisk w oknie
endpoint = "http://home-host.tarpan-flops.ts.net:8080/v1/chat/completions"
model = "bielik-11b-v3"        # sprawdź /v1/models — serwer bywa przestawiany
api_key = ""                   # puste = bez nagłówka Authorization
temperature = 0.0              # i tak nie gwarantuje powtarzalności
max_tokens = 2000
timeout_ms = 120000
disable_thinking = true        # bez tego model z thinkingiem jest 5–45× wolniejszy
short_text_words = 40          # do tylu słów notatka jest „krótka"
max_change_ratio_short = 0.5   # próg zmian dla krótkiej notatki
max_change_ratio_long = 0.35   # próg dla dłuższej — tu leży ryzyko redakcji
min_changed_words = 2          # tyle zmian przechodzi zawsze, bez pytania
allowed_extra_chars = ""       # znaki ponad alfabet polski i interpunkcję
system_prompt = '''…'''        # instrukcja korektora; domyślna w docs/11
user_template = "Popraw:\n\n{text}"   # {text} zastępuje treść notatki
redaction_system_prompt = '''…'''     # instrukcja redaktora; domyślna w docs/11
redaction_user_template = "Polecenie: {instruction}\n\nTekst:\n\n{text}"
```

**Prompt jest polem konfiguracji, nie stałą w kodzie.** To on decyduje o całym
zachowaniu korekty: ten sam model z promptem ostrożnym nie naprawia nawet
oczywistych błędów, a z promptem pokazującym przykłady napraw działa
([11-korekta-llm.md](11-korekta-llm.md#prompt-decyduje-o-wszystkim)).
Dobieranie promptu jest więc strojeniem, a nie zmianą programu. Kreator
zapisuje go jako literał wielolinijkowy `'''…'''`, żeby dał się czytać
i poprawiać ręcznie.

Sekcja obsługuje **dwie** czynności: korektę transkrypcji i redakcję
poleceniem głosowym ([11-korekta-llm.md](11-korekta-llm.md#redakcja-poleceniem-głosowym)).
Dzielą serwer, model i limity, różnią się promptem — `system_prompt` zakazuje
zmiany stylu, `redaction_system_prompt` przeciwnie, każe wykonać polecenie
użytkownika. `enabled = false` ukrywa oba przyciski naraz. Progi
`max_change_ratio_*` dotyczą **wyłącznie korekty**: polecenie „zmień na
bardziej formalny" ma zmienić dużo i alarm przy 41% byłby tu szumem.

Szablon `redaction_user_template` musi zawierać oba pola — `{instruction}`
i `{text}` — inaczej start okna kończy się błędem walidacji: model bez
jednego z nich nie wie, co zrobić albo z czym.

Progi `max_change_ratio_*` sprawdzane są **razem z** `min_changed_words`:
przekroczenie samego odsetka przy jednej czy dwóch naprawionych pomyłkach
nie jest redakcją, tylko dobrze wykonaną korektą krótkiego zdania.

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

## Kreator `diktate setup`

Pliki TOML zostają jedynym źródłem konfiguracji, ale pierwsze ich napisanie
nie może być warunkiem wejścia. `diktate setup` prowadzi przez nie dialogiem
w terminalu i na koniec **zapisuje te same pliki**, które można potem
edytować ręcznie. Kreator nie jest drugą ścieżką konfiguracji — jest
generatorem pierwszej.

Kształt pól, wartości domyślne i walidację kreator bierze z `diktate-config`;
nie ma własnej listy pól, bo dwie listy rozjeżdżają się w tygodniach.

### Krótka ścieżka

Sześć pytań, każde z odpowiedzią domyślną pod Enterem: model, język,
urządzenie wejściowe (lista z `cpal`, bez nagrywania), rytm segmentacji,
ikona w zasobniku, skrót przełączający nagrywanie **wewnątrz okna**
(`gtk.toml`, `shortcuts.toggle_recording`). To świadomie nie jest skrót
naprawdę globalny — ten rejestruje osobno środowisko graficzne poleceniem
`diktate-gtk --install-shortcut` (patrz README, „Skrót globalny”), bo to
GNOME/inny kompozytor trzyma globalne powiązania klawiszy, a nie pliki
Diktate. `diktate setup --advanced` przechodzi
zamiast tego przez każde pole `daemon.toml` i `gtk.toml` po kolei, z bieżącą
wartością w nawiasie.

### Profile rytmu

Progi VAD w oderwaniu od siebie nie znaczą nic, więc krótka ścieżka pyta
o jeden wybór, a nie o trzy liczby:

| profil | `long_segment_ms` | `silence_ms_to_end_long` | `silence_ms_to_end` |
|--------|-------------------|--------------------------|---------------------|
| szybki | 2000 | 200 | 1200 |
| zrównoważony (domyślny) | 2500 | 200 | 1500 |
| dokładny | 4000 | 400 | 2000 |

„Szybki" schodzi poniżej progu przepustowości policzonego wyżej dla
`small-q5_1`, więc kreator oferuje go bez zastrzeżeń tylko przy szybszym
modelu; przy wolniejszym mówi wprost, że kolejka zacznie rosnąć. Plik
napisany ręcznie, którego progi nie odpowiadają żadnemu profilowi, kreator
pokazuje jako „profil własny" i nie podmienia go po cichu: pytanie „Rytm
segmentacji?” dostaje wtedy dodatkową, czwartą pozycję na liście — „własny”,
z bieżącymi trzema progami wypisanymi wprost w opisie — i to ona jest
odpowiedzią domyślną pod Enterem, więc samo przejście przez kreator niczego
nie zmienia w rytmie, dopóki ktoś świadomie nie wybierze innej pozycji. Ta
pozycja pojawia się wyłącznie w krótkiej ścieżce: `--advanced` w ogóle
pomija pytanie o profil i pyta wprost o `vad.long_segment_ms`,
`vad.silence_ms_to_end` oraz `vad.silence_ms_to_end_long` jako zwykłe pola
z dzisiejszą wartością w nawiasie — tam pojęcie „profilu własnego” nie
istnieje, bo nie ma czego nazywać: nie pada pytanie, z którym miałby się
różnić.

### Zapis

Kolejność jest częścią umowy, nie szczegółem implementacji:

1. wczytanie stanu bieżącego (brak pliku to wartości domyślne, nie błąd),
2. dialog zmienia kopię tego stanu — dlatego domyślne odpowiedzi to
   wartości **dzisiejsze**, a nie fabryczne,
3. walidacja wyniku: kreator nie potrafi zapisać pliku, który zablokowałby
   start daemona,
4. lista różnic na ekranie (`vad.long_segment_ms: 2500 → 4000`)
   i potwierdzenie,
5. kopia `daemon.toml.bak`, potem zapis atomowy (plik tymczasowy obok,
   `rename`) — jak każdy inny zapis konfiguracji.

Plik po kreatorze zawiera polskie komentarze nad sekcjami i polami, żeby
nadawał się do dalszej ręcznej edycji. Ręczne komentarze użytkownika **nie
są** zachowywane: zapis nadpisuje plik w całości, od tego jest `.bak`.

Po zapisie kreator wykrywa działającego daemona (plik PID) i proponuje
`diktate-daemon --restart`. Bez tego zmiany wyglądałyby, jakby nie działały.

### Modele

`diktate model list` pokazuje kuratorowaną listę modeli z rozmiarem,
zmierzonym czasem przebiegu i oceną polszczyzny, oznaczając te już obecne na
dysku; `diktate model pull <nazwa>` pobiera wybrany. Lista jest krótka
i wkompilowana celowo — pełny spis z API HuggingFace zawiera warianty bez
sensu na tym sprzęcie, a odpytywanie sieci przy samym wyświetleniu listy
psuje kreator bez internetu.

Pobieranie idzie do pliku `.part` (wznawialnego nagłówkiem `Range`)
i dopiero po zgodnej sumie SHA-256 dostaje nazwę docelową. Półpobrany
`ggml-*.bin` jest gorszy niż jego brak: whisper przewraca się na nim
komunikatem, który niczego nie tłumaczy.

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
