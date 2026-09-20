//! Zapis konfiguracji: plik z komentarzami, kopia zapasowa, `rename`.
//!
//! Ręczne komentarze użytkownika nie są zachowywane — plik powstaje od nowa,
//! a poprzednia wersja ląduje w `.bak`. Świadoma rezygnacja z `toml_edit`:
//! zachowanie cudzego układu kosztuje osobną ścieżkę zapisu, a plik jest
//! w całości generowalny. Sam zapis jest atomowy (`docs/04`): inaczej
//! obserwator katalogu potrafi złapać plik w połowie pisania.

use std::path::Path;

use anyhow::{Context, Result};

use crate::{DaemonConfig, GtkConfig};

/// Fragment pierwszej linii `daemon_toml`/`gtk_toml`. Po nim `save`
/// rozpoznaje, czy plik docelowy jest wynikiem poprzedniego przebiegu
/// kreatora — taki plik da się odtworzyć z bieżącej konfiguracji w każdej
/// chwili, więc nie szkodzi podmienić jego `.bak`. Plik bez tego nagłówka
/// jest napisany ręcznie, a jego treść nie do odtworzenia — `.bak` z jego
/// zawartością musi przetrwać kolejne przebiegi kreatora.
const GENERATED_MARKER: &str = "wygenerowane przez `diktate setup`.";

pub fn daemon_toml(cfg: &DaemonConfig) -> String {
    let mut out = String::new();
    out.push_str(&format!("# daemon.toml — {GENERATED_MARKER}\n"));
    out.push_str("# Opis wszystkich pól: docs/04-konfiguracja.md\n\n");

    out.push_str("[audio]\n");
    out.push_str(&field(
        "device",
        &cfg.audio.device,
        "urządzenie wejściowe; \"default\" to systemowe",
    ));
    out.push_str(&number(
        "pre_roll_ms",
        cfg.audio.pre_roll_ms,
        "ile ciszy sprzed mowy dokleić, żeby nie uciąć pierwszej sylaby",
    ));
    out.push_str(&number(
        "level_interval_ms",
        cfg.audio.level_interval_ms,
        "odstęp między odczytami poziomu dla wskaźnika",
    ));

    out.push_str("\n[vad]\n");
    out.push_str(&format!(
        "threshold = {}  # próg decyzyjny Silero, 0.0–1.0\n",
        cfg.vad.threshold
    ));
    out.push_str(&number(
        "silence_ms_to_end",
        cfg.vad.silence_ms_to_end,
        "cisza domykająca krótką wypowiedź",
    ));
    out.push_str(&number(
        "silence_ms_to_end_long",
        cfg.vad.silence_ms_to_end_long,
        "cisza domykająca wypowiedź już rozpędzoną",
    ));
    out.push_str(&number(
        "long_segment_ms",
        cfg.vad.long_segment_ms,
        "od tej długości obowiązuje krótszy próg ciszy",
    ));
    out.push_str(&number(
        "min_speech_ms",
        cfg.vad.min_speech_ms,
        "krótsze odrzucane jako trzask",
    ));
    out.push_str(&number(
        "max_segment_ms",
        cfg.vad.max_segment_ms,
        "twarde cięcie; okno whispera to 30 s",
    ));

    out.push_str("\n[whisper]\n");
    out.push_str(&field(
        "model",
        &cfg.whisper.model,
        "ścieżka ggml; puste = $DIKTATE_MODEL, potem ~/.diktate/models",
    ));
    out.push_str(&field(
        "language",
        &cfg.whisper.language,
        "kod ISO albo \"auto\"",
    ));
    out.push_str(&field(
        "initial_prompt",
        &cfg.whisper.initial_prompt,
        "terminologia i nazwy własne",
    ));
    out.push_str(&number(
        "beam_size",
        cfg.whisper.beam_size,
        "1 = dekodowanie zachłanne",
    ));
    out.push_str(&number(
        "threads",
        cfg.whisper.threads,
        "0 = decyzja whispera",
    ));

    out.push_str("\n[whisper.context]\n");
    out.push_str(&number(
        "max_prompt_chars",
        cfg.whisper.context.max_prompt_chars as u32,
        "twardy limit długości promptu; nadmiar obcinany od początku",
    ));
    out.push_str(&flag(
        "repetition_guard",
        cfg.whisper.context.repetition_guard,
        "segment będący echem promptu licz ponownie bez kontekstu",
    ));
    out.push_str(&format!(
        "repetition_threshold = {}  # podobieństwo, od którego segment uchodzi za echo\n",
        cfg.whisper.context.repetition_threshold
    ));

    out.push_str("\n[queue]\n");
    out.push_str(&format!(
        "max_pending_segments = {}  # ile segmentów może czekać na transkrypcję\n",
        cfg.queue.max_pending_segments
    ));
    out
}

pub fn gtk_toml(cfg: &GtkConfig) -> String {
    let mut out = String::new();
    out.push_str(&format!("# gtk.toml — {GENERATED_MARKER}\n"));
    out.push_str("# Opis wszystkich pól: docs/04-konfiguracja.md\n\n");

    out.push_str("[window]\n");
    out.push_str(&number(
        "width",
        cfg.window.width as u32,
        "szerokość okna w pikselach",
    ));
    out.push_str(&number(
        "height",
        cfg.window.height as u32,
        "wysokość okna w pikselach",
    ));
    out.push_str(&flag(
        "show_level_meter",
        cfg.window.show_level_meter,
        "wskaźnik poziomu — brak wychyleń od razu mówi, że mikrofon jest nie ten",
    ));

    out.push_str("\n[daemon]\n");
    out.push_str(&flag(
        "autostart",
        cfg.daemon.autostart,
        "klient sam uruchamia daemona, gdy ten nie działa",
    ));
    out.push_str(&format!(
        "start_timeout_ms = {}  # ile czekać na gniazdo; mieści wczytanie modelu\n",
        cfg.daemon.start_timeout_ms
    ));

    out.push_str("\n[dictation]\n");
    out.push_str(&flag(
        "insert_at_cursor",
        cfg.dictation.insert_at_cursor,
        "segment wchodzi w miejscu karetki, nie na końcu notatki",
    ));
    out.push_str(&flag(
        "auto_space",
        cfg.dictation.auto_space,
        "spacja na stykach wypowiedzi",
    ));
    out.push_str(&flag(
        "strip_trailing_ellipsis",
        cfg.dictation.strip_trailing_ellipsis,
        "usuwanie wielokropka po pauzie na oddech",
    ));
    out.push_str(&flag(
        "follow_caret",
        cfg.dictation.follow_caret,
        "przewijanie do punktu wstawiania",
    ));

    out.push_str("\n[shortcuts]\n");
    out.push_str(&field("copy", &cfg.shortcuts.copy, "kopiowanie notatki"));
    out.push_str(&field("undo", &cfg.shortcuts.undo, "cofnięcie"));
    out.push_str(&field("redo", &cfg.shortcuts.redo, "ponowienie"));
    out.push_str(&field(
        "toggle_recording",
        &cfg.shortcuts.toggle_recording,
        "przełączenie nagrywania wewnątrz okna; skrót globalny rejestruje środowisko graficzne",
    ));

    out.push_str("\n[tray]\n");
    out.push_str(&flag("enabled", cfg.tray.enabled, "ikona w zasobniku"));
    out.push_str(&flag(
        "show_backlog",
        cfg.tray.show_backlog,
        "licznik zaległości na ikonie",
    ));

    out.push_str("\n[whisper]\n");
    out.push_str(&text_block(
        "prompt",
        &cfg.whisper.prompt,
        "szablon promptu; {kontekst} to miejsce na tekst sprzed kursora",
    ));
    out.push_str(&number(
        "context_chars",
        cfg.whisper.context_chars as u32,
        "ile znaków przed kursorem wchodzi w {kontekst}",
    ));

    let korekta = &cfg.correction;
    out.push_str("\n[correction]\n");
    out.push_str("# Korekta tekstu lokalnym LLM-em — opis i pomiary w docs/11-korekta-llm.md\n");
    out.push_str(&flag(
        "enabled",
        korekta.enabled,
        "przycisk korekty w oknie",
    ));
    out.push_str(&field(
        "endpoint",
        &korekta.endpoint,
        "adres API zgodnego z OpenAI",
    ));
    out.push_str(&field(
        "model",
        &korekta.model,
        "nazwa modelu; sprawdź /v1/models",
    ));
    out.push_str(&field(
        "api_key",
        &korekta.api_key,
        "puste = bez nagłówka Authorization",
    ));
    out.push_str(&format!(
        "temperature = {}  # 0.0 nie gwarantuje powtarzalności\n",
        korekta.temperature
    ));
    out.push_str(&number(
        "max_tokens",
        korekta.max_tokens,
        "limit odpowiedzi",
    ));
    out.push_str(&format!(
        "timeout_ms = {}  # ile czekać na odpowiedź modelu\n",
        korekta.timeout_ms
    ));
    out.push_str(&flag(
        "disable_thinking",
        korekta.disable_thinking,
        "wycisza rozumowanie; bez tego model z thinkingiem jest 5–45× wolniejszy",
    ));
    out.push_str(&number(
        "short_text_words",
        korekta.short_text_words as u32,
        "do tylu słów notatka jest krótka i ma luźniejszy próg",
    ));
    out.push_str(&format!(
        "max_change_ratio_short = {}  # odsetek zmienionych słów w krótkiej notatce\n",
        korekta.max_change_ratio_short
    ));
    out.push_str(&format!(
        "max_change_ratio_long = {}  # odsetek w dłuższej; powyżej model redaguje, a nie poprawia\n",
        korekta.max_change_ratio_long
    ));
    out.push_str(&number(
        "min_changed_words",
        korekta.min_changed_words as u32,
        "tyle zmienionych słów przechodzi zawsze, bez pytania",
    ));
    out.push_str(&field(
        "allowed_extra_chars",
        &korekta.allowed_extra_chars,
        "znaki dopuszczone ponad alfabet polski i interpunkcję",
    ));
    out.push_str(&text_block(
        "system_prompt",
        &korekta.system_prompt,
        "instrukcja korektora — to ona decyduje o zachowaniu modelu",
    ));
    out.push_str(&field(
        "user_template",
        &korekta.user_template,
        "{text} zastępuje treść notatki",
    ));
    out
}

/// Pole wielolinijkowe jako literał TOML-a `'''…'''`. Prompt korektora ma być
/// w pliku czytelny i edytowalny, a nie jedną linią z `\n` w środku. Tekst
/// zawierający sam ogranicznik wraca do zwykłego zapisu — poprawność pliku
/// jest ważniejsza niż jego uroda.
fn text_block(name: &str, value: &str, description: &str) -> String {
    if value.contains("'''") || value.ends_with('\'') {
        return field(name, value, description);
    }
    // Ogranicznik zamykający stoi tuż za tekstem: w literale `'''` znika
    // tylko przełamanie **po** otwarciu, więc nowa linia przed zamknięciem
    // dokleiłaby się do promptu i plik przestałby się odczytywać tak, jak
    // został zapisany.
    format!("# {description}\n{name} = '''\n{value}'''\n")
}

fn field(name: &str, value: &str, description: &str) -> String {
    format!(
        "{name} = {}  # {description}\n",
        toml::Value::String(value.to_string())
    )
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
/// `.bak` dostaje nową treść tylko wtedy, gdy plik zastępowany nie jest
/// wynikiem wcześniejszego przebiegu kreatora (`GENERATED_MARKER`) — inaczej
/// drugie uruchomienie kreatora podmieniłoby kopię pliku napisanego ręcznie
/// na kopię pliku, który sam wygenerował, i obietnica „masz gdzie wrócić”
/// z `docs/04` byłaby po dwóch przebiegach pusta.
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
        let target_is_generated = std::fs::read_to_string(path)
            .map(|existing| {
                existing
                    .lines()
                    .next()
                    .is_some_and(|first| first.contains(GENERATED_MARKER))
            })
            .unwrap_or(false);
        if !(bak.exists() && target_is_generated) {
            std::fs::copy(path, &bak)
                .with_context(|| format!("nie udało się zapisać kopii {}", bak.display()))?;
        }
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
    use crate::{DaemonConfig, GtkConfig};

    #[test]
    fn zapisany_daemon_toml_da_się_wczytać_z_powrotem() {
        let mut cfg = DaemonConfig::default();
        cfg.whisper.language = "en".to_string();
        cfg.vad.long_segment_ms = 4_000;
        // Sekcja `[whisper.context]` dopisana w tym zadaniu — wartości
        // niedomyślne, żeby zapis→odczyt naprawdę wykazał, że wraca to,
        // co poszło na dysk.
        cfg.whisper.context.max_prompt_chars = 400;
        cfg.whisper.context.repetition_guard = false;
        cfg.whisper.context.repetition_threshold = 0.65;

        let text = daemon_toml(&cfg);
        let wczytany = DaemonConfig::from_toml(&text).unwrap();

        assert_eq!(wczytany.whisper.language, "en");
        assert_eq!(wczytany.vad.long_segment_ms, 4_000);
        assert_eq!(wczytany.whisper.context.max_prompt_chars, 400);
        assert!(!wczytany.whisper.context.repetition_guard);
        assert!((wczytany.whisper.context.repetition_threshold - 0.65).abs() < f32::EPSILON);
    }

    #[test]
    fn zapisany_gtk_toml_da_się_wczytać_z_powrotem() {
        let mut cfg = GtkConfig::default();
        cfg.tray.enabled = false;
        // Szablon promptu whispera dopisany w tym zadaniu — wielolinijkowy
        // i z `{kontekst}`, żeby sprawdzić dokładnie ten przypadek, dla
        // którego istnieje `text_block`.
        cfg.whisper.prompt = "Terminy: ggml, whisper.\n{kontekst}".to_string();
        cfg.whisper.context_chars = 350;

        let wczytany = GtkConfig::from_toml(&gtk_toml(&cfg)).unwrap();

        assert!(!wczytany.tray.enabled);
        assert_eq!(wczytany.whisper.prompt, "Terminy: ggml, whisper.\n{kontekst}");
        assert_eq!(wczytany.whisper.context_chars, 350);
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
        let dir = std::env::temp_dir().join(format!("diktate-write-{}", std::process::id()));
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
    fn druga_wizyta_kreatora_nie_kasuje_kopii_ręcznego_pliku() {
        // Pierwszy zapis zastępuje plik napisany ręcznie — jego treść trafia
        // do `.bak`, bo tylko ona jest nie do odtworzenia. Drugi zapis
        // zastępuje już plik wygenerowany przez kreator, więc `.bak` ma
        // zostać nietknięty: nadpisanie go podmieniłoby jedyną kopię ręcznej
        // treści na kopię czegoś, co i tak dałoby się wygenerować ponownie.
        let dir =
            std::env::temp_dir().join(format!("diktate-write-bak-manual-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("daemon.toml");
        let ręczna_treść = "# napisane ręcznie, ma zostać w .bak\nlanguage = \"pl\"\n";
        std::fs::write(&path, ręczna_treść).unwrap();

        let pierwszy = daemon_toml(&DaemonConfig::default());
        save(&path, &pierwszy).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join("daemon.toml.bak")).unwrap(),
            ręczna_treść
        );

        let mut inna = DaemonConfig::default();
        inna.whisper.language = "en".to_string();
        let drugi = daemon_toml(&inna);
        save(&path, &drugi).unwrap();

        assert_eq!(
            std::fs::read_to_string(dir.join("daemon.toml.bak")).unwrap(),
            ręczna_treść,
            "drugi przebieg kreatora nadpisał kopię pliku napisanego ręcznie"
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), drugi);
    }

    #[test]
    fn zapis_nie_zostawia_pliku_tymczasowego() {
        let dir = std::env::temp_dir().join(format!("diktate-write-tmp-{}", std::process::id()));
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
