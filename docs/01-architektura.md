# 01 — Architektura

## Cel

Dyktowanie długich notatek na lokalnym komputerze, bez wysyłania czegokolwiek
do sieci, z transkrypcją pojawiającą się na bieżąco i możliwością poprawiania
tekstu głosem w trakcie mówienia. Docelowo także wykonywanie akcji na podstawie
komend głosowych i wpisywanie tekstu bezpośrednio do aktywnego okna.

## Wybór języka

Rozważane były Rust i Python. Wybrano **Rust** na podstawie czterech kryteriów.

### Akceleracja sprzętowa

Sprzęt docelowy to Intel Meteor Lake (iGPU Arc + NPU) oraz druga maszyna z AMD.
Żadna nie ma CUDA, co wyklucza najwygodniejszą ścieżkę pythonową —
`faster-whisper`/CTranslate2 wspiera wyłącznie CPU i CUDA. Pozostają backendy
whisper.cpp: Vulkan, SYCL/oneAPI, OpenVINO, ROCm.

W obu językach wywoływane jest to samo C++, różnica leży w jakości bindingów.
Pythonowe `pywhispercpp` jest cieńsze, wolniej nadąża za upstreamem, a zmiana
backendu wymaga przebudowy kółka.

Pierwotnie stał tu `whisper-rs`, ale okazał się dokładnie tym, czego zarzucano
Pythonowi: zostawał w tyle za silnikiem. Najnowszy `whisper-rs-sys` wozi
whisper.cpp 1.8.3, a więc bez Parakeeta, i to przesądziło. Dziś whisper.cpp
jest submodułem, a wiązania są własne i wąskie — warstwa pośrednia w C++ trzyma
wiedzę o układzie struktur silnika, przez granicę FFI idą tylko wskaźniki
i liczby ([12-backendy-transkrypcji.md](12-backendy-transkrypcji.md),
`crates/diktate-whispercpp-sys`).

### Audio czasu rzeczywistego

Rdzeń aplikacji to ciągły ring buffer, VAD na strumieniu i równoległa
transkrypcja poprzedniego segmentu — bez zacinania interfejsu. Rust daje wątki
bez GIL i deterministyczne opóźnienia. W Pythonie transkrypcja i pętla GTK
dzielą interpreter, a callback `sounddevice` potrafi gubić ramki przy GC.

### Zajętość pamięci

Rust + GTK4: ~30–60 MB RSS poza modelem. Python + PyGObject + numpy +
onnxruntime: ~200–300 MB przed wykonaniem jakiejkolwiek pracy. Aplikacja
rezyduje w tray-u przez cały dzień, więc różnica jest odczuwalna.

### Dystrybucja

Rust produkuje pojedynczą binarkę. PyInstaller z PyGObject wymaga pakowania
typelibów, schematów GSettings i bibliotek natywnych, i w praktyce kończy się
instrukcją „zainstaluj zależności systemowe".

### Co tracimy

Szybkość pierwszego prototypu oraz ekosystem ML. Ta druga strata jest pozorna:
lokalny LLM do redakcji tekstu wystawia HTTP (Ollama, `llama-server` — API
zgodne z OpenAI), więc integracja to `reqwest` + `serde`, ewentualnie SSE dla
streamingu. Diaryzacja (`pyannote`), gdyby kiedyś była potrzebna, również
najlepiej żyje jako osobny proces, nie jako zależność frontendu.

## Podział na procesy

Rozdzielenie interfejsu od rdzenia jest wymaganiem, nie optymalizacją: aplikacja
ma działać także poza GNOME, w tym z terminala i na maszynie bez serwera
wyświetlania.

```
┌──────────────────────────────────────────────────────────┐
│ diktate-daemon                                           │
│   audio (cpal) → VAD (Silero/ort) → whisper.cpp           │
│   journal segmentów, serwer JSON-RPC na gnieździe UNIX    │
└──────────────┬───────────────────────────────────────────┘
               │ $XDG_RUNTIME_DIR/diktate.sock
    ┌──────────┼──────────────┬──────────────────┐
    ▼          ▼              ▼                  ▼
diktate-gtk  diktate-cli  diktate-agent   (przyszli klienci)
okno + tray  stdout/skrypty  akcje głosowe
```

Daemon **nie linkuje GTK** i nie wie nic o dokumencie, zaznaczeniach ani
redakcji. Jego kontrakt brzmi: *oto strumień tego, co użytkownik mówi*. Każdy
klient decyduje samodzielnie, co z tym zrobić.

Konsekwencja, która jest czystym zyskiem: komendy głosowe nie muszą siedzieć
w GUI. `diktate-agent` jest niezależnym konsumentem tego samego strumienia,
działa bez okna i bez GNOME.

## Crate'y

Jedno repozytorium, cargo workspace.

| Crate | Typ | Odpowiedzialność |
|-------|-----|------------------|
| `diktate-core` | lib | Capture, VAD, kolejka segmentów, whisper, journal, config daemona |
| `diktate-proto` | lib | Typy protokołu IPC (serde). Jedyne źródło kształtu wiadomości. Trzyma też lokalizacje sesyjne — ścieżkę gniazda i plik PID — bo są wspólne dla daemona i klientów |
| `diktate-document` | lib | Model dokumentu: spany, operacje, splice, undo |
| `diktate-daemon` | bin | `diktate-core` + serwer IPC |
| `diktate-cli` | bin | Klient terminalowy, zarządzanie modelami, `toggle` |
| `diktate-gtk` | bin | Okno GTK4/Adwaita, tray, ustawienia |
| `diktate-agent` | bin | Dopasowywanie komend głosowych, wykonywanie akcji |

`diktate-core` da się przetestować bez uruchamiania jakiegokolwiek procesu.
`diktate-document` da się przetestować bez audio i bez GTK. `diktate-proto`
zależy od niego wszystko, więc niezgodność wersji protokołu ujawnia się przy
kompilacji, nie w runtime.

## Przepływ danych — jedno dyktowanie

```
[hotkey / tray / diktate-cli toggle]
      │  RPC: session.start
      ▼
 daemon ──► AudioCapture (cpal, 16 kHz mono f32, ring buffer ~30 s)
              │ ramki 30 ms, callback tylko wrzuca do rtrb i wraca
              ▼
            VAD (Silero v5 przez ort)
              │ granice mowy + pre-roll 300 ms
              ▼
            SegmentQueue (bounded, domyślnie 8)
              │
              ▼
            Transcriber (whisper.cpp, Vulkan, model w pamięci)
              │ tekst segmentu
              ▼
            Filtry halucynacji ──► odrzucone znikają
              │
              ├──► journal (NDJSON, append-only)
              └──► broadcast segment.added ──► wszyscy subskrybenci
```

Transkrypcja segmentu *N* biegnie równolegle z capture i VAD segmentu *N+1*.
Kolejka jest ograniczona: gdy użytkownik mówi szybciej, niż whisper nadąża,
segmenty czekają, a daemon raportuje `backlog` w zdarzeniu `state.changed`.
Nic nie jest gubione, ale klient może pokazać, że sprzęt nie wyrabia.

## Pipeline audio — szczegóły

### Capture

`cpal` na PipeWire (ALSA jako fallback), 16 kHz mono f32 — natywny format
whispera, zero resamplingu w gorącej ścieżce. Callback audio jest nienaruszalny:
wrzuca ramki do lock-free ring buffera (`rtrb`) i natychmiast wraca.

### Pre-roll

Nagrywanie segmentu zaczyna się *przed* potwierdzeniem mowy przez VAD: po
detekcji początku wypowiedzi sięgamy do bufora wstecz o `pre_roll_ms`
(domyślnie 300). Bez tego regularnie ginie pierwsza sylaba, bo VAD potrzebuje
kilku ramek na decyzję. Przy starcie hotkeyem pozwala to zacząć mówić
w tej samej chwili, w której naciskany jest klawisz.

### VAD

Silero v5 przez `ort` (ONNX Runtime), ramki 30 ms. Maszyna stanów:

- `Silence → Speech` po `speech_frames_to_start` ramkach powyżej progu
- `Speech → Silence` po `silence_ms_to_end` poniżej progu → segment zamknięty
- twardy limit `max_segment_ms` — whisper ma okno 30 s i dłuższe audio
  obsługuje gorzej
- `min_segment_ms` — krótsze fragmenty odrzucane jako trzaski

VAD wbudowany w whisper.cpp nie nadaje się do tego zastosowania, bo działa na
całym nagraniu wstecz, a potrzebna jest decyzja na żywo.

### Transkrypcja

whisper.cpp przez własne wiązania (`diktate-whispercpp-sys`); do wyboru
whisper albo Parakeet ([12](12-backendy-transkrypcji.md)). Model ładowany raz
przy starcie daemona i trzymany w pamięci —
ładowanie `large-v3` zajmuje kilkanaście sekund i nie może się dziać per
segment. Jeden dedykowany wątek transkrypcji.

### Kontekst między segmentami

Kontekst poprawia spójność pisowni nazw własnych i stylu, ale wewnętrzny
mechanizm whispera (przenoszenie tokenów) jest niewidoczny i niesterowalny,
a jedna wpadka potrafi rozkręcić model w pętlę powtarzania tej samej frazy.

Dlatego kontekst jest budowany **jawnie po naszej stronie**, i to po stronie
**klienta**: prompt składa się z szablonu (`gtk.toml`, `[whisper] prompt`)
i ogona dokumentu sprzed punktu wstawiania (`context_chars`, domyślnie 200
znaków), po czym leci do daemona metodą `session.context`
([02](02-protokol-ipc.md#kontekst-transkrypcji)). Tekst od klienta **nadpisuje**
`whisper.initial_prompt`, nie dokleja się do niego — co użytkownik widzi
w oknie ustawień, ma być dokładnie tym, co dostaje model, a doklejanie
z przodu niewidocznego zdania z `daemon.toml` łamałoby tę obietnicę.
`initial_prompt` obowiązuje więc tylko wtedy, gdy klient kontekstu nie
przysyła (`diktate-cli record`, `--example bench-model`, cudze klienty).

**Strażnik pętli** (`repetition_guard`): jeśli nowy segment jest w znacznej
części powtórzeniem promptu (podobieństwo powyżej `repetition_threshold`) lub
zawiera zapętloną frazę, segment leci drugi raz z pustym promptem, a do
klienta idzie wynik tego drugiego przebiegu. Domyślnie kontekst włączony.

### Prompt

`initial_prompt` z konfiguracji — słownictwo branżowe, nazwy własne,
preferowana interpunkcja. Definiowalny per profil językowy. To główna dźwignia
jakości transkrypcji polskiej tam, gdzie nikt szablonu z `[whisper] prompt`
nie przysyła (wyżej) — w kliencie GTK tę rolę przejmuje ten szablon.

### Filtr halucynacji

Whisper na ciszy i szumie produkuje powtarzalne artefakty („Napisy stworzone
przez społeczność Amara.org", „Dziękuję za uwagę", ciągi kropek). Segmenty
pasujące do wzorców z `filters.drop_patterns` są odrzucane, zanim trafią do
klientów. Lista jest konfigurowalna, bo zależy od modelu i języka.

## Backendy i modele

Vulkan jako domyślny — jeden kod pokrywa Intel Arc i AMD RDNA, bez oneAPI
i bez ROCm. `openvino` (enkoder na NPU Intela) i `rocm` to opcjonalne feature
flagi Cargo, kompilowane tylko na życzenie. CPU zawsze dostępne.

Przy starcie daemon inicjalizuje wybrany backend. Przy niepowodzeniu schodzi na
CPU, ale **jawnie**: ostrzeżenie w logu i w zdarzeniu `state.changed`. Cicha
degradacja do CPU, gdzie `large-v3` liczy się minutę na segment, byłaby
najgorszym możliwym zachowaniem.

Modele w `~/.diktate/models/`, pobierane z HuggingFace przez
`diktate model pull <nazwa>` z weryfikacją sumy SHA-256. Katalog jest celowo
poza `$XDG_DATA_HOME`: ścieżka ma być przewidywalna i łatwa do wskazania,
a wielogigabajtowe wagi nie są danymi, które ktokolwiek chce synchronizować.
Interaktywnie ten sam pobieracz obsługuje `diktate setup`
([04-konfiguracja.md](04-konfiguracja.md#kreator-diktate-setup)). Zmiana modelu w locie
(`model.load`): daemon ładuje nowy, przełącza się po opróżnieniu kolejki,
zwalnia stary.

## Journal

Daemon prowadzi append-only journal surowych segmentów bieżącej sesji
(NDJSON w `$XDG_STATE_HOME/diktate/`) oraz krótki bufor ostatnich N segmentów
w pamięci. To **dziennik, nie rozumienie tekstu** — daemon numeruje i zapisuje,
nie interpretuje.

Trzy rzeczy, których inaczej nie ma:

- klient podłączony w trakcie mówienia dostaje to, co przegapił
  (`session.subscribe { since_seq }`), zamiast pustego okna
- awaria albo zamknięcie okna GTK nie kasuje dziesięciu minut dyktowania
- `diktate-agent` i okno GTK słuchają równolegle, każdy dostaje komplet

Historia *zredagowana* — to, co widać w oknie po poprawkach — jest zapisywana
przez klienta, osobno i w innym formacie. Patrz [03](03-model-dokumentu.md).
