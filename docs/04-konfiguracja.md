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

Każdy proces waliduje swój plik przy starcie (błąd walidacji = odmowa startu
z czytelnym komunikatem, nie ciche wartości domyślne) i przeładowuje go na
`inotify`. Zmiana progu VAD działa bez restartu daemona.

Zapis pliku przez jakiekolwiek narzędzie musi być **atomowy**: zapis do pliku
tymczasowego w tym samym katalogu, potem `rename`. Inaczej inotify potrafi
złapać plik w połowie zapisu.

## `daemon.toml`

```toml
[audio]
device = "default"            # nazwa z `diktate-cli devices`
sample_rate = 16000           # natywny dla whispera, nie zmieniać bez powodu
channels = 1
ring_buffer_secs = 30
pre_roll_ms = 300             # audio sprzed detekcji mowy dołączane do segmentu

[vad]
engine = "silero"             # "silero" | "energy" | "off"
model_path = ""               # puste = wbudowany silero_vad.onnx
threshold = 0.5               # 0.0–1.0
speech_frames_to_start = 3    # ramek 30 ms potwierdzających początek mowy
silence_ms_to_end = 600       # cisza zamykająca segment
min_segment_ms = 300          # krótsze odrzucane jako trzask
max_segment_ms = 30000        # twarde cięcie (okno whispera to 30 s)
frame_ms = 30

[whisper]
model = "large-v3-turbo"
backend = "vulkan"            # "vulkan" | "openvino" | "rocm" | "cpu"
gpu_device = 0
fallback_to_cpu = true        # przy niepowodzeniu — zawsze z ostrzeżeniem
threads = 0                   # 0 = liczba rdzeni fizycznych
language = "pl"               # kod ISO albo "auto"
initial_prompt = ""
temperature = 0.0
beam_size = 5
best_of = 5
suppress_blank = true
translate = false

[whisper.context]
enabled = true                # doklejanie ogona poprzedniego tekstu do promptu
chars = 200
repetition_guard = true
repetition_threshold = 0.85   # powyżej → ponowna transkrypcja bez kontekstu

[queue]
max_pending_segments = 8
on_overflow = "block"         # "block" | "drop_oldest"
warn_backlog_at = 3           # od tylu segmentów klient dostaje ostrzeżenie

[filters]
drop_patterns = [
  "Napisy stworzone przez.*",
  "Napisy: .*",
  "^[\\s.,!?…]*$",
]
drop_if_no_speech_prob_above = 0.6
drop_if_avg_logprob_below = -1.0

[journal]
enabled = true
dir = ""                      # puste = $XDG_STATE_HOME/diktate/journal
retention_days = 30

[wakeword]
enabled = false               # stały nasłuch mikrofonu — świadoma decyzja
model = "tiny"                # model czuwania, osobny od [whisper].model
language = "pl"
match_threshold = 0.85        # wyższy niż progi agenta
normalize_diacritics = true
confirm_with_main_model = true
confirm_below_logprob = -0.7
dictation_phrases = ["dyktuj", "notuj", "zapisz notatkę"]
agent_keywords = ["zdzisław"]
unload_main_model_after_idle_secs = 0   # 0 = trzymaj model główny w pamięci

[wakeword.limits]
max_utterance_ms = 6000       # dłuższe wypowiedzi nie są transkrybowane w czuwaniu
cooldown_ms = 1500            # blokada po trafieniu

[ipc]
socket_path = ""              # puste = $XDG_RUNTIME_DIR/diktate.sock
max_clients = 8

[log]
level = "info"                # "trace" | "debug" | "info" | "warn" | "error"
file = ""                     # puste = stderr / journald
```

### Profile językowe

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
always_on_top = true
width = 620
height = 420
position = "center"           # "center" | "cursor" | "remember"
close_on_copy = true
show_level_meter = true
show_confidence = true        # segmenty o niskiej pewności innym kolorem

[replace]
mode = "one_utterance"        # "one_utterance" | "until_confirm"
highlight_color = "#f6d32d"

[shortcuts]
copy = "<Control>c"
copy_and_close = "<Control>Return"
redictate = "<Control>r"
undo = "<Control>z"
redo = "<Control><Shift>z"
save = "<Control>s"
discard = "Escape"
toggle_recording = "<Control>space"   # wewnątrz okna, nie globalny

[history]
enabled = true
dir = ""                      # puste = $XDG_DATA_HOME/diktate/notes
write_markdown = true
autosave_on_close = true

[tray]
enabled = true
icon_idle = "diktate-symbolic"
icon_recording = "diktate-recording-symbolic"
show_backlog = true
```

Skrót globalny (uruchamiający nagrywanie spoza okna) **nie jest tutaj** —
rejestruje go środowisko graficzne. Patrz [06-gui.md](06-gui.md#skrót-globalny).

## `agent.toml`

Opisany w całości w [05-agent.md](05-agent.md).

Sekcja `[wakeword]` powyżej jest opisana w [10-wake-word.md](10-wake-word.md).

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
