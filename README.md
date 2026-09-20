# Diktate

Diktate to narzędzie do lokalnej transkrypcji mowy na tekst dla Linuksa, napisane w Rust.
Projekt jest w fazie bardzo wczesnego rozwoju — zrealizowany jest dopiero **etap 0** (szkielet przechodzący),
czyli podstawowy prototyp łączący nagranie przez mikrofon, transkrypcję za pomocą whisper.cpp
(domyślnie na GPU przez Vulkan, opcjonalnie na CPU) oraz wyświetlenie wyniku w oknie GTK.
Nie jest to jeszcze gotowe narzędzie do codziennego użytku.

Dokumentacja projektowa znajduje się w katalogu [docs/README.md](docs/README.md).
Szczegóły etapu 0 w pliku [docs/00-etap-0.md](docs/00-etap-0.md).

## Wymagania

- **Rust**: wersja stabilna, zainstalowana przez [rustup](https://rustup.rs/)
- **cmake**: do budowania whisper.cpp
- **gcc-c++**: kompilator C++
- **clang-devel**: biblioteka libclang (wymagana przez bindgen do generacji bindingów whisper.cpp)
- **alsa-devel**: biblioteki ALSA (do przechwytywania dźwięku przez cpal)
- **gtk4-devel**: biblioteki GTK 4 (do interfejsu okienkowego)
- **libadwaita-devel**: (zależy od GTK)

Przykładowa instalacja na openSUSE:

```bash
sudo zypper in clang-devel gtk4-devel alsa-devel cmake gcc-c++
```

Na Debianie/Ubuntu:

```bash
sudo apt-get install libclang-dev libgtk-4-dev libasound2-dev cmake g++
```

## Model i pierwsza konfiguracja

Najprościej uruchomić kreator — pobiera model i pisze konfigurację:

```bash
diktate setup
```

Bez dialogu:

```bash
diktate model list                 # co jest do wyboru i co już masz
diktate model pull small-q5_1      # pobranie do ~/.diktate/models/
```

Ścieżka modelu jest ustalana przez pole `whisper.model` w `daemon.toml`;
gdy jest puste, o kolejności decyduje zmienna środowiskowa `DIKTATE_MODEL`
(jeśli ustawiona), a domyślnie to `~/.diktate/models/ggml-small-q5_1.bin`.

## Budowanie

```bash
cargo build --workspace
```

Zbuduje wszystkie craty projektu (rdzeń, CLI, daemon, interfejs GTK).
Binarne pliki trafią do `target/debug/`.

### Backend transkrypcji: Vulkan domyślnie, CPU opcjonalnie

`diktate-core` (i przekazujące dalej `diktate-cli`, `diktate-daemon`) ma cechę
Cargo `vulkan`, która włącza backend GPU w ggml. **Od tej wersji jest domyślnie
włączona** — zwykłe budowanie już z niej korzysta:

```bash
cargo build --workspace
```

Backend Vulkan pokrywa jednym kodem zarówno Intel Arc (iGPU, np. Meteor
Lake), jak i AMD/NVIDIA (przez Vulkan zamiast dedykowanego backendu CUDA).
Wymaga w systemie nagłówków Vulkan i kompilatora shaderów (na openSUSE:
`sudo zypper in vulkan-devel shaderc`).

Na tym sprzęcie (Intel Meteor Lake, iGPU Arc) Vulkan daje wyraźne
przyspieszenie względem CPU. Pomiar na `testdata/jfk.wav`, model `small`,
`diktate-cli record` (czas zgłoszony przez samą aplikację, bez narzutu
ładowania modelu i startu procesu):

| Backend | Czas transkrypcji |
|---------|--------------------|
| CPU     | 18,1 s             |
| Vulkan  | 3,1–3,2 s          |

Urządzenie wykryte przez whisper.cpp (stderr): `ggml_vulkan: 0 = Intel(R)
Graphics (MTL) (Intel open-source Mesa driver)`, `whisper_backend_init_gpu:
using Vulkan0 backend`.

Backend CPU jest nadal dostępny i w pełni obsługiwany:

```bash
cargo build --workspace --no-default-features
```

(albo per-crate: `cargo build -p diktate-cli --no-default-features`).

Cecha dotyczy whispera. Parakeet, drugi model z tego samego drzewa
whisper.cpp, liczy na rdzeniach **zawsze** — na tym iGPU jego enkoder jest
2,5× wolniejszy niż na CPU, czyli odwrotnie niż u whispera
([docs/12](docs/12-backendy-transkrypcji.md)).

### Skąd bierze się whisper.cpp

Silnik jest submodułem `vendor/whisper.cpp` przypiętym do wydania **v1.9.4**.
Po świeżym sklonowaniu repozytorium trzeba raz:

```bash
git submodule update --init
```

`WHISPER_CPP_DIR=/ścieżka/do/klonu` nadpisuje ten wybór, gdy ktoś ma już
zbudowane drzewo whisper.cpp i nie chce kompilować go po raz drugi.

Wiązania są własne i celowo wąskie: `crates/diktate-whispercpp-sys` buduje
whisper.cpp przez cmake i wystawia kilka funkcji operujących na wskaźnikach
i liczbach, a całą wiedzę o układzie struktur silnika trzyma warstwa
pośrednia w C++ (`src/shim.cpp`). Powód jest konkretny: `whisper_full_params`
ma 61 pól i jest przekazywana przez wartość, więc odwzorowanie jej w Ruście
psułoby się przy każdej zmianie w whisper.cpp — bez błędu kompilacji, za to
z przesuniętymi wartościami w czasie pracy.

Wcześniej rolę tę pełnił crate `whisper-rs`. Odszedł, bo wiązania rustowe
zostawały w tyle za silnikiem: najnowszy `whisper-rs-sys` wozi whisper.cpp
1.8.3, czyli bez parakeeta, a używana wersja 0.14.4 — 1.7.4. Razem z nim
zniknęły dwa obejścia: łatka `[patch.crates-io]` na `vendor/whisper-rs-0.14.4`
(jego `vulkan.rs` wołał usunięte symbole `ggml_backend_vk_*`) i
`WHISPER_DONT_GENERATE_BINDINGS=1` w `.cargo/config.toml` (bindgen 0.71
z clang 22 i glibc 2.43 źle wyliczał rozmiar `_IO_FILE`, a w konsekwencji
`whisper_full_params`). Oba były objawami tej samej choroby i żadne nie ma
już czego leczyć.

## Uruchamianie

### 1. Szybki test — transkrypcja pliku WAV

Bez mikrofonu, bez GTK, bez daemona. Służy do sprawdzenia whispera i konfiguracji modelu:

```bash
./target/debug/diktate-cli record --wav testdata/jfk.wav --language en
```

Wynik pojawi się na standardowym wyjściu.

Dostępne opcje:
- `--wav FILE`: zamiast nagrywać, wczytaj i ztranskrybuj plik WAV (mono, 16 kHz)
- `--language LANG`: język transkrypcji (domyślnie `pl`)
- `--model PATH`: ścieżka do modelu (domyślnie `$DIKTATE_MODEL` lub `~/.diktate/models/ggml-small-q5_1.bin`)

### 2. Nagranie z mikrofonu (CLI)

Nagranie przez N sekund z domyślnego urządzenia audio:

```bash
./target/debug/diktate-cli record --seconds 10
```

Transkrypcja rozpoczyna się po zatrzymaniu nagrania.

### 3. Daemon i okno GTK

Dyktowanie ciągłe: daemon nasłuchuje na gnieździe uniksowym, VAD tnie mowę na
wypowiedzi, a tekst pojawia się w oknie na bieżąco — bez czekania na Stop.

Wystarczy uruchomić okno:

```bash
./target/debug/diktate-gtk
```

Jeśli daemon nie działa, klient **uruchomi go sam** (szuka go kolejno w
`$DIKTATE_DAEMON`, w katalogu obok siebie i w `PATH`) i poczeka na gniazdo —
podtytuł pokazuje „Uruchamiam daemona…", a przycisk mikrofonu budzi się
z połączeniem. Uruchomiony w ten sposób daemon zostaje po zamknięciu okna,
więc następne uruchomienie ma model już wczytany. Wyłącza to `autostart =
false` w sekcji `[daemon]` pliku `gtk.toml`.

Daemon można też prowadzić ręcznie w osobnym terminalu — wtedy klient po
prostu zastaje gotowe gniazdo:

```bash
./target/debug/diktate-daemon            # start; odmawia, gdy inny już działa
./target/debug/diktate-daemon --restart  # zastąp działającego nowym procesem
./target/debug/diktate-daemon --stop     # zatrzymaj działającego
```

Daemon jest **jeden** na sesję użytkownika i pilnuje tego przez plik
`$XDG_RUNTIME_DIR/diktate.pid`. Zwykły start przy działającym daemonie kończy
się błędem zamiast po cichu przejąć gniazdo — inaczej starszy proces zostawał
w pamięci z nieaktualną konfiguracją i zmiany wyglądały, jakby nie działały.
Po zmianie `daemon.toml` albo po przebudowaniu binarki właściwym ruchem jest
`--restart`: zatrzymuje poprzednika (`SIGTERM`, a po trzech sekundach
`SIGKILL`) i wchodzi na jego miejsce. Nieaktualny plik PID — po `kill -9` albo
restarcie maszyny — jest rozpoznawany i sprzątany, więc nie blokuje startu.

W oknie: przycisk mikrofonu przełącza nagrywanie, wskaźnik pod paskiem
nagłówka pokazuje poziom sygnału (brak wychyleń = złe urządzenie wejściowe),
a `Kopiuj` wkłada notatkę do schowka. Tekst wchodzi **w miejsce karetki**, nie
na koniec — kliknięcie w środek notatki przenosi tam dyktowanie. `Ctrl+Z`
i `Ctrl+Shift+Z` cofają i ponawiają dyktowanie oraz pisanie jednym
mechanizmem.

### 4. Skrót globalny

Jeden skrót przywołuje okno i włącza mikrofon:

```bash
./target/debug/diktate-gtk --install-shortcut "<Control><Alt>d"   # GNOME
./target/debug/diktate-gtk --remove-shortcut
```

Poza GNOME zwiąż własnym narzędziem (`sxhkd`, Hyprland, i3, KDE) polecenie
`diktate-gtk --toggle`.

### 5. Konfiguracja

`~/.config/diktate/daemon.toml` i `~/.config/diktate/gtk.toml`. Brak plików to
wartości domyślne; literówka w nazwie pola zatrzymuje start z komunikatem.
Pełna lista pól: [docs/04-konfiguracja.md](docs/04-konfiguracja.md).

Oba pliki pisze też `diktate setup` — dialog w terminalu, z bieżącymi
wartościami jako domyślnymi i listą różnic przed zapisem. `diktate setup
--advanced` pyta o każde pole po kolei.

## Crate'y projektu

| Crate | Opis |
|-------|------|
| `diktate-core` | Rdzeń: przechwytywanie (cpal), VAD (Silero), segmentacja, filtry halucynacji, transkrypcja (whisper.cpp — whisper na Vulkanie albo Parakeet na CPU). |
| `diktate-proto` | Typy protokołu IPC (JSON-RPC po gnieździe uniksowym). |
| `diktate-document` | Model notatki: spany, kotwice, log operacji, cofanie. Bez audio, bez GTK. |
| `diktate-config` | `daemon.toml` i `gtk.toml` — wczytywanie i walidacja. |
| `diktate-daemon` | Serwer nasłuchujący na gnieździe, prowadzący sesję dyktowania. |
| `diktate-cli` | Klient terminalowy — `record` do testowania bez interfejsu, `setup` (kreator konfiguracji) i `model list/pull`. |
| `diktate-gtk` | Interfejs graficzny (GTK 4 + libadwaita): okno, tray, skrót globalny. |

## Czego jeszcze nie ma

Plan rozwoju: [docs/09-roadmapa.md](docs/09-roadmapa.md).

- **Zastępowanie przez ponowne podyktowanie** (etap 2): zaznaczenie fragmentu
  nie uzbraja jeszcze zastąpienia; model dokumentu ma pod to fundament
  (kotwice odporne na napływ wypowiedzi), brakuje operacji `Splice`
- **Historia notatek** (etap 2): `raw.ndjson`, `ops.ndjson`, `note.md`
- **Komendy głosowe i aktywacja głosem** (etap 3)
- **Wpisywanie do aktywnego okna** przez `ydotool` (etap 4)
- **Redakcja lokalnym LLM-em** (etap 5)
- **`diktate-cli`**: ma dziś `record`, `setup` i `model`; `toggle`, `status`,
  `devices`, `config check` dochodzą razem z etapem, który ich potrzebuje
- **Journal daemona** oraz sekcje `[filters]`, `[log]`, `[ipc]` w konfiguracji

## Test IPC

Skrypt `scripts/smoke-ipc.py` testuje protokół IPC daemona
(wysyła `session.start`, czeka, wysyła `session.stop`, oczekuje na `segment.added`).
Wymaga uruchomionego daemona:

```bash
python3 scripts/smoke-ipc.py
```
