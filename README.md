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

Transkrypcja wymaga modelu whisper. Należy pobrać model `ggml-base.bin` do katalogu `~/models/whisper/`:

```bash
mkdir -p ~/models/whisper
curl -L -o ~/models/whisper/ggml-base.bin \
  https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.bin
```

Ścieżka modelu jest ustalana przez zmienną środowiskową `DIKTATE_MODEL` (jeśli ustawiona)
lub domyślnie szukana w `~/models/whisper/ggml-base.bin`.

## Budowanie

```bash
cargo build --workspace
```

Zbuduje wszystkie craty projektu (rdzeń, CLI, daemon, interfejs GTK).
Binarne pliki trafią do `target/debug/`.

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
- `--model PATH`: ścieżka do modelu (domyślnie `$DIKTATE_MODEL` lub `~/models/whisper/ggml-base.bin`)

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
- **Przyspieszenie GPU** (Vulkan): transkrypcja tylko na CPU

## Test IPC

Skrypt `scripts/smoke-ipc.py` testuje protokół IPC daemona
(wysyła `session.start`, czeka, wysyła `session.stop`, oczekuje na `segment.final`).
Wymaga uruchomionego daemona:

```bash
python3 scripts/smoke-ipc.py
```
