//! Klient IPC: wysyłanie żądań i odbieranie powiadomień.
//!
//! Zapis idzie z wątku GUI, odczyt z osobnego wątku, który przerzuca tekst do
//! pętli GTK kanałem asynchronicznym.

use anyhow::{anyhow, Context, Result};
use async_channel::Receiver;
use diktate_proto::{
    socket_path, NotificationParams, Outgoing, Request, SessionState, METHOD_SESSION_START,
    METHOD_SESSION_STOP,
};
use std::cell::Cell;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;

/// Wiadomość przerzucona z wątku czytającego gniazdo do pętli GTK.
///
/// `Segment` to gotowy tekst wypowiedzi, `State` to zmiana stanu sesji wraz
/// z liczbą wypowiedzi czekających na transkrypcję.
pub enum Incoming {
    Segment(String),
    State { state: SessionState, backlog: u32 },
}

pub struct Client {
    writer: std::cell::RefCell<UnixStream>,
    next_id: Cell<u64>,
    incoming: Receiver<Incoming>,
}

impl Client {
    pub fn connect() -> Result<Self> {
        let path = socket_path();
        let stream = UnixStream::connect(&path)
            .with_context(|| format!("nie udało się połączyć z {}", path.display()))?;
        let reader_stream = stream
            .try_clone()
            .context("nie udało się sklonować gniazda")?;

        let (tx, rx) = async_channel::unbounded::<Incoming>();
        std::thread::spawn(move || {
            let reader = BufReader::new(reader_stream);
            for line in reader.lines() {
                let Ok(line) = line else { break };
                if line.trim().is_empty() {
                    continue;
                }
                match serde_json::from_str::<Outgoing>(&line) {
                    Ok(Outgoing::Notification(notification)) => match notification.params {
                        NotificationParams::SegmentFinal { text } => {
                            if tx.send_blocking(Incoming::Segment(text)).is_err() {
                                break;
                            }
                        }
                        NotificationParams::StateChanged { state, backlog } => {
                            if tx.send_blocking(Incoming::State { state, backlog }).is_err() {
                                break;
                            }
                        }
                    },
                    // Odpowiedzi na start/stop nie niosą nic poza potwierdzeniem.
                    Ok(Outgoing::Response(_)) => {}
                    Err(err) => eprintln!("diktate-gtk: nieczytelna wiadomość: {err}"),
                }
            }
        });

        Ok(Self {
            writer: std::cell::RefCell::new(stream),
            next_id: Cell::new(1),
            incoming: rx,
        })
    }

    pub fn incoming(&self) -> Receiver<Incoming> {
        self.incoming.clone()
    }

    pub fn start(&self) -> Result<()> {
        self.send(METHOD_SESSION_START)
    }

    pub fn stop(&self) -> Result<()> {
        self.send(METHOD_SESSION_STOP)
    }

    fn send(&self, method: &str) -> Result<()> {
        let id = self.next_id.get();
        self.next_id.set(id + 1);
        let line = serde_json::to_string(&Request::new(id, method))
            .map_err(|err| anyhow!("nie udało się zserializować żądania: {err}"))?;
        let mut writer = self.writer.borrow_mut();
        writer
            .write_all(line.as_bytes())
            .and_then(|()| writer.write_all(b"\n"))
            .and_then(|()| writer.flush())
            .context("nie udało się wysłać żądania")
    }
}
