//! Daemon: gniazdo uniksowe, sesja strumieniowa, transkrypcja na bieżąco.
//!
//! Jeden klient naraz, obsługiwany sekwencyjnie. Sesja (`Session`) po
//! `session.start` nagrywa i transkrybuje w tle na własnych wątkach — gotowe
//! segmenty wracają do klienta powiadomieniami `segment.final`, niezależnie
//! od pętli obsługującej żądania na tym samym gnieździe. Dlatego strumień do
//! zapisu jest dzielony przez `Arc<Mutex<UnixStream>>`: i pętla żądań
//! (odpowiedzi), i wątek transkrypcji sesji (powiadomienia) piszą do niego
//! pod tym samym zamkiem, żeby żadna linia NDJSON się nie przeplotła.
use anyhow::{anyhow, Context, Result};
use diktate_core::{transcribe::default_model_path, Session, Transcriber};
use diktate_proto::{
    socket_path, Notification, Request, Response, METHOD_SESSION_START, METHOD_SESSION_STOP,
};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::{Arc, Mutex};

fn main() -> Result<()> {
    let model = default_model_path();
    eprintln!("diktate-daemon: wczytuję model {}", model.display());
    let transcriber = Arc::new(Transcriber::new(&model)?);

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

fn serve(stream: UnixStream, transcriber: &Arc<Transcriber>) -> Result<()> {
    let writer = stream
        .try_clone()
        .context("nie udało się sklonować gniazda")?;
    // Dzielony między tę pętlę (odpowiedzi) a wątek transkrypcji sesji
    // (powiadomienia) — patrz komentarz modułu.
    let writer = Arc::new(Mutex::new(writer));
    let reader = BufReader::new(stream);
    let mut session: Option<Session> = None;

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
                match Session::start(Arc::clone(transcriber), move |text| {
                    eprintln!("diktate-daemon: {text}");
                    if let Err(err) = send(&notify_writer, &Notification::segment_final(text)) {
                        eprintln!("diktate-daemon: nie udało się wysłać powiadomienia: {err:#}");
                    }
                }) {
                    Ok(new_session) => {
                        session = Some(new_session);
                        send(&writer, &Response::ok(request.id))?;
                        eprintln!("diktate-daemon: nagrywam (strumieniowo)");
                    }
                    Err(err) => {
                        send(
                            &writer,
                            &Response::err(request.id, -32000, format!("{err:#}")),
                        )?;
                    }
                }
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
        if let Err(err) = active.stop() {
            eprintln!("diktate-daemon: błąd zatrzymania sesji po rozłączeniu klienta: {err:#}");
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
