//! Daemon: gniazdo uniksowe, sesja strumieniowa, transkrypcja na bieżąco.
//!
//! Jeden klient naraz, obsługiwany sekwencyjnie. Sesja (`Session`) po
//! `session.start` nagrywa i transkrybuje w tle na własnych wątkach — gotowe
//! segmenty wracają do klienta powiadomieniami `segment.added`, niezależnie
//! od pętli obsługującej żądania na tym samym gnieździe. Dlatego strumień do
//! zapisu jest dzielony przez `Arc<Mutex<UnixStream>>`: i pętla żądań
//! (odpowiedzi), i wątek transkrypcji sesji (powiadomienia) piszą do niego
//! pod tym samym zamkiem, żeby żadna linia NDJSON się nie przeplotła.
mod cli;

use anyhow::{anyhow, bail, Context, Result};
use diktate_config::DaemonConfig;
use diktate_core::segmenter::SegmenterConfig;
use diktate_core::{transcribe::default_model_path, Session, SessionConfig, Transcriber};
use diktate_proto::{
    pidfile, socket_path, Notification, Request, Response, SegmentAdded, SessionState, METHOD_SESSION_START,
    METHOD_SESSION_STOP, METHOD_SESSION_SUBSCRIBE,
};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

fn main() -> Result<()> {
    match cli::parse(std::env::args().skip(1))? {
        cli::Command::Help => {
            println!("{}", cli::USAGE);
            return Ok(());
        }
        cli::Command::Stop => return stop(),
        cli::Command::Restart => replace_running(),
        cli::Command::Run => {
            // Dwa daemony na jednym gnieździe to cicha pułapka: ten młodszy
            // przejmuje gniazdo, a starszy zostaje w pamięci z nieaktualną
            // konfiguracją i wygląda, jakby zmiany „nie działały".
            if let Some(pid) = pidfile::running() {
                bail!(
                    "daemon już działa (PID {pid}) — użyj --restart, żeby go zastąpić, \
                     albo --stop, żeby go zatrzymać"
                );
            }
        }
    }

    // Błąd w konfiguracji zatrzymuje start z czytelnym komunikatem — nigdy
    // nie wracamy po cichu do wartości domyślnych (docs/04).
    let config = DaemonConfig::load().context("konfiguracja daemona")?;

    let model = model_path(&config);
    eprintln!("diktate-daemon: wczytuję model {}", model.display());
    let transcriber = Arc::new(
        Transcriber::new(&model)?
            .with_language(config.whisper.language.clone())
            .with_initial_prompt(config.whisper.initial_prompt.clone())
            .with_beam_size(config.whisper.beam_size)
            .with_threads(config.whisper.threads),
    );

    let path = socket_path();
    if path.exists() {
        // Gniazdo po poprzednim uruchomieniu — daemon jest jeden, więc usuwamy.
        std::fs::remove_file(&path)
            .with_context(|| format!("nie udało się usunąć {}", path.display()))?;
    }
    let listener = UnixListener::bind(&path)
        .with_context(|| format!("nie udało się otworzyć gniazda {}", path.display()))?;
    // Plik PID dopiero tutaj: wcześniejszy zapis obiecywałby działającego
    // daemona, choć start mógł jeszcze polec na modelu albo na gnieździe.
    pidfile::write_current()?;
    eprintln!(
        "diktate-daemon: nasłuchuję na {} (PID {})",
        path.display(),
        std::process::id()
    );

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                if let Err(err) = serve(stream, &transcriber) {
                    eprintln!("diktate-daemon: klient odłączony z błędem: {err:#}");
                }
            }
            Err(err) => eprintln!("diktate-daemon: nieudane połączenie: {err}"),
        }
    }
    pidfile::remove();
    Ok(())
}

/// `--stop`: zatrzymuje działającego daemona. Brak daemona nie jest błędem —
/// żądany stan („nie działa") i tak jest osiągnięty.
fn stop() -> Result<()> {
    match pidfile::running() {
        Some(pid) => {
            if pidfile::terminate(pid) {
                pidfile::remove();
                eprintln!("diktate-daemon: zatrzymany (PID {pid})");
                Ok(())
            } else {
                Err(anyhow!("nie udało się zatrzymać procesu {pid}"))
            }
        }
        None => {
            eprintln!("diktate-daemon: żaden daemon nie działa");
            Ok(())
        }
    }
}

/// `--restart`: robi miejsce dla siebie. Nieudane zatrzymanie poprzednika nie
/// przerywa startu — gniazdo i tak przejmiemy, a komunikat wystarczy, żeby
/// użytkownik wiedział, że ktoś został w pamięci.
fn replace_running() {
    if let Some(pid) = pidfile::running() {
        if pidfile::terminate(pid) {
            eprintln!("diktate-daemon: zastępuję poprzedni proces (PID {pid})");
        } else {
            eprintln!("diktate-daemon: nie udało się zatrzymać procesu {pid} — startuję mimo to");
        }
    }
}

/// Ścieżka modelu: z `daemon.toml`, a gdy puste — `$DIKTATE_MODEL` albo
/// ścieżka domyślna.
fn model_path(config: &DaemonConfig) -> std::path::PathBuf {
    if config.whisper.model.is_empty() {
        default_model_path()
    } else {
        std::path::PathBuf::from(&config.whisper.model)
    }
}

/// Parametry sesji z konfiguracji. Czytane przy każdym `session.start`, więc
/// zmiana progu VAD działa bez restartu daemona — wystarczy zacząć nagrywać
/// od nowa.
fn session_config(config: &DaemonConfig) -> SessionConfig {
    SessionConfig {
        segmenter: SegmenterConfig {
            pre_roll_ms: config.audio.pre_roll_ms,
            silence_ms_to_end: config.vad.silence_ms_to_end,
            silence_ms_to_end_long: config.vad.silence_ms_to_end_long,
            long_segment_ms: config.vad.long_segment_ms,
            min_speech_ms: config.vad.min_speech_ms,
            max_segment_ms: config.vad.max_segment_ms,
        },
        vad_threshold: config.vad.threshold,
        queue_capacity: config.queue.max_pending_segments,
        level_interval_ms: config.audio.level_interval_ms,
    }
}

fn serve(stream: UnixStream, transcriber: &Arc<Transcriber>) -> Result<()> {
    let writer = stream
        .try_clone()
        .context("nie udało się sklonować gniazda")?;
    // Dzielony między tę pętlę (odpowiedzi) a wątek transkrypcji sesji
    // (powiadomienia) — patrz komentarz modułu.
    let writer = Arc::new(Mutex::new(writer));
    let reader = BufReader::new(stream);
    let mut session: Option<Session> = None;
    // `audio.level` to kilkanaście wiadomości na sekundę — dostaje je
    // wyłącznie klient, który poprosił o nie jawnie (docs/02).
    let levels_wanted = Arc::new(AtomicBool::new(false));

    for line in reader.lines() {
        let line = line.context("nie udało się odczytać linii z gniazda")?;
        if line.trim().is_empty() {
            continue;
        }
        let request: Request = match serde_json::from_str(&line) {
            Ok(request) => request,
            Err(err) => {
                eprintln!("diktate-daemon: nieczytelna wiadomość: {err}");
                continue;
            }
        };

        match request.method.as_str() {
            METHOD_SESSION_START => {
                if session.is_some() {
                    send(
                        &writer,
                        &Response::err(request.id, -32003, "sesja już aktywna"),
                    )?;
                    continue;
                }
                let notify_writer = Arc::clone(&writer);
                let backlog_writer = Arc::clone(&writer);
                let level_writer = Arc::clone(&writer);
                let level_flag = Arc::clone(&levels_wanted);
                // Konfiguracja czytana na nowo przy każdym starcie sesji.
                // Plik niepoprawny nie wywraca działającego daemona —
                // zostajemy przy tym, co działało, i mówimy o tym głośno.
                let config = match DaemonConfig::load() {
                    Ok(config) => config,
                    Err(err) => {
                        eprintln!(
                            "diktate-daemon: konfiguracja niepoprawna, zostaję przy poprzedniej: {err:#}"
                        );
                        DaemonConfig::default()
                    }
                };
                match Session::start(
                    Arc::clone(transcriber),
                    session_config(&config),
                    move |segment| {
                        eprintln!(
                            "diktate-daemon: [{}] {} ({}–{} ms)",
                            segment.seq, segment.text, segment.audio_start_ms, segment.audio_end_ms
                        );
                        let params = SegmentAdded {
                            seq: segment.seq,
                            text: segment.text,
                            audio_start_ms: segment.audio_start_ms,
                            audio_end_ms: segment.audio_end_ms,
                        };
                        if let Err(err) =
                            send(&notify_writer, &Notification::segment_added(&params))
                        {
                            eprintln!(
                                "diktate-daemon: nie udało się wysłać powiadomienia: {err:#}"
                            );
                        }
                    },
                    move |backlog| {
                        eprintln!("diktate-daemon: zaległość kolejki: {backlog}");
                        if let Err(err) = send(
                            &backlog_writer,
                            &Notification::state_changed(SessionState::Recording, backlog),
                        ) {
                            eprintln!(
                                "diktate-daemon: nie udało się wysłać powiadomienia o stanie: {err:#}"
                            );
                        }
                    },
                    move |level| {
                        if !level_flag.load(Ordering::Relaxed) {
                            return;
                        }
                        // Poziom jest wyłącznie wizualizacją — jeśli nie da
                        // się go wysłać, milczymy: zaśmiecanie logu dwudziestoma
                        // komunikatami na sekundę byłoby gorsze od braku wskaźnika.
                        let _ = send(
                            &level_writer,
                            &Notification::audio_level(level.rms, level.peak),
                        );
                    },
                ) {
                    Ok(new_session) => {
                        session = Some(new_session);
                        send(&writer, &Response::ok(request.id))?;
                        eprintln!("diktate-daemon: nagrywam (strumieniowo)");
                        if let Err(err) = send(
                            &writer,
                            &Notification::state_changed(SessionState::Recording, 0),
                        ) {
                            eprintln!(
                                "diktate-daemon: nie udało się wysłać powiadomienia o stanie: {err:#}"
                            );
                        }
                    }
                    Err(err) => {
                        send(
                            &writer,
                            &Response::err(request.id, -32000, format!("{err:#}")),
                        )?;
                    }
                }
            }
            METHOD_SESSION_SUBSCRIBE => {
                let wanted = request.params.unwrap_or_default().levels;
                levels_wanted.store(wanted, Ordering::Relaxed);
                send(&writer, &Response::ok(request.id))?;
            }
            METHOD_SESSION_STOP => {
                let Some(active) = session.take() else {
                    send(
                        &writer,
                        &Response::err(request.id, -32001, "nie ma aktywnej sesji"),
                    )?;
                    continue;
                };
                // `Session::stop` blokuje, aż dokończy się transkrypcja tego,
                // co zdążyło się nazbierać w kolejce — to celowe, więc
                // odpowiadamy dopiero po jej zakończeniu.
                match active.stop() {
                    Ok(()) => {
                        send(&writer, &Response::ok(request.id))?;
                        eprintln!("diktate-daemon: sesja zatrzymana");
                        // Powiadomienie o przejściu w stan bezczynności idzie
                        // dopiero teraz, po `stop()` — dopiero wtedy cała
                        // zakolejkowana transkrypcja na pewno dotarła do
                        // klienta (patrz komentarz modułu).
                        if let Err(err) =
                            send(&writer, &Notification::state_changed(SessionState::Idle, 0))
                        {
                            eprintln!(
                                "diktate-daemon: nie udało się wysłać powiadomienia o stanie: {err:#}"
                            );
                        }
                    }
                    Err(err) => {
                        send(
                            &writer,
                            &Response::err(request.id, -32002, format!("{err:#}")),
                        )?;
                    }
                }
            }
            other => {
                send(
                    &writer,
                    &Response::err(request.id, -32601, format!("nieznana metoda: {other}")),
                )?;
            }
        }
    }

    // Klient się rozłączył (EOF na gnieździe) — jeśli sesja była aktywna,
    // trzeba ją zatrzymać, żeby nie zostawić działającego mikrofonu.
    if let Some(active) = session.take() {
        eprintln!("diktate-daemon: klient rozłączony w trakcie sesji, zatrzymuję ją");
        match active.stop() {
            Ok(()) => {
                // Klient już nie czyta z gniazda, ale wysyłamy mimo to —
                // to spójne z resztą kodu i nieszkodliwe (co najwyżej
                // zapis się nie uda, co i tak tylko logujemy).
                if let Err(err) = send(&writer, &Notification::state_changed(SessionState::Idle, 0))
                {
                    eprintln!(
                        "diktate-daemon: nie udało się wysłać powiadomienia o stanie: {err:#}"
                    );
                }
            }
            Err(err) => {
                eprintln!("diktate-daemon: błąd zatrzymania sesji po rozłączeniu klienta: {err:#}");
            }
        }
    }

    Ok(())
}

/// Serializuje i wysyła jedną wiadomość NDJSON pod zamkiem strumienia —
/// całą linię razem ze znakiem nowej linii, żeby nie przeplotła się z inną
/// wiadomością pisaną z drugiego wątku (patrz komentarz modułu).
fn send<T: serde::Serialize>(writer: &Mutex<UnixStream>, message: &T) -> Result<()> {
    let line = serde_json::to_string(message).context("nie udało się zserializować wiadomości")?;
    let mut writer = writer
        .lock()
        .map_err(|_| anyhow!("blokada strumienia wyjściowego zatruta"))?;
    writer
        .write_all(line.as_bytes())
        .and_then(|()| writer.write_all(b"\n"))
        .and_then(|()| writer.flush())
        .context("nie udało się wysłać wiadomości do klienta")
}
