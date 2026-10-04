# Diktate

Local, offline speech-to-text dictation for the Linux desktop.

You talk, the text shows up in a small GNOME window while you are still
talking, and one shortcut copies it to the clipboard. Everything runs on your
own machine — audio never leaves it. Transcription is done by
[whisper.cpp](https://github.com/ggml-org/whisper.cpp), on the GPU through
Vulkan by default (Intel, AMD and NVIDIA alike) or on the CPU.

> **Status: early development.** Diktate is usable day to day, but it is a
> personal project: the user interface, messages and design documents
> ([`docs/`](docs/)) are in **Polish**, and the default transcription
> language is Polish. Any language Whisper supports can be set in the
> configuration.

## Features

- **Continuous dictation.** A [Silero VAD](https://github.com/snakers4/silero-vad)
  cuts speech into utterances at natural pauses and each one is transcribed as
  soon as it ends — text appears while you speak, not after you press Stop.
- **Edit while you dictate.** Text goes in at the caret, not at the end:
  click into the middle of the note and keep talking. Undo and redo cover
  dictation and typing alike.
- **GPU or CPU.** whisper.cpp with the Vulkan backend (default) or a pure CPU
  build. An optional [Parakeet](https://huggingface.co/nvidia/parakeet-tdt-0.6b-v3)
  backend runs on the CPU.
- **Context-aware.** The end of your note can be passed to Whisper as a prompt,
  which helps with names and terminology.
- **Optional LLM post-editing.** Fix transcription errors, or rewrite the text
  with a spoken instruction, through any OpenAI-compatible chat endpoint
  (for example a local `llama-server`). Edits that change too much are shown
  for approval first, and every edit can be undone.
- **Native GNOME look.** libadwaita window that follows the system theme,
  a global shortcut registered with one command, a tray icon.

## How it works

```
diktate-gtk  ───── JSON-RPC over a Unix socket ─────▶  diktate-daemon
 window,              $XDG_RUNTIME_DIR/diktate.sock      microphone → VAD → whisper.cpp
 shortcut,   ◀──── text segments, input level, state ──  model loaded once, kept warm
 tray
```

- **`diktate-daemon`** owns the microphone and the model. There is one per user
  session (guarded by `$XDG_RUNTIME_DIR/diktate.pid`), and it stays running
  between dictations so the model does not have to be loaded again.
- **`diktate-gtk`** is the window. If no daemon is running, it starts one
  itself and waits for it.
- **`diktate-cli`** is the terminal tool: setup wizard, model downloads, and
  a one-shot `record` command for testing without the GUI.

## Requirements

- Linux with GNOME, on Wayland or X11. Other desktops work too — you only bind
  the global shortcut yourself.
- Rust stable, installed with [rustup](https://rustup.rs/).
- A C/C++ toolchain, CMake and pkg-config — whisper.cpp is compiled from source.
- `perl` and `make` — one build dependency compiles a vendored OpenSSL.
- Development files for GTK 4, libadwaita ≥ 1.5 and ALSA.
- For the default **Vulkan** build: Vulkan headers and loader, the `glslc`
  shader compiler, SPIRV-Headers, and a working Vulkan driver (Mesa for Intel
  and AMD, the proprietary driver for NVIDIA). `vulkaninfo --summary` should
  list your GPU.
- Network access during the first build: Cargo crates, the whisper.cpp
  submodule, and the ONNX Runtime library used by the voice activity detector.

**openSUSE Tumbleweed** (the platform Diktate is developed on):

```bash
sudo zypper in git cmake gcc-c++ pkgconf-pkg-config perl make \
    gtk4-devel libadwaita-devel alsa-devel \
    vulkan-devel shaderc spirv-headers
```

**Fedora:**

```bash
sudo dnf install git cmake gcc-c++ pkgconf-pkg-config perl make \
    gtk4-devel libadwaita-devel alsa-lib-devel \
    vulkan-loader-devel vulkan-headers glslc spirv-headers-devel
```

**Debian 13 / Ubuntu 24.04 or newer:**

```bash
sudo apt install git build-essential cmake pkg-config perl \
    libgtk-4-dev libadwaita-1-dev libasound2-dev \
    libvulkan-dev glslc spirv-headers
```

Only the openSUSE list is tested; the others are best effort. For a CPU-only
build you can leave out the Vulkan packages.

## Building

```bash
git clone --recurse-submodules https://github.com/zdzisiekcom/diktate.git
cd diktate
cargo build --release --workspace
```

The engine lives in the `vendor/whisper.cpp` submodule, pinned to release
v1.9.4. If you cloned without `--recurse-submodules`, fetch it first:

```bash
git submodule update --init
```

The first build compiles whisper.cpp and, with Vulkan, its compute shaders —
expect several minutes. whisper.cpp is always compiled with optimisations,
so even a debug build of Diktate transcribes at full speed.

CPU-only variant, no Vulkan needed:

```bash
cargo build --release --workspace --no-default-features
```

To build against your own whisper.cpp checkout (v1.9.4 or newer) instead of
the submodule, set `WHISPER_CPP_DIR=/path/to/whisper.cpp`.

### Installing

The binaries link whisper.cpp statically, so installing means copying three
files:

```bash
install -Dm755 target/release/diktate-{cli,daemon,gtk} -t ~/.local/bin/
```

Make sure `~/.local/bin` is on your `PATH`, and keep `diktate-gtk` and
`diktate-daemon` in the same directory — the window looks for the daemon in
`$DIKTATE_DAEMON`, then next to its own binary, then on `PATH`.

## First run

### 1. Get a model

The setup wizard downloads a model and writes the configuration:

```bash
diktate-cli setup
```

It asks for the model, language, microphone and dictation rhythm, shows a diff
before writing anything, and uses your current values as defaults when you run
it again. `diktate-cli setup --advanced` goes through every option.

Or download a model directly:

```bash
diktate-cli model list               # available models; marks the ones you have
diktate-cli model pull small-q5_1    # saved to ~/.diktate/models/
```

| Model                 | Size   | Notes                                                              |
|-----------------------|--------|--------------------------------------------------------------------|
| `base-q5_1`           | 60 MB  | fastest, noticeably weaker — mixes up words                         |
| `small-q5_1`          | 190 MB | **recommended** — good quality, fast                                |
| `small`               | 488 MB | same quality as `small-q5_1`, slower and 2.5× bigger                |
| `large-v3-turbo-q5_0` | 574 MB | best quality, but on an integrated GPU too slow for continuous dictation |

Models are downloaded from the
[`ggerganov/whisper.cpp`](https://huggingface.co/ggerganov/whisper.cpp)
repository on Hugging Face and verified by SHA-256. Quantized (`q5`) models
are not slower on Vulkan, so they are the default choice. Any other ggml
Whisper model works as well — point `whisper.model` in `daemon.toml` at it.

The daemon picks the model in this order: `whisper.model` in `daemon.toml`,
then the `DIKTATE_MODEL` environment variable, then
`~/.diktate/models/ggml-small-q5_1.bin`.

### 2. Check that transcription works

No microphone, GUI or daemon needed:

```bash
diktate-cli record --wav testdata/jfk.wav --language en
```

The transcript is printed to standard output. Without `--wav`,
`diktate-cli record --seconds 10` records from the default microphone instead.

### 3. Dictate

```bash
diktate-gtk
```

Press the microphone button (or the global shortcut, see below) and talk.
The bar under the header shows the input level — if it does not move, the
wrong input device is selected. The very first start takes a moment while the
daemon loads the model; after that the daemon stays in the background and
later starts are instant.

When you are done, press **Super+C**: the note is copied to the clipboard,
recording stops and the window closes. Paste the text wherever you need it.
(Typing straight into the focused application is planned, not implemented.)

Keyboard shortcuts inside the window:

| Shortcut                  | Action                                                          |
|---------------------------|-----------------------------------------------------------------|
| `Super+C`                 | Copy the note, stop recording, close the window                 |
| `Ctrl+Z` / `Ctrl+Shift+Z` | Undo / redo — dictation and typing alike                        |
| `Ctrl+,`                  | Settings: Whisper context prompt, LLM correction                |
| `Super+D`                 | Correct the text with an LLM (if configured)                    |
| `Super+E`                 | Rewrite the selection or the whole note with a spoken instruction (LLM) |
| `Escape`                  | Cancel the current selection mode or close a dialog             |

## Global shortcut in GNOME

On Wayland an application cannot grab a global hotkey by itself, so Diktate
registers a GNOME **custom keyboard shortcut** that runs
`diktate-gtk --toggle`. The first press opens the window and starts
recording; each next press toggles recording and brings the window to the
front. Only one window ever exists — a second `diktate-gtk` just talks to the
first one.

Install the binaries first (see [Installing](#installing)), then register the
shortcut **from the installed binary** — the shortcut stores the absolute path
of the binary that created it, so registering from `target/` would break on
the next `cargo clean`:

```bash
~/.local/bin/diktate-gtk --install-shortcut                    # Ctrl+Alt+D
~/.local/bin/diktate-gtk --install-shortcut "<Super><Alt>d"    # or any other keys
```

Running it again replaces the existing entry instead of adding a second one.
To remove it:

```bash
~/.local/bin/diktate-gtk --remove-shortcut
```

The key uses GTK accelerator syntax: modifiers `<Control>`, `<Alt>`,
`<Shift>`, `<Super>` followed by a key name, e.g. `<Control><Alt>d` or
`<Super>F9`. Pick something that does not collide with your existing
shortcuts.

### Doing it by hand

The same thing through GNOME Settings: **Settings → Keyboard → View and
Customize Shortcuts → Custom Shortcuts → Add Shortcut**, then:

- **Name:** `Diktate`
- **Command:** `/home/YOUR_USER/.local/bin/diktate-gtk --toggle` — use the full
  path, GNOME Shell's `PATH` may not include `~/.local/bin`
- **Shortcut:** press the keys you want

The entry created by `--install-shortcut` shows up in the same list as
"Diktate — dyktowanie", and its key can be changed there too.

### Other desktops

Bind the command `diktate-gtk --toggle` with your desktop's own tool —
KDE System Settings → Shortcuts, `bindsym` in sway or i3, `bind` in Hyprland,
sxhkd, and so on.

### Tray icon

The tray icon is a StatusNotifierItem. GNOME shows those only with the
[AppIndicator and KStatusNotifierItem Support](https://extensions.gnome.org/extension/615/appindicator-support/)
extension (preinstalled on Ubuntu). Without it Diktate works the same, just
without the icon; `[tray] enabled = false` in `gtk.toml` turns it off.

## Configuration

Two TOML files in `~/.config/diktate/` (or `$XDG_CONFIG_HOME/diktate/`):

| File          | Read by          | Contains                                                         |
|---------------|------------------|------------------------------------------------------------------|
| `daemon.toml` | `diktate-daemon` | microphone, voice activity detection, transcription backend, model, language |
| `gtk.toml`    | `diktate-gtk`    | window, daemon autostart, dictation behaviour, tray, Whisper context, LLM correction |

A missing file or field means the default value. Unknown fields are rejected
with an error message rather than silently ignored, so a typo is caught at
start-up. `diktate-cli setup` writes both files; the full reference with all
defaults is [docs/04-konfiguracja.md](docs/04-konfiguracja.md) (in Polish).

The `daemon.toml` options you are most likely to change:

```toml
[audio]
device = "default"        # input device; the setup wizard lists the available ones

[whisper]
model = ""                # empty = $DIKTATE_MODEL, then ~/.diktate/models/ggml-small-q5_1.bin
language = "pl"           # ISO code ("en", "de", …) or "auto"
initial_prompt = ""       # names and terms to bias recognition towards

[vad]
long_segment_ms = 2500    # once you have talked this long, a 200 ms pause already
                          # pushes the text out — this sets how often text appears
```

The daemon re-reads `daemon.toml` every time recording starts, so most changes
apply to the next dictation. The model, however, is loaded only when the
daemon starts — after changing the model or backend, or after installing new
binaries, restart it:

```bash
diktate-daemon --restart   # replace the running daemon with a fresh one
diktate-daemon --stop      # stop it
diktate-daemon             # run it in the foreground and watch its log
```

A plain `diktate-daemon` refuses to start while another one is running —
otherwise an old process with stale settings could linger unnoticed. If you
prefer to manage the daemon yourself, set `autostart = false` in the
`[daemon]` section of `gtk.toml`.

### LLM correction (optional)

`Super+D` and `Super+E` send the note to an OpenAI-compatible
`/v1/chat/completions` endpoint — for example a local llama.cpp server.
Point it at your own server in `gtk.toml`:

```toml
[correction]
enabled = true
endpoint = "http://localhost:8080/v1/chat/completions"
model = "bielik-11b-v3"   # whatever your server serves — check /v1/models
api_key = ""              # empty = no Authorization header
```

`enabled = false` hides both buttons. The prompts are configuration too, and
the defaults are tuned for Polish — see
[docs/11-korekta-llm.md](docs/11-korekta-llm.md).

## Troubleshooting

- **The window subtitle says "Brak daemona" (no daemon) or "Daemon się
  rozłączył" (daemon disconnected).** Run `diktate-daemon --restart` in a
  terminal to see its log — most often the model file is missing
  (`diktate-cli model pull small-q5_1`).
- **The level bar does not move.** Wrong input device: run `diktate-cli setup`
  or set `[audio] device`.
- **A config change or a rebuild seems to have no effect.** The old daemon is
  still running: `diktate-daemon --restart`.
- **Transcription is slow.** Start the daemon in a terminal
  (`diktate-daemon --restart`); while loading the model it should log
  `using Vulkan0 backend`. If it does not, check `vulkaninfo --summary`, and
  make sure you did not build with `--no-default-features`.
- **Text comes out in the wrong language.** Set `[whisper] language` in
  `daemon.toml`.
- **Build fails because `src/parakeet.cpp` or `CMakeLists.txt` is missing.**
  The submodule is not checked out: `git submodule update --init`.

## Development

```bash
cargo build --workspace                  # debug build (Vulkan)
cargo test --workspace                   # no microphone, model or display needed
cargo clippy --workspace --all-targets

cargo run -p diktate-core --example segment-wav -- testdata/jfk.wav   # where the segmenter cuts
cargo run -p diktate-core --example bench-model -- sample.wav 4       # model cost, loaded once
python3 scripts/smoke-ipc.py             # IPC smoke test, needs a running daemon
```

| Crate                    | Role |
|--------------------------|------|
| `diktate-core`           | Audio capture (cpal), voice activity detection (Silero), segmentation, hallucination filters, transcription |
| `diktate-whispercpp-sys` | Builds whisper.cpp and exposes a narrow C++ shim to Rust — only pointers and numbers cross the FFI boundary |
| `diktate-proto`          | IPC message types — the single source of truth for the protocol |
| `diktate-document`       | The note model: spans, anchors, operation log, undo — no audio, no GTK |
| `diktate-config`         | Loading and validating `daemon.toml` and `gtk.toml` |
| `diktate-daemon`         | The background process that runs dictation sessions |
| `diktate-cli`            | Setup wizard, model downloads, `record` |
| `diktate-gtk`            | GTK 4 / libadwaita window, global shortcut, tray, settings |

Design documents, roadmap and the reasoning behind the dictation behaviour
are in [`docs/`](docs/README.md) (Polish). They take precedence over the code:
when the two disagree, one of them gets fixed deliberately.

## License

MIT, as declared in `Cargo.toml`.

Diktate builds on [whisper.cpp](https://github.com/ggml-org/whisper.cpp)
(MIT) and [Silero VAD](https://github.com/snakers4/silero-vad) (MIT).
`testdata/jfk.wav` is the sample recording shipped with whisper.cpp.
