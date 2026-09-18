//! Klient IPC: wysyłanie żądań i odbieranie powiadomień.
//!
//! Zapis idzie z wątku GUI, odczyt z osobnego wątku, który przerzuca tekst do
//! pętli GTK kanałem asynchronicznym.

use anyhow::{anyhow, Context, Result};
use async_channel::Receiver;
use diktate_proto::{
    socket_path, NotificationParams, Outgoing, Request, SegmentAdded, SessionState, Subscription,
    METHOD_SESSION_START, METHOD_SESSION_STOP,
};
use std::cell::Cell;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

use crate::daemon;

/// Wiadomość przerzucona z wątku czytającego gniazdo do pętli GTK.
///
/// `Segment` to gotowa wypowiedź wraz z jej miejscem w osi czasu sesji,
/// `State` to zmiana stanu sesji wraz z liczbą wypowiedzi czekających na
/// transkrypcję, `Level` to odczyt poziomu sygnału do wskaźnika. Ze szczytu
/// (`peak`) okno dziś nie korzysta — wskaźnik rysuje wartość skuteczną, więc
/// szczyt nie jest tu przenoszony, choć protokół go niesie.
pub enum Incoming {
    Segment(SegmentAdded),
    State { state: SessionState, backlog: u32 },
    Level { rms: f32 },
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
                        NotificationParams::SegmentAdded(segment) => {
                            if tx.send_blocking(Incoming::Segment(segment)).is_err() {
                                break;
                            }
                        }
                        NotificationParams::StateChanged { state, backlog } => {
                            if tx
                                .send_blocking(Incoming::State { state, backlog })
                                .is_err()
                            {
                                break;
                            }
                        }
                        NotificationParams::AudioLevel { rms, .. } => {
                            if tx.send_blocking(Incoming::Level { rms }).is_err() {
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

    /// Łączy się z daemonem, a gdy ten nie działa — uruchamia go i czeka na
    /// gniazdo.
    ///
    /// Pierwsza próba idzie bez zwłoki: w typowym przypadku daemon już
    /// nasłuchuje i nic tu nie kosztuje. Dopiero jej niepowodzenie oznacza
    /// brak procesu; wtedy wołane jest `on_spawn` (okno ma powiedzieć, że
    /// czeka na start), a kolejne próby ponawiają się z narastającym
    /// odstępem aż do `timeout`. Czas musi pomieścić wczytanie modelu
    /// whispera, bo daemon otwiera gniazdo dopiero po nim.
    ///
    /// Wywoływana z osobnego wątku — czekanie blokuje, a pętla GTK nie ma
    /// prawa stanąć.
    pub fn connect_or_spawn(timeout: Duration, on_spawn: impl FnOnce()) -> Result<Self> {
        if let Ok(client) = Self::connect() {
            return Ok(client);
        }

        on_spawn();
        let pid = daemon::spawn()?;
        eprintln!("diktate-gtk: uruchomiłem daemona (pid {pid})");

        let deadline = Instant::now() + timeout;
        let mut delay = Duration::from_millis(50);
        loop {
            std::thread::sleep(delay);
            match Self::connect() {
                Ok(client) => return Ok(client),
                Err(err) => {
                    if Instant::now() >= deadline {
                        return Err(err.context(format!(
                            "daemon (pid {pid}) nie otworzył gniazda w ciągu {} s",
                            timeout.as_secs_f32()
                        )));
                    }
                }
            }
            delay = (delay * 2).min(Duration::from_millis(500));
        }
    }

    pub fn incoming(&self) -> Receiver<Incoming> {
        self.incoming.clone()
    }

    pub fn start(&self) -> Result<()> {
        self.send(METHOD_SESSION_START)
    }

    /// Prosi daemona o odczyty poziomu sygnału. Bez tego nie przychodzą —
    /// to kilkanaście wiadomości na sekundę i dostaje je tylko ten, kto
    /// rysuje wskaźnik.
    pub fn subscribe_levels(&self) -> Result<()> {
        self.send_request(&Request::subscribe(
            self.take_id(),
            Subscription { levels: true },
        ))
    }

    pub fn stop(&self) -> Result<()> {
        self.send(METHOD_SESSION_STOP)
    }

    fn take_id(&self) -> u64 {
        let id = self.next_id.get();
        self.next_id.set(id + 1);
        id
    }

    fn send(&self, method: &str) -> Result<()> {
        self.send_request(&Request::new(self.take_id(), method))
    }

    fn send_request(&self, request: &Request) -> Result<()> {
        let line = serde_json::to_string(request)
            .map_err(|err| anyhow!("nie udało się zserializować żądania: {err}"))?;
        let mut writer = self.writer.borrow_mut();
        writer
            .write_all(line.as_bytes())
            .and_then(|()| writer.write_all(b"\n"))
            .and_then(|()| writer.flush())
            .context("nie udało się wysłać żądania")
    }
}
