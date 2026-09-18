# 08 — Dystrybucja i testy

## Odbiorca

Linux, na własny użytek i dla kilku znajomych. To ustalenie ma konkretne
konsekwencje: **bez Flatpaka**.

Sandbox Flatpaka kolidowałby z trzema rzeczami naraz: dostępem do NPU przez
OpenVINO, `ydotool` (wymaga `/dev/uinput`), oraz wykonywaniem dowolnych
poleceń przez `diktate-agent`. Każdą z nich dałoby się obejść uprawnieniami
albo portalami, ale koszt przewyższa zysk przy tym odbiorcy.

Architektura nie zamyka drogi do Flatpaka później — warstwa integracji
z systemem jest oddzielona od rdzenia — ale nie płacimy za to teraz.

## Budowanie

```bash
cargo build --release                        # Vulkan, domyślnie
cargo build --release --features openvino    # + enkoder na NPU Intela
cargo build --release --features rocm        # + ROCm dla AMD
cargo build --release --no-default-features  # tylko CPU
```

whisper.cpp jest kompilowany przez `whisper-rs` w ramach builda, więc flagi
CMake są pod kontrolą Cargo. Wymagane do budowy: `cmake`, `clang`, nagłówki
Vulkan (`vulkan-headers`, `shaderc`), GTK4 i libadwaita (tylko dla
`diktate-gtk`).

`diktate-daemon`, `diktate-cli` i `diktate-agent` **nie zależą od GTK** —
budują się i działają na maszynie bez serwera wyświetlania.

## Artefakty

| Artefakt | Zawartość |
|----------|-----------|
| `diktate-x86_64-linux.tar.zst` | Cztery binarki, `.desktop`, ikony, przykładowe TOML-e, `install.sh` |
| Pakiet RPM | To samo, z jednostkami systemd użytkownika i zależnościami |
| Pakiet DEB | Jak wyżej |

Modele **nie są pakowane** — `large-v3` waży ponad 3 GB. Pobierane przy
pierwszym uruchomieniu przez `diktate-cli model pull`.

`install.sh` wykonuje: instalację binarek do `~/.local/bin`, plików
`.desktop` i ikon do `~/.local/share`, przykładowych konfiguracji (bez
nadpisywania istniejących), jednostek systemd, opcjonalnie rejestrację skrótu
globalnego, oraz kontrolę obecności rozszerzenia AppIndicator.

## Uruchamianie

Jednostki systemd użytkownika (`~/.config/systemd/user/`):

```
diktate-daemon.service   # rezydentny, WantedBy=default.target
diktate-agent.service    # opcjonalny, After=diktate-daemon.service
```

`diktate-gtk` uruchamiany z autostartu XDG albo ręcznie — nie jest usługą,
bo jego cyklem życia zarządza sesja graficzna.

Kolejność jest istotna: daemon startuje niezależnie od sesji graficznej, więc
skrót klawiaturowy i agent działają nawet wtedy, gdy okno GTK nigdy nie zostało
otwarte.

## Aktualizacje

Bez automatycznych aktualizacji. `diktate-cli version --check` sprawdza wydania
na GitHubie i wypisuje informację. Przy tym odbiorcy mechanizm samoaktualizacji
byłby kodem o wyższym ryzyku niż wartości.

## Wersjonowanie

SemVer dla wydań. Protokół IPC ma **osobny numer wersji** — daemon odrzuca
klienta o niekompatybilnej wersji głównej z jawnym komunikatem, zamiast
zawodzić w sposób trudny do zdiagnozowania. Numer protokołu rośnie wolniej niż
numer aplikacji.

## Strategia testów

### Testy jednostkowe

| Crate | Co jest testowane | Czego nie potrzeba |
|-------|-------------------|---------------------|
| `diktate-document` | Spany, splice, kotwice, undo, testy własnościowe | Audio, GTK, daemon |
| `diktate-core` | Maszyna stanów VAD, filtry, kolejka, ładowanie konfiguracji | Mikrofon, GPU |
| `diktate-proto` | Serializacja, zgodność wersji | — |
| `diktate-agent` | Dopasowywanie reguł, podstawianie parametrów, tryb dry-run | Mikrofon |

`diktate-document` jest w całości testowalny bez wejścia-wyjścia. Testy
własnościowe (`proptest`) na sekwencjach losowych operacji weryfikują, że
`fold(ops)` daje stabilny wynik i że kotwice nigdy nie stają się nieważne przez
samo wstawienie nowego segmentu — także wtedy, gdy wstawienie wypada wewnątrz
spanu i tnie go na pół
([03](03-model-dokumentu.md#kotwice--dlaczego-to-jest-spójne)).

### Testy z nagraniami

Katalog `testdata/` z krótkimi plikami WAV i oczekiwanym wyjściem. Pipeline
przyjmuje wejście z pliku zamiast z mikrofonu (`--input-file`), co czyni
całą ścieżkę audio → VAD → segmenty deterministyczną i testowalną w CI.

Przypadki obowiązkowe: mowa ciągła bez przerw (twarde cięcie na 30 s), mowa
z długimi przerwami, cisza i szum (filtr halucynacji), mowa zaczynająca się
natychmiast (pre-roll), mowa w dwóch językach, wypowiedź dłuższa niż
`long_segment_ms` z krótką pauzą w środku (adaptacyjny próg ciszy tnie ją
tam, gdzie zwykły próg by nie ciął).

Same transkrypcje nie są porównywane dosłownie — model bywa
niedeterministyczny między wersjami. Sprawdzana jest liczba segmentów, ich
granice czasowe i podobieństwo tekstu powyżej progu.

### Testy integracyjne

Daemon uruchamiany z `--input-file` i tymczasowym gniazdem, klient testowy
łączy się i weryfikuje sekwencję zdarzeń. Pokrywa: `subscribe` w trakcie sesji
z `since_seq`, rozłączenie i ponowne połączenie bez utraty segmentów, odmowę
przy niezgodnej wersji protokołu, przełączenie modelu w locie.

### Czego nie testujemy automatycznie

Jakości transkrypcji jako takiej, zachowania tray-a w różnych środowiskach,
wydajności GPU. To obszary weryfikowane ręcznie, na liście kontrolnej przed
wydaniem.

## CI

GitHub Actions: `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test`
z backendem CPU (runnerzy nie mają GPU), oraz build wszystkich kombinacji
feature flag bez uruchamiania. Budowa `diktate-gtk` wymaga zainstalowania GTK4
w obrazie.
