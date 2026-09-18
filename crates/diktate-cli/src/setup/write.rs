//! Zapis konfiguracji: plik z komentarzami, kopia zapasowa, `rename`.
//!
//! Ręczne komentarze użytkownika nie są zachowywane — plik powstaje od nowa,
//! a poprzednia wersja ląduje w `.bak`. Świadoma rezygnacja z `toml_edit`:
//! zachowanie cudzego układu kosztuje osobną ścieżkę zapisu, a plik jest
//! w całości generowalny. Sam zapis jest atomowy (`docs/04`): inaczej
//! obserwator katalogu potrafi złapać plik w połowie pisania.

use std::path::Path;

use anyhow::{Context, Result};
use diktate_config::{DaemonConfig, GtkConfig};

/// Na razie wołane tylko przez testy i przyszły dialog kreatora (to
/// zadanie dostarcza jądro bez interfejsu).
#[allow(dead_code)]
pub fn daemon_toml(cfg: &DaemonConfig) -> String {
    let mut out = String::new();
    out.push_str("# daemon.toml — wygenerowane przez `diktate setup`.\n");
    out.push_str("# Opis wszystkich pól: docs/04-konfiguracja.md\n\n");

    out.push_str("[audio]\n");
    out.push_str(&field("device", &cfg.audio.device, "urządzenie wejściowe; \"default\" to systemowe"));
    out.push_str(&number("pre_roll_ms", cfg.audio.pre_roll_ms, "ile ciszy sprzed mowy dokleić, żeby nie uciąć pierwszej sylaby"));
    out.push_str(&number("level_interval_ms", cfg.audio.level_interval_ms, "odstęp między odczytami poziomu dla wskaźnika"));

    out.push_str("\n[vad]\n");
    out.push_str(&format!("threshold = {}  # próg decyzyjny Silero, 0.0–1.0\n", cfg.vad.threshold));
    out.push_str(&number("silence_ms_to_end", cfg.vad.silence_ms_to_end, "cisza domykająca krótką wypowiedź"));
    out.push_str(&number("silence_ms_to_end_long", cfg.vad.silence_ms_to_end_long, "cisza domykająca wypowiedź już rozpędzoną"));
    out.push_str(&number("long_segment_ms", cfg.vad.long_segment_ms, "od tej długości obowiązuje krótszy próg ciszy"));
    out.push_str(&number("min_speech_ms", cfg.vad.min_speech_ms, "krótsze odrzucane jako trzask"));
    out.push_str(&number("max_segment_ms", cfg.vad.max_segment_ms, "twarde cięcie; okno whispera to 30 s"));

    out.push_str("\n[whisper]\n");
    out.push_str(&field("model", &cfg.whisper.model, "ścieżka ggml; puste = $DIKTATE_MODEL, potem ~/.diktate/models"));
    out.push_str(&field("language", &cfg.whisper.language, "kod ISO albo \"auto\""));
    out.push_str(&field("initial_prompt", &cfg.whisper.initial_prompt, "terminologia i nazwy własne"));
    out.push_str(&number("beam_size", cfg.whisper.beam_size, "1 = dekodowanie zachłanne"));
    out.push_str(&number("threads", cfg.whisper.threads, "0 = decyzja whispera"));

    out.push_str("\n[queue]\n");
    out.push_str(&format!(
        "max_pending_segments = {}  # ile segmentów może czekać na transkrypcję\n",
        cfg.queue.max_pending_segments
    ));
    out
}

/// Na razie wołane tylko przez testy i przyszły dialog kreatora.
#[allow(dead_code)]
pub fn gtk_toml(cfg: &GtkConfig) -> String {
    let mut out = String::new();
    out.push_str("# gtk.toml — wygenerowane przez `diktate setup`.\n");
    out.push_str("# Opis wszystkich pól: docs/04-konfiguracja.md\n\n");

    out.push_str("[window]\n");
    out.push_str(&number("width", cfg.window.width as u32, "szerokość okna w pikselach"));
    out.push_str(&number("height", cfg.window.height as u32, "wysokość okna w pikselach"));
    out.push_str(&flag("show_level_meter", cfg.window.show_level_meter, "wskaźnik poziomu — brak wychyleń od razu mówi, że mikrofon jest nie ten"));

    out.push_str("\n[daemon]\n");
    out.push_str(&flag("autostart", cfg.daemon.autostart, "klient sam uruchamia daemona, gdy ten nie działa"));
    out.push_str(&format!(
        "start_timeout_ms = {}  # ile czekać na gniazdo; mieści wczytanie modelu\n",
        cfg.daemon.start_timeout_ms
    ));

    out.push_str("\n[dictation]\n");
    out.push_str(&flag("insert_at_cursor", cfg.dictation.insert_at_cursor, "segment wchodzi w miejscu karetki, nie na końcu notatki"));
    out.push_str(&flag("auto_space", cfg.dictation.auto_space, "spacja na stykach wypowiedzi"));
    out.push_str(&flag("strip_trailing_ellipsis", cfg.dictation.strip_trailing_ellipsis, "usuwanie wielokropka po pauzie na oddech"));
    out.push_str(&flag("follow_caret", cfg.dictation.follow_caret, "przewijanie do punktu wstawiania"));

    out.push_str("\n[shortcuts]\n");
    out.push_str(&field("copy", &cfg.shortcuts.copy, "kopiowanie notatki"));
    out.push_str(&field("undo", &cfg.shortcuts.undo, "cofnięcie"));
    out.push_str(&field("redo", &cfg.shortcuts.redo, "ponowienie"));
    out.push_str(&field("toggle_recording", &cfg.shortcuts.toggle_recording, "przełączenie nagrywania wewnątrz okna; skrót globalny rejestruje środowisko graficzne"));

    out.push_str("\n[tray]\n");
    out.push_str(&flag("enabled", cfg.tray.enabled, "ikona w zasobniku"));
    out.push_str(&flag("show_backlog", cfg.tray.show_backlog, "licznik zaległości na ikonie"));
    out
}

fn field(name: &str, value: &str, description: &str) -> String {
    format!("{name} = {}  # {description}\n", toml::Value::String(value.to_string()))
}

fn number(name: &str, value: u32, description: &str) -> String {
    format!("{name} = {value}  # {description}\n")
}

fn flag(name: &str, value: bool, description: &str) -> String {
    format!("{name} = {value}  # {description}\n")
}

/// Kopia zapasowa, potem zapis atomowy: plik tymczasowy obok i `rename`.
///
/// Nazwa pliku tymczasowego niesie numer procesu (`std::process::id()`),
/// żeby dwa równoległe uruchomienia kreatora nie pisały do tej samej
/// nazwy i nie nadpisały sobie zawartości przed `rename`.
///
/// Na razie wołane tylko przez testy i przyszły dialog kreatora.
#[allow(dead_code)]
pub fn save(path: &Path, text: &str) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .with_context(|| format!("nie udało się utworzyć {}", dir.display()))?;
    }
    if path.exists() {
        let bak = path.with_extension(
            path.extension()
                .map(|e| format!("{}.bak", e.to_string_lossy()))
                .unwrap_or_else(|| "bak".to_string()),
        );
        std::fs::copy(path, &bak)
            .with_context(|| format!("nie udało się zapisać kopii {}", bak.display()))?;
    }
    let tmp = path.with_extension(format!("toml.{}.tmp", std::process::id()));
    std::fs::write(&tmp, text)
        .with_context(|| format!("nie udało się zapisać {}", tmp.display()))?;
    std::fs::rename(&tmp, path)
        .with_context(|| format!("nie udało się nazwać {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use diktate_config::{DaemonConfig, GtkConfig};

    #[test]
    fn zapisany_daemon_toml_da_się_wczytać_z_powrotem() {
        let mut cfg = DaemonConfig::default();
        cfg.whisper.language = "en".to_string();
        cfg.vad.long_segment_ms = 4_000;

        let text = daemon_toml(&cfg);
        let wczytany = DaemonConfig::from_toml(&text).unwrap();

        assert_eq!(wczytany.whisper.language, "en");
        assert_eq!(wczytany.vad.long_segment_ms, 4_000);
    }

    #[test]
    fn zapisany_gtk_toml_da_się_wczytać_z_powrotem() {
        let mut cfg = GtkConfig::default();
        cfg.tray.enabled = false;
        let wczytany = GtkConfig::from_toml(&gtk_toml(&cfg)).unwrap();
        assert!(!wczytany.tray.enabled);
    }

    #[test]
    fn plik_ma_polskie_komentarze() {
        // Plik po kreatorze ma nadawać się do dalszej ręcznej edycji,
        // a nie być zrzutem serializera.
        let text = daemon_toml(&DaemonConfig::default());
        assert!(text.contains("# "), "brak komentarzy w wygenerowanym pliku");
        assert!(text.contains("cisza domykająca"));
    }

    #[test]
    fn zapis_zostawia_kopię_poprzedniej_wersji() {
        let dir = std::env::temp_dir()
            .join(format!("diktate-write-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("daemon.toml");
        std::fs::write(&path, "stara zawartość").unwrap();

        save(&path, "nowa zawartość").unwrap();

        assert_eq!(std::fs::read_to_string(&path).unwrap(), "nowa zawartość");
        assert_eq!(
            std::fs::read_to_string(dir.join("daemon.toml.bak")).unwrap(),
            "stara zawartość"
        );
    }

    #[test]
    fn zapis_nie_zostawia_pliku_tymczasowego() {
        let dir = std::env::temp_dir()
            .join(format!("diktate-write-tmp-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("daemon.toml");

        save(&path, "zawartość").unwrap();

        let ma_plik_tmp = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|wpis| wpis.ok())
            .any(|wpis| wpis.file_name().to_string_lossy().contains(".tmp"));
        assert!(!ma_plik_tmp, "po zapisie w katalogu został plik .tmp");
    }
}
