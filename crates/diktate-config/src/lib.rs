//! Konfiguracja: `daemon.toml` i `gtk.toml`.
//!
//! Zasada z `docs/04-konfiguracja.md`: **żadnej magicznej stałej w kodzie**.
//! Każdy próg, opóźnienie i limit ma swoje pole i udokumentowaną wartość
//! domyślną, a błąd walidacji jest odmową startu z czytelnym komunikatem —
//! nigdy cichym powrotem do domyślnych. Literówka w nazwie pola zatrzymuje
//! start (`deny_unknown_fields`), bo cicho pominięte pole to najgorszy
//! rodzaj błędu konfiguracji: wygląda jak działające.
//!
//! Każdy proces czyta **wyłącznie swój** plik. Agent na maszynie bez serwera
//! wyświetlania nie powinien nawet widzieć `gtk.toml`, a klient GUI nie ma
//! powodu ruszać progów VAD.

use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

/// Katalog konfiguracji: `$XDG_CONFIG_HOME/diktate`, z odwrotem na
/// `~/.config/diktate`.
pub fn config_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("XDG_CONFIG_HOME") {
        return PathBuf::from(dir).join("diktate");
    }
    let home = std::env::var_os("HOME").map(PathBuf::from);
    home.unwrap_or_else(|| PathBuf::from("."))
        .join(".config/diktate")
}

/// Wczytuje plik konfiguracyjny; brak pliku to wartości domyślne, a nie błąd.
fn read_or_default(path: &std::path::Path) -> Result<String> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(text),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(err) => Err(err).with_context(|| format!("nie udało się odczytać {}", path.display())),
    }
}

// --- daemon.toml ---

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct DaemonConfig {
    pub audio: AudioConfig,
    pub vad: VadConfig,
    pub whisper: WhisperConfig,
    pub queue: QueueConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct AudioConfig {
    /// Nazwa urządzenia wejściowego; `"default"` to urządzenie systemowe.
    pub device: String,
    /// Ile ciszy sprzed wykrycia mowy dołączyć do segmentu — bez tego
    /// pierwsza sylaba bywa ucięta.
    pub pre_roll_ms: u32,
    /// Odstęp między odczytami poziomu wysyłanymi do wskaźnika.
    pub level_interval_ms: u32,
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            device: "default".to_string(),
            pre_roll_ms: 300,
            level_interval_ms: 50,
        }
    }
}

/// Progi segmentacji. Nazwy odpowiadają `SegmenterConfig` w `diktate-core` —
/// to jedne i te same pokrętła, opisane w `docs/04`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct VadConfig {
    /// Próg decyzyjny Silero (0.0–1.0).
    pub threshold: f32,
    /// Cisza domykająca wypowiedź krótszą niż `long_segment_ms`.
    pub silence_ms_to_end: u32,
    /// Cisza domykająca wypowiedź dłuższą niż `long_segment_ms` — krótka,
    /// żeby tekst płynął na ekran w trakcie mówienia.
    pub silence_ms_to_end_long: u32,
    /// Po jakiej długości wypowiedzi przełączyć się na krótszy próg ciszy.
    pub long_segment_ms: u32,
    /// Krótsze wypowiedzi odrzucamy jako stuknięcie albo szum.
    pub min_speech_ms: u32,
    /// Twardy limit długości segmentu.
    pub max_segment_ms: u32,
}

impl Default for VadConfig {
    fn default() -> Self {
        Self {
            threshold: 0.5,
            silence_ms_to_end: 1_500,
            silence_ms_to_end_long: 200,
            long_segment_ms: 2_500,
            min_speech_ms: 300,
            max_segment_ms: 15_000,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct WhisperConfig {
    /// Ścieżka modelu ggml; puste = `$DIKTATE_MODEL` albo ścieżka domyślna.
    pub model: String,
    /// Kod ISO języka albo `"auto"`.
    pub language: String,
    /// Podpowiedź kontekstowa doklejana do promptu (terminologia, nazwy).
    pub initial_prompt: String,
    /// Szerokość wiązki; 1 oznacza dekodowanie zachłanne.
    pub beam_size: u32,
    /// Liczba wątków; 0 = tyle, ile rdzeni.
    pub threads: u32,
}

impl Default for WhisperConfig {
    fn default() -> Self {
        Self {
            model: String::new(),
            language: "pl".to_string(),
            initial_prompt: String::new(),
            beam_size: 1,
            threads: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct QueueConfig {
    /// Ile segmentów może czekać na transkrypcję. Limit jest celowy: mowa
    /// bywa szybsza niż GPU, a kolejka bez dna zjadłaby pamięć i odsuwała
    /// tekst coraz dalej od chwili, w której padł.
    pub max_pending_segments: usize,
}

impl Default for QueueConfig {
    fn default() -> Self {
        Self {
            max_pending_segments: 16,
        }
    }
}

impl DaemonConfig {
    pub fn path() -> PathBuf {
        config_dir().join("daemon.toml")
    }

    /// Wczytuje `daemon.toml`. Brak pliku to wartości domyślne; plik
    /// niepoprawny to błąd, nie ciche domyślne.
    pub fn load() -> Result<Self> {
        let path = Self::path();
        let text = read_or_default(&path)?;
        Self::from_toml(&text).with_context(|| format!("w pliku {}", path.display()))
    }

    pub fn from_toml(text: &str) -> Result<Self> {
        let config: Self = toml::from_str(text).context("niepoprawny daemon.toml")?;
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<()> {
        if !(0.0..=1.0).contains(&self.vad.threshold) {
            bail!(
                "vad.threshold musi mieścić się w 0.0–1.0, a jest {}",
                self.vad.threshold
            );
        }
        if self.vad.min_speech_ms >= self.vad.max_segment_ms {
            bail!(
                "vad.min_speech_ms ({}) musi być mniejsze od vad.max_segment_ms ({})",
                self.vad.min_speech_ms,
                self.vad.max_segment_ms
            );
        }
        if self.vad.max_segment_ms > WHISPER_WINDOW_MS {
            bail!(
                "vad.max_segment_ms ({}) przekracza okno whispera ({WHISPER_WINDOW_MS} ms) — \
                 dłuższy segment i tak zostałby obcięty, tyle że po cichu",
                self.vad.max_segment_ms
            );
        }
        if self.vad.silence_ms_to_end_long > self.vad.silence_ms_to_end {
            bail!(
                "vad.silence_ms_to_end_long ({}) musi być krótsze niż vad.silence_ms_to_end ({}) \
                 — adaptacyjny próg ma rytm skracać, nie wydłużać",
                self.vad.silence_ms_to_end_long,
                self.vad.silence_ms_to_end
            );
        }
        if self.vad.long_segment_ms >= self.vad.max_segment_ms {
            bail!(
                "vad.long_segment_ms ({}) musi być mniejsze od vad.max_segment_ms ({}) — \
                 inaczej krótszy próg ciszy nigdy by się nie włączył",
                self.vad.long_segment_ms,
                self.vad.max_segment_ms
            );
        }
        if self.audio.level_interval_ms == 0 {
            bail!("audio.level_interval_ms musi być większe od zera");
        }
        if self.queue.max_pending_segments == 0 {
            bail!("queue.max_pending_segments musi być większe od zera");
        }
        Ok(())
    }
}

/// Whisper przetwarza zawsze okno 30 sekund — to liczba, o którą opiera się
/// cała segmentacja (`CLAUDE.md`, `docs/07`).
const WHISPER_WINDOW_MS: u32 = 30_000;

// --- gtk.toml ---

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct GtkConfig {
    pub window: WindowConfig,
    pub daemon: GtkDaemonConfig,
    pub dictation: DictationConfig,
    pub shortcuts: ShortcutsConfig,
    pub tray: TrayConfig,
}

/// Co klient GUI robi, gdy daemon nie działa.
///
/// Domyślnie uruchamia go sam: z punktu widzenia użytkownika Diktate jest
/// jedną aplikacją, a nie usługą, którą trzeba pamiętać, żeby włączyć.
/// Wyłączenie `autostart` zostawia decyzję systemowi — sensowne tam, gdzie
/// daemon chodzi z `systemd --user`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct GtkDaemonConfig {
    pub autostart: bool,
    /// Ile czekać na gniazdo po uruchomieniu daemona. Czas liczy się od
    /// startu procesu i musi pomieścić wczytanie modelu whispera — to
    /// sekundy, nie milisekundy.
    pub start_timeout_ms: u64,
}

impl Default for GtkDaemonConfig {
    fn default() -> Self {
        Self {
            autostart: true,
            start_timeout_ms: 30_000,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct WindowConfig {
    pub width: i32,
    pub height: i32,
    /// Czy pokazywać wskaźnik poziomu sygnału. Jego funkcja jest
    /// diagnostyczna: brak wychyleń od razu mówi, że mikrofon jest nie ten.
    pub show_level_meter: bool,
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self {
            width: 620,
            height: 420,
            show_level_meter: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct DictationConfig {
    /// Segment wchodzi w miejscu karetki, nie na końcu notatki.
    pub insert_at_cursor: bool,
    /// Spacja rozdzielająca na stykach wypowiedzi.
    pub auto_space: bool,
    /// Usuwanie wielokropka kończącego wypowiedź przerwaną pauzą na oddech.
    pub strip_trailing_ellipsis: bool,
    /// Przewijanie do punktu wstawiania — tylko gdy ten był widoczny.
    pub follow_caret: bool,
}

impl Default for DictationConfig {
    fn default() -> Self {
        Self {
            insert_at_cursor: true,
            auto_space: true,
            strip_trailing_ellipsis: true,
            follow_caret: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct ShortcutsConfig {
    pub copy: String,
    pub undo: String,
    pub redo: String,
    /// Przełączenie nagrywania **wewnątrz okna**. Skrót globalny rejestruje
    /// środowisko graficzne i nie ma go w tym pliku (`docs/06`).
    pub toggle_recording: String,
}

impl Default for ShortcutsConfig {
    fn default() -> Self {
        Self {
            copy: "<Control>c".to_string(),
            undo: "<Control>z".to_string(),
            redo: "<Control><Shift>z".to_string(),
            toggle_recording: "<Control>space".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct TrayConfig {
    pub enabled: bool,
    /// Licznik zaległości na ikonie.
    pub show_backlog: bool,
}

impl Default for TrayConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            show_backlog: true,
        }
    }
}

impl GtkConfig {
    pub fn path() -> PathBuf {
        config_dir().join("gtk.toml")
    }

    pub fn load() -> Result<Self> {
        let path = Self::path();
        let text = read_or_default(&path)?;
        Self::from_toml(&text).with_context(|| format!("w pliku {}", path.display()))
    }

    pub fn from_toml(text: &str) -> Result<Self> {
        let config: Self = toml::from_str(text).context("niepoprawny gtk.toml")?;
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<()> {
        if self.window.width <= 0 {
            bail!(
                "window.width musi być dodatnie, a jest {}",
                self.window.width
            );
        }
        if self.window.height <= 0 {
            bail!(
                "window.height musi być dodatnie, a jest {}",
                self.window.height
            );
        }
        for (nazwa, skrót) in [
            ("shortcuts.copy", &self.shortcuts.copy),
            ("shortcuts.undo", &self.shortcuts.undo),
            ("shortcuts.redo", &self.shortcuts.redo),
            (
                "shortcuts.toggle_recording",
                &self.shortcuts.toggle_recording,
            ),
        ] {
            if skrót.trim().is_empty() {
                bail!("{nazwa} jest pusty — nie ma czego związać z akcją");
            }
        }
        Ok(())
    }
}
