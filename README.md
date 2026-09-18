# Diktate

Diktate to narzędzie do lokalnej transkrypcji mowy na tekst dla Linuksa, napisane w Rust.
Projekt jest w fazie bardzo wczesnego rozwoju — zrealizowany jest dopiero **etap 0** (szkielet przechodzący),
czyli podstawowy prototyp łączący nagranie przez mikrofon, transkrypcję za pomocą whisper.cpp
na CPU oraz wyświetlenie wyniku w oknie GTK. Nie jest to jeszcze gotowe narzędzie do codziennego użytku.

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

## Model

Transkrypcja wymaga modelu whisper. Należy pobrać model `ggml-small.bin` do katalogu `~/models/whisper/`:

```bash
mkdir -p ~/models/whisper
curl -L -o ~/models/whisper/ggml-small.bin \
  https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.bin
```

Ścieżka modelu jest ustalana przez zmienną środowiskową `DIKTATE_MODEL` (jeśli ustawiona)
lub domyślnie szukana w `~/models/whisper/ggml-small.bin`.

## Budowanie

```bash
cargo build --workspace
```

Zbuduje wszystkie craty projektu (rdzeń, CLI, daemon, interfejs GTK).
Binarne pliki trafią do `target/debug/`.

### Backend transkrypcji: CPU domyślnie, Vulkan opcjonalnie

`diktate-core` (i przekazujące dalej `diktate-cli`, `diktate-daemon`) ma cechę
Cargo `vulkan`, która włącza backend GPU whisper.cpp przez `whisper-rs/vulkan`.
**Domyślnie jest wyłączona** — budowanie idzie na CPU bez dodatkowych flag:

```bash
cargo build --workspace
```

Backend Vulkan miał docelowo pokryć jednym kodem zarówno Intel Arc (iGPU, np.
Meteor Lake), jak i AMD. Próba włączenia:

```bash
cargo build -p diktate-cli --features vulkan
```

wymaga w systemie nagłówków Vulkan i kompilatora shaderów (na openSUSE:
`sudo zypper in vulkan-devel shaderc`) — na tym sprzęcie te pakiety są
zainstalowane i CMake poprawnie buduje `libggml-vulkan.a` whisper.cpp. Mimo
to build **obecnie się nie kompiluje**: to repo ma w `.cargo/config.toml`
ustawione `WHISPER_DONT_GENERATE_BINDINGS=1` (obejście niepowiązanego błędu
bindgena z rozmiarami struktur glibc), przez co crate `whisper-rs-sys` zawsze
używa dołączonych do paczki, gotowych bindingów FFI zamiast generować je na
nowo dla aktywnych cech. Te gotowe bindingi nie zawierają deklaracji
`ggml_backend_vk_*`, więc `whisper-rs` 0.14.4 (moduł `vulkan.rs`) pada błędem
`E0432 unresolved imports`. Dopóki to nie zostanie naprawione wyżej w
łańcuchu (regeneracja bindingów whisper-rs-sys albo poprawka w whisper-rs),
backendem domyślnym i jedynym używanym w praktyce pozostaje CPU.

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
- `--model PATH`: ścieżka do modelu (domyślnie `$DIKTATE_MODEL` lub `~/models/whisper/ggml-small.bin`)

### 2. Nagranie z mikrofonu (CLI)

Nagranie przez N sekund z domyślnego urządzenia audio:

```bash
./target/debug/diktate-cli record --seconds 10
```

Transkrypcja rozpoczyna się po zatrzymaniu nagrania.

### 3. Daemon i okno GTK

Etap 0b — interaktywne dyktowanie: daemon nasłuchuje na gnieździe uniksowym,
okno GTK komunikuje się z daemonem przez IPC.

W pierwszym terminalu uruchom daemon:

```bash
./target/debug/diktate-daemon
```

W drugim terminalu uruchom interfejs:

```bash
./target/debug/diktate-gtk
```

Okno GTK będzie wyświetlać przycisk „Nagrywaj" (przełączający nagrywanie),
pole tekstowe na rozpoznany tekst i przycisk „Kopiuj".
Po zatrzymaniu nagrania (ponowne kliknięcie przycisku) tekst pojawi się w oknie.

## Crate'y projektu

| Crate | Opis |
|-------|------|
| `diktate-core` | Rdzeń: przechwytywanie dźwięku (cpal) i transkrypcja (whisper-rs na CPU). |
| `diktate-proto` | Typy protokołu IPC (JSON-RPC po gnieździe uniksowym). |
| `diktate-daemon` | Serwer nasłuchujący na gnieździe, wykonujący nagrywanie i transkrypcję. |
| `diktate-cli` | Klient terminalowy — komenda `record` do testowania bez interfejsu. |
| `diktate-gtk` | Interfejs graficzny (GTK 4) — okno dyktowania z przyciskami. |

## Czego jeszcze nie ma

Etap 0 jest szkieletem — wiele funkcji zaplanowanych na kolejne etapy.
Lista pominięć w etapie 0 oraz plan rozwoju w [docs/09-roadmapa.md](docs/09-roadmapa.md).

Brakujące funkcje to między innymi:

- **VAD** (Voice Activity Detection): koniec nagrania wyznacza przycisk Stop
- **Transkrypcja strumieniowa**: tekst pojawia się podczas nagrania, nie po zatrzymaniu
- **Filtry halucynacji**: brak post-przetwarzania tekstu
- **Model dokumentu**: brak kotwic, `Ctrl+Z`, zamieniania fragmentów
- **Konfiguracja**: stałe w kodzie zamiast plików TOML
- **Tray i skrót globalny**: interfejs okienkowy tylko w oknie, bez integracji z pulpitem
- **Przyspieszenie GPU** (Vulkan): cecha Cargo `vulkan` istnieje, ale obecnie
  się nie kompiluje (patrz sekcja „Backend transkrypcji" wyżej) — w praktyce
  transkrypcja tylko na CPU

## Test IPC

Skrypt `scripts/smoke-ipc.py` testuje protokół IPC daemona
(wysyła `session.start`, czeka, wysyła `session.stop`, oczekuje na `segment.final`).
Wymaga uruchomionego daemona:

```bash
python3 scripts/smoke-ipc.py
```
