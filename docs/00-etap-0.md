# 00 — Etap 0: szkielet przechodzący (walking skeleton)

Cel: **naciskam przycisk, mówię, naciskam stop — tekst pojawia się w oknie.**

Etap 0 nie jest częścią produktu. Jest dowodem, że pionowy przekrój przez całą
architekturę działa: mikrofon → whisper → gniazdo uniksowe → okno GTK. Wszystko,
co da się z tego przekroju usunąć bez zrywania ciągłości, jest usunięte.

Powód wydzielenia go z [etapu 1](09-roadmapa.md#etap-1--dyktowanie-do-schowka):
etap 1 zawiera VAD, transkrypcję strumieniową, filtry halucynacji, model spanów,
tray i konfigurację. Każda z tych rzeczy może zawieść niezależnie. Jeśli
zbudujemy je wszystkie naraz, pierwsze uruchomienie da nam kilka splecionych
błędów zamiast jednego. Etap 0 izoluje ryzyko sprzętowe — build whisper.cpp,
wybór urządzenia audio, jakość rozpoznawania polskiego — zanim cokolwiek
zostanie na nim zbudowane.

## 0a — audio i whisper bez interfejsu

Zakres:

- `diktate-core`: przechwytywanie przez `cpal`, 16 kHz mono f32, całość
  nagrania trzymana w `Vec<f32>` w pamięci. Bez ring buffera, bez pre-rolla.
- `whisper-rs`: jeden przebieg po **zakończeniu** nagrania. Nie strumieniowo.
- `diktate-cli record --seconds N` wypisuje rozpoznany tekst na stdout.

Kryterium zaliczenia: `diktate-cli record --seconds 10` po polsku zwraca tekst,
który da się przeczytać.

Backend whispera w 0a to **CPU**. Vulkan dokładamy dopiero wtedy, gdy ścieżka
CPU działa — inaczej nie wiadomo, czy problem leży w audio, czy w sterowniku.

## 0b — daemon, IPC i okno

Zakres:

- `diktate-proto`: dokładnie dwie metody i jedno powiadomienie.
  - `session.start` → `{}`
  - `session.stop` → `{}`
  - notyfikacja `segment.final` → `{ "text": "..." }`
- `diktate-daemon`: gniazdo `$XDG_RUNTIME_DIR/diktate.sock`, NDJSON,
  obsługa jednego klienta naraz. Bez journala, bez sesji, bez stanu poza
  „nagrywam / nie nagrywam".
- `diktate-gtk`: jedno okno. `TextView`, przycisk przełączający nagrywanie,
  przycisk „Kopiuj". Tekst z `segment.final` dopisywany na końcu bufora.

Kryterium zaliczenia: okno otwarte, przycisk wciśnięty, mowa, stop — tekst
w oknie, `Kopiuj` wkłada go do schowka.

## Świadomie pominięte

| Element etapu 1 | Zastępnik w etapie 0 |
|---|---|
| VAD (Silero przez `ort`) | granicę segmentu wyznacza przycisk Stop |
| Transkrypcja strumieniowa, `SegmentQueue` | jeden przebieg po nagraniu |
| Filtry halucynacji, normalizacja stykowa | brak — tekst surowy |
| `diktate-document`, spany, kotwice, `Ctrl+Z` | zwykły `TextBuffer`, dopisywanie na końcu |
| Konfiguracja TOML | stałe w kodzie: urządzenie domyślne, ścieżka modelu, `pl` |
| Tray (`ksni`), skrót globalny | przycisk w oknie |
| `model pull`, zarządzanie modelami | model pobrany ręcznie |
| Journal NDJSON | brak |
| Backend Vulkan | CPU |

## Dlaczego nie jeden proces

Sklejenie rdzenia z GTK w jedną binarkę byłoby szybsze o kilka godzin, ale
granica procesów jest fundamentem architektury
([01](01-architektura.md#podział-na-procesy), decyzja 3 w
[README](README.md#decyzje-które-już-zapadły)) — z niej wynika CLI, agent
i praca bez serwera wyświetlania. W wersji szczątkowej ta granica kosztuje
dwie metody RPC i jedno powiadomienie. Postawienie jej później oznaczałoby
rozplątywanie GTK z rdzenia w momencie, gdy dochodzi trzeci klient.

## Wymagania wstępne

- Rust (stabilny, przez `rustup`)
- `cmake`, kompilator C++ — whisper.cpp buduje się jako część `whisper-rs`
- `alsa-devel` (albo odpowiednik pipewire) dla `cpal`
- `gtk4-devel`, `libadwaita-devel`
- Model `ggml-base` albo `ggml-small`. Model `tiny` po polsku jest za słaby na
  dyktowanie — przyda się dopiero jako model czuwania
  ([10](10-wake-word.md)).

## Po etapie 0

Pierwszym przyrostem jest VAD: kiedy segmenty zaczynają się kończyć same,
przycisk Stop przestaje być potrzebny i reszta
[etapu 1](09-roadmapa.md#etap-1--dyktowanie-do-schowka) staje się serią
przyrostów na działającym szkielecie, a nie pierwszym uruchomieniem.
