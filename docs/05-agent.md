# 05 — `diktate-agent`

## Rola

Niezależny proces, który subskrybuje strumień segmentów z daemona, dopasowuje
je do reguł i wykonuje akcje. Nie wymaga GUI ani serwera wyświetlania —
działa w tiling WM, w tmuksie i na maszynie headless.

To bezpośrednia konsekwencja „głupiego" daemona: skoro segmenty są rozgłaszane
do wszystkich subskrybentów, komendy głosowe nie muszą być częścią interfejsu
graficznego.

```
daemon ──segment.added──┬──► diktate-gtk    (dokument, okno)
                        └──► diktate-agent  (reguły, akcje)
```

## Format reguły

```toml
[agent]
enabled = false               # domyślnie WYŁĄCZONY — patrz Bezpieczeństwo
default_keyword = "zdzisław"
strip_keyword = true          # usuń keyword z tekstu przed dopasowaniem wzorca
notify_on_action = true       # powiadomienie systemowe po wykonaniu

[[rule]]
name = "zgaś światło"
keyword = "zdzisław"          # pominięte = default_keyword
match = "contains"            # "contains" | "regex" | "fuzzy" | "exact"
pattern = "zgaś światło"
action = { type = "exec", cmd = ["/home/msobczak/bin/lights", "off"] }
consume = true

[[rule]]
name = "nowy terminal"
match = "fuzzy"
pattern = "uruchom nowe okno terminala"
fuzzy_threshold = 0.75
action = { type = "exec", cmd = ["ptyxis", "--new-window"] }
consume = true

[[rule]]
name = "zatrzymaj nagrywanie"
match = "fuzzy"
pattern = "stop"
action = { type = "ipc", method = "session.stop" }
consume = true

[[rule]]
name = "wyłącz komputer"
match = "contains"
pattern = "wyłącz komputer"
action = { type = "shell", script = "systemctl poweroff" }
confirm = true                # powiadomienie z potwierdzeniem przed wykonaniem
consume = true
```

## Dopasowywanie

### Pozycja słowa kluczowego jest dowolna

Wymaganie wynika wprost ze scenariuszy użycia: „zdzisław, zgaś światło"
i „możesz mi uruchomić nowe okno terminala zdzisław" muszą działać tak samo.

Dlatego keyword jest **bramką na całym segmencie**, nie prefiksem. Algorytm:

1. Segment nie zawiera keyworda → przekaż dalej jako zwykły tekst, koniec.
2. Usuń keyword z tekstu (jeśli `strip_keyword`), znormalizuj: małe litery,
   usuń interpunkcję i zbędne spacje, opcjonalnie usuń polskie znaki
   diakrytyczne (`normalize_diacritics`).
3. Dopasuj do wzorców reguł w kolejności ich wystąpienia w pliku.
4. Pierwsze trafienie wygrywa. Wykonaj akcję.
5. Jeśli `consume = true` — segment nie jest przekazywany dalej jako tekst.

### Dopasowanie musi być rozmyte

Whisper nie odda tego samego zdania identycznie dwa razy — różnice
w interpunkcji, odmianie i drobnych słowach są normą. Dlatego `fuzzy`
(podobieństwo Jaro-Winklera albo znormalizowana odległość Levenshteina, próg
domyślnie 0.75) jest zalecanym trybem, a `contains` i `exact` pozostają dla
krótkich, jednoznacznych fraz.

### Wzorce z parametrami

```toml
[[rule]]
name = "ustaw głośność"
match = "regex"
pattern = "ustaw głośność na (?P<poziom>\\d+)"
action = { type = "exec", cmd = ["wpctl", "set-volume", "@DEFAULT_SINK@", "${poziom}%"] }
```

Nazwane grupy z `regex` są dostępne jako `${nazwa}` w `cmd` i w `script`.
Podstawienie następuje **do argumentów tablicy**, nigdy przez konkatenację
łańcucha powłoki — to eliminuje wstrzyknięcie polecenia przez treść
wypowiedzi. Reguły `type = "shell"` tej ochrony nie mają i dlatego wymagają
świadomego użycia.

## Typy akcji

| Typ | Opis |
|-----|------|
| `exec` | Uruchom program z tablicą argumentów. Bez powłoki. Zalecane |
| `shell` | Wykonaj skrypt przez `sh -c`. Wygodne, ale podatne na wstrzyknięcie |
| `ipc` | Wywołaj metodę daemona (`session.stop`, `profile.set`, …) |
| `notify` | Tylko powiadomienie systemowe — przydatne przy strojeniu reguł |
| `http` | Żądanie HTTP (automatyka domowa, webhooki) |

Każda akcja ma opcjonalne `timeout_ms` (domyślnie 10000), `cwd` i `env`.
Przekroczenie czasu kończy proces i loguje ostrzeżenie.

## Bezpieczeństwo

Agent wykonuje dowolne polecenia na podstawie tego, co usłyszał mikrofon.
To nie jest funkcja wygody, tylko powierzchnia ataku i źródło wypadków —
telewizor w tle albo rozmowa obok mogą wyzwolić regułę.

Dlatego:

- **Domyślnie wyłączony.** `agent.enabled = false` w świeżej instalacji.
  Włączenie jest świadomą decyzją użytkownika.
- **`confirm = true`** na regule wymusza powiadomienie z przyciskiem
  potwierdzenia przed wykonaniem. Zalecane dla wszystkiego destrukcyjnego.
- **Tryb `--dry-run`** loguje dopasowania i polecenia, których *nie* wykonał.
  To podstawowe narzędzie strojenia progów rozmycia.
- **Prawa pliku.** `agent.toml` z prawami innymi niż `0600` powoduje
  ostrzeżenie przy starcie — plik zawiera polecenia wykonywane na koncie
  użytkownika.
- **Bez eskalacji.** Agent nigdy nie uruchamia się z podwyższonymi
  uprawnieniami i nie wywołuje `sudo` sam z siebie. Jeśli reguła tego
  potrzebuje, użytkownik konfiguruje `polkit` albo `sudoers` świadomie.
- **Log audytu.** Każde wykonanie zapisywane do
  `$XDG_STATE_HOME/diktate/agent-audit.log`: czas, tekst segmentu, dopasowana
  reguła, polecenie, kod wyjścia.

## Konfiguracja z poziomu GUI

Okno ustawień `diktate-gtk` dostaje zakładkę „Akcje głosowe": lista reguł,
edytor pojedynczej reguły, przycisk testu („powiedz to teraz i zobacz, czy
trafia" — dopasowanie na żywo bez wykonywania akcji).

Mechanizm jest celowo prymitywny: **GUI zapisuje `agent.toml`, agent go
przeładowuje na inotify**. Żadnego IPC między GUI a agentem.

Konsekwencje, dla których ta droga została wybrana:

- GUI działa identycznie niezależnie od tego, czy agent jest uruchomiony
- agent działa identycznie niezależnie od tego, czy GUI kiedykolwiek istniało
- konfigurację da się edytować `vim`em, wersjonować w git i kopiować między
  maszynami
- nie ma dwóch źródeł prawdy, które mogłyby się rozjechać

GUI musi zachować komentarze i kolejność sekcji przy zapisie (parser TOML
z zachowaniem formatowania — `toml_edit`). Plik jest własnością użytkownika,
nie aplikacji.

## Kolejność wdrożenia

Agent nie należy do pierwszej wersji — patrz [09-roadmapa.md](09-roadmapa.md).
Jest jednak projektowany od początku, bo jego istnienie jest powodem, dla
którego daemon rozgłasza segmenty do wielu subskrybentów zamiast obsługiwać
jednego klienta.
