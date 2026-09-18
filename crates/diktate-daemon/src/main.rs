//! Daemon: gniazdo uniksowe, nagrywanie, transkrypcja.
//!
//! Etap 0: jeden klient naraz, obsługiwany sekwencyjnie. Bez journala, bez
//! sesji, bez stanu poza „nagrywam / nie nagrywam".

use anyhow::{Context, Result};
use diktate_core::{transcribe::default_model_path, Recorder, Recording, Transcriber};
use diktate_proto::{
    socket_path, Notification, Request, Response, METHOD_SESSION_START, METHOD_SESSION_STOP,
};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};

fn main() -> Result<()> {
    let model = default_model_path();
    eprintln!("diktate-daemon: wczytuję model {}", model.display());
    let transcriber = Transcriber::new(&model)?;

    let path = socket_path();
    if path.exists() {
        // Gniazdo po poprzednim uruchomieniu — daemon jest jeden, więc usuwamy.
        std::fs::remove_file(&path)
            .with_context(|| format!("nie udało się usunąć {}", path.display()))?;
    }
    let listener = UnixListener::bind(&path)
        .with_context(|| format!("nie udało się otworzyć gniazda {}", path.display()))?;
    eprintln!("diktate-daemon: nasłuchuję na {}", path.display());

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
    Ok(())
}

fn serve(stream: UnixStream, transcriber: &Transcriber) -> Result<()> {
    let mut writer = stream
        .try_clone()
        .context("nie udało się sklonować gniazda")?;
    let reader = BufReader::new(stream);
    let mut recording: Option<Recording> = None;

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
            METHOD_SESSION_START => match Recorder::start() {
                Ok(new_recording) => {
                    recording = Some(new_recording);
                    send(&mut writer, &Response::ok(request.id))?;
                    eprintln!("diktate-daemon: nagrywam");
                }
                Err(err) => {
                    send(
                        &mut writer,
                        &Response::err(request.id, -32000, format!("{err:#}")),
                    )?;
                }
            },
            METHOD_SESSION_STOP => {
                let Some(active) = recording.take() else {
                    send(
                        &mut writer,
                        &Response::err(request.id, -32001, "nie ma aktywnego nagrania"),
                    )?;
                    continue;
                };
                // Odpowiadamy przed transkrypcją: klient nie ma na co czekać,
                // tekst przyjdzie powiadomieniem.
                send(&mut writer, &Response::ok(request.id))?;
                match active.stop() {
                    Ok(samples) => {
                        eprintln!(
                            "diktate-daemon: nagranie {:.1} s, transkrybuję",
                            samples.len() as f32 / diktate_core::SAMPLE_RATE as f32
                        );
                        match transcriber.transcribe(&samples) {
                            Ok(text) if text.is_empty() => {
                                eprintln!("diktate-daemon: pusta transkrypcja")
                            }
                            Ok(text) => {
                                eprintln!("diktate-daemon: {text}");
                                send(&mut writer, &Notification::segment_final(text))?;
                            }
                            Err(err) => eprintln!("diktate-daemon: {err:#}"),
                        }
                    }
                    Err(err) => eprintln!("diktate-daemon: {err:#}"),
                }
            }
            other => {
                send(
                    &mut writer,
                    &Response::err(request.id, -32601, format!("nieznana metoda: {other}")),
                )?;
            }
        }
    }
    Ok(())
}

fn send<T: serde::Serialize>(writer: &mut UnixStream, message: &T) -> Result<()> {
    let line = serde_json::to_string(message).context("nie udało się zserializować wiadomości")?;
    writer
        .write_all(line.as_bytes())
        .and_then(|()| writer.write_all(b"\n"))
        .and_then(|()| writer.flush())
        .context("nie udało się wysłać wiadomości do klienta")
}
