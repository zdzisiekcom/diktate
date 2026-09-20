//! Zapis konfiguracji z kreatora.
//!
//! Renderowanie i atomowy zapis mieszkają w `diktate-config`
//! (`render.rs`) — kreator i okno ustawień piszą tę samą treść i nie mogą
//! się rozjechać. Tutaj zostaje to, co jest sprawą kreatora.

pub use diktate_config::render::{daemon_toml, gtk_toml, save};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::setup::diff;
    use diktate_config::{DaemonConfig, GtkConfig};

    #[test]
    fn daemon_toml_nie_gubi_żadnego_pola() {
        // Sieć bezpieczeństwa na pole dodane kiedyś w `diktate-config`
        // i zapomniane w tym generatorze: każde pole dostaje tu wartość inną
        // niż domyślna, więc pominięcie któregokolwiek w `daemon_toml`
        // ujawni się jako niepusta różnica po przejściu przez `from_toml`.
        let mut cfg = DaemonConfig::default();
        cfg.audio.device = "hw:1".to_string();
        cfg.audio.pre_roll_ms = 301;
        cfg.audio.level_interval_ms = 51;
        cfg.vad.threshold = 0.6;
        cfg.vad.silence_ms_to_end = 1_600;
        cfg.vad.silence_ms_to_end_long = 201;
        cfg.vad.long_segment_ms = 2_501;
        cfg.vad.min_speech_ms = 301;
        cfg.vad.max_segment_ms = 15_001;
        cfg.whisper.model = "/tmp/model.bin".to_string();
        cfg.whisper.language = "en".to_string();
        cfg.whisper.initial_prompt = "test prompt".to_string();
        cfg.whisper.beam_size = 2;
        cfg.whisper.threads = 1;
        cfg.whisper.context.max_prompt_chars = 401;
        cfg.whisper.context.repetition_guard = false;
        cfg.whisper.context.repetition_threshold = 0.42;
        cfg.transcription.backend = diktate_config::Backend::Parakeet;
        cfg.parakeet.model = "/tmp/parakeet.bin".to_string();
        cfg.parakeet.threads = 3;
        cfg.queue.max_pending_segments = 17;

        let wczytany = DaemonConfig::from_toml(&daemon_toml(&cfg)).unwrap();
        let zmiany = diff::changes(&cfg, &wczytany).unwrap();
        assert!(zmiany.is_empty(), "generator zgubił pola: {zmiany:?}");
    }

    #[test]
    fn gtk_toml_nie_gubi_żadnego_pola() {
        // Ta sama sieć bezpieczeństwa co wyżej, dla `gtk_toml`.
        let mut cfg = GtkConfig::default();
        cfg.window.width = 621;
        cfg.window.height = 421;
        cfg.window.show_level_meter = false;
        cfg.daemon.autostart = false;
        cfg.daemon.start_timeout_ms = 30_001;
        cfg.dictation.insert_at_cursor = false;
        cfg.dictation.auto_space = false;
        cfg.dictation.strip_trailing_ellipsis = false;
        cfg.dictation.follow_caret = false;
        cfg.shortcuts.copy = "<Control>x".to_string();
        cfg.shortcuts.undo = "<Control>y".to_string();
        cfg.shortcuts.redo = "<Control><Shift>y".to_string();
        cfg.shortcuts.toggle_recording = "<Control>v".to_string();
        cfg.tray.enabled = false;
        cfg.tray.show_backlog = false;
        cfg.whisper.prompt = "Kontekst: {kontekst}\nDrugi wiersz szablonu.".to_string();
        cfg.whisper.context_chars = 351;
        cfg.correction.redaction_system_prompt =
            "Redaktor testowy.\nDrugi wiersz instrukcji.".to_string();
        cfg.correction.redaction_user_template =
            "Cel: {instruction}\n\nTekst:\n\n{text}".to_string();

        let wczytany = GtkConfig::from_toml(&gtk_toml(&cfg)).unwrap();
        let zmiany = diff::changes(&cfg, &wczytany).unwrap();
        assert!(zmiany.is_empty(), "generator zgubił pola: {zmiany:?}");
    }
}
