//! Przebieg kreatora: pytania, podgląd zmian, zapis, podpowiedź restartu.
//!
//! Domyślne odpowiedzi to wartości **dzisiejsze**, a nie fabryczne: kreator
//! uruchomiony po raz drugi ma poprawiać to, co jest, a nie kasować strojenie.

use anyhow::Result;
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
                m.polish.opis()
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
    let path = cmd::pull_model(model)?;
    daemon.whisper.model = path.display().to_string();

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
    let options: Vec<String> = profile::RHYTHMS
        .iter()
        .map(|r| format!("{:<14} {} ms — {}", r.name, r.long_segment_ms, r.opis))
        .collect();
    let current = profile::current(&daemon)
        .and_then(|r| profile::RHYTHMS.iter().position(|x| x.name == r.name))
        .unwrap_or(1);
    let choice = dialog::ask_choice("Rytm segmentacji?", &options, current)?;
    let rhythm = &profile::RHYTHMS[choice];
    if profile::too_fast_for(rhythm, model.run_ms) {
        println!(
            "\nUwaga: przy modelu {} segment w tym rytmie kosztuje więcej, niż trwa.",
            model.name
        );
        println!("Kolejka będzie rosła, a tekst pojawiał się coraz później, nie wcześniej.");
        if !dialog::ask_bool("Mimo to ustawić ten rytm?", false)? {
            return run(advanced);
        }
    }
    profile::apply(&mut daemon.vad, rhythm);

    // --- GTK ---
    gtk.tray.enabled = dialog::ask_bool("Ikona w zasobniku systemowym?", gtk.tray.enabled)?;
    gtk.shortcuts.toggle_recording = dialog::ask_text(
        "Skrót przełączający nagrywanie w oknie (globalny rejestruje środowisko graficzne)",
        &gtk.shortcuts.toggle_recording,
    )?;

    if advanced {
        ask_rest(&mut daemon, &mut gtk)?;
    }

    // --- podgląd i zapis ---
    let changes_daemon = diff::changes(&daemon_before, &daemon)?;
    let changes_gtk = diff::changes(&gtk_before, &gtk)?;
    if changes_daemon.is_empty() && changes_gtk.is_empty() {
        println!("\nNic się nie zmieniło — nie ma czego zapisywać.");
        return Ok(());
    }
    println!("\nZmiany:");
    for change in changes_daemon.iter().chain(changes_gtk.iter()) {
        println!("  {}: {} → {}", change.field, change.before, change.after);
    }
    if !dialog::ask_bool("Zapisać?", true)? {
        println!("Nic nie zapisano.");
        return Ok(());
    }

    let daemon_path = DaemonConfig::path();
    let gtk_path = GtkConfig::path();
    write::save(&daemon_path, &write::daemon_toml(&daemon))?;
    write::save(&gtk_path, &write::gtk_toml(&gtk))?;

    println!("\nZapisano:");
    println!("  {}", daemon_path.display());
    println!("  {}", gtk_path.display());
    println!("  model: {}", daemon.whisper.model);
    println!("\nWszystkie pola: diktate setup --advanced");

    if let Some(pid) = diktate_proto::pidfile::running() {
        println!(
            "\nDaemon (proces {pid}) nadal chodzi ze starą konfiguracją. \
             Bez restartu zmiany wyglądają, jakby nie działały:"
        );
        println!("  diktate-daemon --restart");
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
