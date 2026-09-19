//! Przebieg kreatora: pytania, podgląd zmian, zapis, podpowiedź restartu.
//!
//! Domyślne odpowiedzi to wartości **dzisiejsze**, a nie fabryczne: kreator
//! uruchomiony po raz drugi ma poprawiać to, co jest, a nie kasować strojenie.

use anyhow::{bail, Result};
use diktate_config::{DaemonConfig, GtkConfig};

use crate::model::{catalog, cmd};
use crate::setup::{diff, dialog, profile, write};

pub fn run(advanced: bool) -> Result<()> {
    println!("Kreator konfiguracji Diktate.");
    println!("Enter przyjmuje wartość w nawiasie. Nic nie zapisujemy bez pytania.");

    let daemon_before = DaemonConfig::load()?;
    let gtk_before = GtkConfig::load()?;
    let mut daemon = daemon_before.clone();
    let mut gtk = gtk_before.clone();

    // --- model ---
    let options: Vec<String> = catalog::MODELS
        .iter()
        .map(|m| {
            let state = if m.installed().is_some() { " [masz]" } else { "" };
            format!(
                "{:<22} {:>7}  {:>4.1} s/20 s  polszczyzna: {}{state}",
                m.name,
                cmd::format_size(m.bytes),
                m.run_ms as f32 / 1000.0,
                m.polish.description()
            )
        })
        .collect();
    let default = catalog::MODELS
        .iter()
        .position(|m| daemon.whisper.model.contains(m.file))
        .unwrap_or_else(|| {
            catalog::MODELS
                .iter()
                .position(|m| m.recommended)
                .unwrap_or(0)
        });
    let choice = dialog::ask_choice("Który model whispera?", &options, default)?;
    let model = &catalog::MODELS[choice];
    // Pobranie idzie do 547 MB (`CLAUDE.md`) — zanim kreator ruszy siecią,
    // ma o tym powiedzieć wprost, a nie zacząć ściągać zaraz po wyborze
    // z listy. Model już obecny na dysku nie potrzebuje pytania: `pull_model`
    // i tak nie dotknie sieci, samo pytanie byłoby czczym rytuałem.
    let should_download = model.installed().is_some()
        || dialog::ask_bool(
            &format!(
                "Pobrać model {} ({})?",
                model.name,
                cmd::format_size(model.bytes)
            ),
            true,
        )?;
    let mut model_not_downloaded_hint: Option<String> = None;
    if should_download {
        let path = cmd::pull_model(model)?;
        daemon.whisper.model = path.display().to_string();
    } else {
        model_not_downloaded_hint = Some(format!(
            "Model {} nie został pobrany — whisper.model zostaje bez zmiany. \
             Pobranie później: diktate model pull {}",
            model.name, model.name
        ));
    }

    // --- język ---
    daemon.whisper.language =
        dialog::ask_text("Język transkrypcji (kod ISO albo \"auto\")", &daemon.whisper.language)?;

    // --- urządzenie ---
    let devices = input_devices();
    if devices.is_empty() {
        daemon.audio.device =
            dialog::ask_text("Urządzenie wejściowe", &daemon.audio.device)?;
    } else {
        let mut options = vec!["default (urządzenie systemowe)".to_string()];
        options.extend(devices.iter().cloned());
        let current = options
            .iter()
            .position(|o| o.starts_with(&daemon.audio.device))
            .unwrap_or(0);
        let choice = dialog::ask_choice("Które urządzenie wejściowe?", &options, current)?;
        daemon.audio.device = if choice == 0 {
            "default".to_string()
        } else {
            devices[choice - 1].clone()
        };
    }

    // --- rytm ---
    // To pytanie należy tylko do krótkiej ścieżki (docs/04, „Krótka
    // ścieżka"): `--advanced` „przechodzi zamiast tego przez każde pole po
    // kolei", więc pyta wprost o `vad.long_segment_ms` i resztę w
    // `ask_rest` — bez tej osłony zadalibyśmy o te same trzy progi dwa razy,
    // raz jako profil, raz jako pola.
    if !advanced {
        // Progi spoza trzech profili to „profil własny" (docs/04) — kreatorowi
        // nie wolno podmienić go po cichu, więc dokładamy do listy dodatkową
        // pozycję, która zostawia progi bez zmian, i robimy z niej domyślną
        // odpowiedź pod Enterem.
        let mut options: Vec<String> = profile::RHYTHMS
            .iter()
            .map(|r| format!("{:<14} {} ms — {}", r.name, r.long_segment_ms, r.description))
            .collect();
        let own_index = if profile::current(&daemon).is_none() {
            options.push(format!(
                "własny          — zostaw progi, które już masz (long_segment_ms = {}, \
                 silence_ms_to_end = {}, silence_ms_to_end_long = {})",
                daemon.vad.long_segment_ms,
                daemon.vad.silence_ms_to_end,
                daemon.vad.silence_ms_to_end_long,
            ));
            Some(options.len() - 1)
        } else {
            None
        };
        let default_rhythm = match profile::current(&daemon) {
            Some(r) => profile::RHYTHMS
                .iter()
                .position(|x| x.name == r.name)
                .unwrap_or(1),
            None => own_index.expect("own_index ustawiony, gdy current() zwraca None"),
        };
        // Odmowa ostrzeżenia o zbyt szybkim rytmie wraca tylko do tego pytania —
        // nie do całego kreatora, żeby nie gubić odpowiedzi z bieżącej sesji.
        let chosen_rhythm = loop {
            let choice = dialog::ask_choice("Rytm segmentacji?", &options, default_rhythm)?;
            if Some(choice) == own_index {
                break None;
            }
            let rhythm = &profile::RHYTHMS[choice];
            if profile::too_fast_for(rhythm, model.run_ms) {
                println!(
                    "\nUwaga: przy modelu {} segment w tym rytmie kosztuje więcej, niż trwa.",
                    model.name
                );
                println!("Kolejka będzie rosła, a tekst pojawiał się coraz później, nie wcześniej.");
                if !dialog::ask_bool("Mimo to ustawić ten rytm?", false)? {
                    continue;
                }
            }
            break Some(rhythm);
        };
        if let Some(rhythm) = chosen_rhythm {
            profile::apply(&mut daemon.vad, rhythm);
        }
    }

    // --- GTK ---
    gtk.tray.enabled = dialog::ask_bool("Ikona w zasobniku systemowym?", gtk.tray.enabled)?;
    println!(
        "\nSkrót naprawdę globalny (działa spoza okna) rejestruje się osobno: \
         diktate-gtk --install-shortcut \"<Control><Alt>d\""
    );
    gtk.shortcuts.toggle_recording = dialog::ask_text(
        "Skrót przełączający nagrywanie w oknie (globalny rejestruje środowisko graficzne)",
        &gtk.shortcuts.toggle_recording,
    )?;

    if advanced {
        ask_rest(&mut daemon, &mut gtk)?;
    }

    // --- walidacja wyniku ---
    // Kreator nie potrafi zapisać pliku, który zablokowałby start daemona
    // albo klienta (docs/04, „Zapis"). `from_toml` woła walidację `diktate-config`
    // wewnątrz, więc nie utrzymujemy tu drugiej, własnej listy reguł.
    //
    // `--advanced` pyta o surowe liczby, więc błąd walidacji nie może
    // wyrzucać całej sesji pytań — wraca do `ask_rest` z wartościami, które
    // użytkownik już podał, i próbuje ponownie. Krótka ścieżka nie ma czego
    // poprawiać w ten sposób (pytania o profil już same pilnują spójności),
    // więc tam błąd kończy kreator, i to niezerowym kodem wyjścia (`bail!`),
    // a nie cichym `Ok(())`.
    let (daemon_text, gtk_text) = loop {
        // Ostrzeżenie o zbyt szybkim rytmie należy do kreatora niezależnie od
        // tego, którą ścieżką się do niego doszło (docs/04) — krótka ścieżka
        // pyta o nie przy wyborze profilu, `--advanced` dopiero tutaj, bo
        // dopiero teraz zna progi wynikowe.
        if advanced
            && profile::too_fast_thresholds(
                daemon.vad.long_segment_ms,
                daemon.vad.silence_ms_to_end_long,
                model.run_ms,
            )
        {
            println!(
                "\nUwaga: przy modelu {} segment w tym rytmie kosztuje więcej, niż trwa.",
                model.name
            );
            println!("Kolejka będzie rosła, a tekst pojawiał się coraz później, nie wcześniej.");
            if !dialog::ask_bool("Mimo to zostawić te progi?", false)? {
                ask_rest(&mut daemon, &mut gtk)?;
                continue;
            }
        }

        let daemon_text = write::daemon_toml(&daemon);
        let gtk_text = write::gtk_toml(&gtk);
        let error = DaemonConfig::from_toml(&daemon_text)
            .err()
            .or_else(|| GtkConfig::from_toml(&gtk_text).err());
        let Some(e) = error else {
            break (daemon_text, gtk_text);
        };
        println!("\nTa konfiguracja nie wystartuje: {e}");
        if !advanced {
            bail!("Nic nie zapisano.");
        }
        if !dialog::ask_bool("Poprawić odpowiedzi?", true)? {
            bail!("Nic nie zapisano.");
        }
        ask_rest(&mut daemon, &mut gtk)?;
    };

    // --- podgląd i zapis ---
    let changes_daemon = diff::changes(&daemon_before, &daemon)?;
    let changes_gtk = diff::changes(&gtk_before, &gtk)?;
    if changes_daemon.is_empty() && changes_gtk.is_empty() {
        println!("\nNic się nie zmieniło — nie ma czego zapisywać.");
        if let Some(hint) = &model_not_downloaded_hint {
            println!("\n{hint}");
        }
        return Ok(());
    }
    println!("\nZmiany:");
    for change in changes_daemon.iter().chain(changes_gtk.iter()) {
        println!("  {}: {} → {}", change.field, change.before, change.after);
    }
    if !dialog::ask_bool("Zapisać?", true)? {
        println!("Nic nie zapisano.");
        if let Some(hint) = &model_not_downloaded_hint {
            println!("\n{hint}");
        }
        return Ok(());
    }

    // Piszemy tylko ten plik, w którym coś się faktycznie zmieniło — inaczej
    // dotknięcie samej opcji GTK kasowałoby ręczne komentarze w nietkniętym
    // `daemon.toml` i podmieniało jego `.bak` bez powodu.
    let daemon_path = DaemonConfig::path();
    let gtk_path = GtkConfig::path();
    let mut saved = Vec::new();
    if !changes_daemon.is_empty() {
        write::save(&daemon_path, &daemon_text)?;
        saved.push(&daemon_path);
    }
    if !changes_gtk.is_empty() {
        write::save(&gtk_path, &gtk_text)?;
        saved.push(&gtk_path);
    }

    println!("\nZapisano:");
    for path in &saved {
        println!("  {}", path.display());
    }
    if !changes_daemon.is_empty() {
        println!("  model: {}", daemon.whisper.model);
    }
    println!("\nWszystkie pola: diktate setup --advanced");
    if let Some(hint) = &model_not_downloaded_hint {
        println!("\n{hint}");
    }

    if !changes_daemon.is_empty() {
        if let Some(pid) = diktate_proto::pidfile::running() {
            println!(
                "\nDaemon (proces {pid}) nadal chodzi ze starą konfiguracją. \
                 Bez restartu zmiany wyglądają, jakby nie działały:"
            );
            println!("  diktate-daemon --restart");
        }
    }
    Ok(())
}

/// Nazwy urządzeń wejściowych. Pusta lista znaczy tyle, że nie da się ich
/// wymienić (brak serwera dźwięku) — wtedy pytamy o nazwę tekstem.
fn input_devices() -> Vec<String> {
    diktate_core::input_device_names().unwrap_or_default()
}

/// `--advanced`: każde pozostałe pole po kolei, z bieżącą wartością
/// w nawiasie.
fn ask_rest(daemon: &mut DaemonConfig, gtk: &mut GtkConfig) -> Result<()> {
    println!("\n--- Pozostałe pola daemon.toml ---");
    daemon.vad.threshold = dialog::ask_f32(
        "vad.threshold — próg decyzyjny Silero, 0.0–1.0",
        daemon.vad.threshold,
    )?;
    daemon.audio.pre_roll_ms = dialog::ask_u32(
        "audio.pre_roll_ms — ile ciszy sprzed mowy dokleić",
        daemon.audio.pre_roll_ms,
    )?;
    daemon.audio.level_interval_ms = dialog::ask_u32(
        "audio.level_interval_ms — odstęp odczytów poziomu",
        daemon.audio.level_interval_ms,
    )?;
    daemon.vad.min_speech_ms = dialog::ask_u32(
        "vad.min_speech_ms — krótsze wypowiedzi odrzucamy jako trzask",
        daemon.vad.min_speech_ms,
    )?;
    daemon.vad.max_segment_ms = dialog::ask_u32(
        "vad.max_segment_ms — twarde cięcie (okno whispera to 30 000 ms)",
        daemon.vad.max_segment_ms,
    )?;
    daemon.vad.silence_ms_to_end = dialog::ask_u32(
        "vad.silence_ms_to_end — cisza domykająca krótką wypowiedź",
        daemon.vad.silence_ms_to_end,
    )?;
    daemon.vad.silence_ms_to_end_long = dialog::ask_u32(
        "vad.silence_ms_to_end_long — cisza domykająca wypowiedź rozpędzoną",
        daemon.vad.silence_ms_to_end_long,
    )?;
    daemon.vad.long_segment_ms = dialog::ask_u32(
        "vad.long_segment_ms — od tej długości obowiązuje krótszy próg",
        daemon.vad.long_segment_ms,
    )?;
    daemon.whisper.initial_prompt = dialog::ask_text(
        "whisper.initial_prompt — terminologia i nazwy własne",
        &daemon.whisper.initial_prompt,
    )?;
    daemon.whisper.beam_size = dialog::ask_u32(
        "whisper.beam_size — 1 to dekodowanie zachłanne",
        daemon.whisper.beam_size,
    )?;
    daemon.whisper.threads = dialog::ask_u32(
        "whisper.threads — 0 zostawia decyzję whisperowi",
        daemon.whisper.threads,
    )?;
    daemon.queue.max_pending_segments = dialog::ask_u32(
        "queue.max_pending_segments — ile segmentów może czekać",
        daemon.queue.max_pending_segments as u32,
    )? as usize;

    println!("\n--- Pozostałe pola gtk.toml ---");
    gtk.window.width = dialog::ask_u32("window.width", gtk.window.width as u32)? as i32;
    gtk.window.height = dialog::ask_u32("window.height", gtk.window.height as u32)? as i32;
    gtk.window.show_level_meter =
        dialog::ask_bool("window.show_level_meter — wskaźnik poziomu?", gtk.window.show_level_meter)?;
    gtk.daemon.autostart =
        dialog::ask_bool("daemon.autostart — klient sam uruchamia daemona?", gtk.daemon.autostart)?;
    gtk.daemon.start_timeout_ms = dialog::ask_u32(
        "daemon.start_timeout_ms — ile czekać na gniazdo",
        gtk.daemon.start_timeout_ms as u32,
    )? as u64;
    gtk.dictation.insert_at_cursor = dialog::ask_bool(
        "dictation.insert_at_cursor — wstawiać w miejscu karetki?",
        gtk.dictation.insert_at_cursor,
    )?;
    gtk.dictation.auto_space =
        dialog::ask_bool("dictation.auto_space — spacja na stykach?", gtk.dictation.auto_space)?;
    gtk.dictation.strip_trailing_ellipsis = dialog::ask_bool(
        "dictation.strip_trailing_ellipsis — usuwać końcowy wielokropek?",
        gtk.dictation.strip_trailing_ellipsis,
    )?;
    gtk.dictation.follow_caret = dialog::ask_bool(
        "dictation.follow_caret — przewijać do punktu wstawiania?",
        gtk.dictation.follow_caret,
    )?;
    gtk.shortcuts.copy = dialog::ask_text("shortcuts.copy", &gtk.shortcuts.copy)?;
    gtk.shortcuts.undo = dialog::ask_text("shortcuts.undo", &gtk.shortcuts.undo)?;
    gtk.shortcuts.redo = dialog::ask_text("shortcuts.redo", &gtk.shortcuts.redo)?;
    gtk.tray.show_backlog =
        dialog::ask_bool("tray.show_backlog — licznik zaległości na ikonie?", gtk.tray.show_backlog)?;
    Ok(())
}
