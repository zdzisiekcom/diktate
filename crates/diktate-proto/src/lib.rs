//! Typy protokołu IPC — jedyne źródło kształtu wiadomości.
//!
//! Transport: NDJSON po gnieździe uniksowym, jedna wiadomość na linię.
//!
//! Zasada, z której wynika kształt `segment.added`: **oś czasu audio jest
//! jedynym prawdziwym porządkiem wypowiedzi**. Przy zaległościach kolejki
//! transkrypcji wypowiedź wcześniejsza potrafi dotrzeć do klienta później niż
//! późniejsza, więc klient porządkuje i rozstrzyga po `audio_start_ms`, a nie
//! po kolejności przybycia (`docs/02-protokol-ipc.md`).
//!
//! `audio.level` jest wysyłany kilkanaście razy na sekundę, więc trafia
//! wyłącznie do klientów, które poprosiły o nie jawnie przez
//! `session.subscribe { levels: true }`.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub mod pidfile;

/// Ścieżka gniazda: `$XDG_RUNTIME_DIR/diktate.sock`, z odwrotem na `/tmp`.
pub fn socket_path() -> PathBuf {
    match std::env::var_os("XDG_RUNTIME_DIR") {
        Some(dir) => PathBuf::from(dir).join("diktate.sock"),
        None => PathBuf::from("/tmp/diktate.sock"),
    }
}

pub const METHOD_SESSION_START: &str = "session.start";
pub const METHOD_SESSION_STOP: &str = "session.stop";
pub const METHOD_SESSION_SUBSCRIBE: &str = "session.subscribe";
pub const NOTIFY_SEGMENT_ADDED: &str = "segment.added";
pub const NOTIFY_STATE_CHANGED: &str = "state.changed";
pub const NOTIFY_AUDIO_LEVEL: &str = "audio.level";

/// Czego klient chce słuchać poza tym, co dostaje każdy.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct Subscription {
    /// Poziom sygnału z mikrofonu (`audio.level`). Domyślnie wyłączony —
    /// to kilkanaście wiadomości na sekundę i ma je dostawać tylko ten,
    /// kto rysuje wskaźnik.
    #[serde(default)]
    pub levels: bool,
}

/// Wiadomość idąca do daemona.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    pub jsonrpc: String,
    pub id: u64,
    pub method: String,
    /// Parametry wywołania. Dziś używa ich wyłącznie `session.subscribe`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Subscription>,
}

impl Request {
    pub fn new(id: u64, method: &str) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            method: method.to_string(),
            params: None,
        }
    }

    pub fn subscribe(id: u64, subscription: Subscription) -> Self {
        Self {
            params: Some(subscription),
            ..Self::new(id, METHOD_SESSION_SUBSCRIBE)
        }
    }
}

/// Wiadomość idąca od daemona: odpowiedź albo powiadomienie.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Outgoing {
    Response(Response),
    Notification(Notification),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    pub jsonrpc: String,
    pub id: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

impl Response {
    pub fn ok(id: u64) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            result: Some(serde_json::json!({})),
            error: None,
        }
    }

    pub fn err(id: u64, code: i32, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            result: None,
            error: Some(RpcError {
                code,
                message: message.into(),
            }),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcError {
    pub code: i32,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Notification {
    pub jsonrpc: String,
    pub method: String,
    pub params: NotificationParams,
}

impl Notification {
    /// Gotowa wypowiedź — najważniejsze zdarzenie w całym systemie.
    pub fn segment_added(segment: &SegmentAdded) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            method: NOTIFY_SEGMENT_ADDED.to_string(),
            params: NotificationParams::SegmentAdded(segment.clone()),
        }
    }

    /// Poziom sygnału do wskaźnika w oknie. Wartości 0.0–1.0.
    pub fn audio_level(rms: f32, peak: f32) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            method: NOTIFY_AUDIO_LEVEL.to_string(),
            params: NotificationParams::AudioLevel { rms, peak },
        }
    }

    /// Zmiana stanu sesji. `backlog` to segmenty czekające w kolejce **wraz
    /// z tym, który jest właśnie transkrybowany** — użytkownik ma wiedzieć,
    /// ile jego wypowiedzi nie dotarło jeszcze na ekran.
    pub fn state_changed(state: SessionState, backlog: u32) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            method: NOTIFY_STATE_CHANGED.to_string(),
            params: NotificationParams::StateChanged { state, backlog },
        }
    }
}

/// Treść powiadomienia `segment.added`.
///
/// `audio_start_ms` i `audio_end_ms` liczone są od początku sesji.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SegmentAdded {
    /// Monotoniczny numer wypowiedzi w kolejności **wypowiadania**. Dziura
    /// w numeracji to wypowiedź porzucona przez przepełnioną kolejkę.
    pub seq: u64,
    pub text: String,
    pub audio_start_ms: u64,
    pub audio_end_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum NotificationParams {
    SegmentAdded(SegmentAdded),
    StateChanged { state: SessionState, backlog: u32 },
    AudioLevel { rms: f32, peak: f32 },
}

/// Stan sesji widziany przez klienta.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionState {
    /// Sesja nieaktywna — mikrofon wyłączony.
    Idle,
    /// Mikrofon zbiera dźwięk; `backlog` mówi, ile czeka na transkrypcję.
    Recording,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn przykladowy_segment() -> SegmentAdded {
        SegmentAdded {
            seq: 1,
            text: "cześć".to_string(),
            audio_start_ms: 0,
            audio_end_ms: 1_000,
        }
    }

    #[test]
    fn request_roundtrip() {
        let line = serde_json::to_string(&Request::new(1, METHOD_SESSION_START)).unwrap();
        let back: Request = serde_json::from_str(&line).unwrap();
        assert_eq!(back.method, METHOD_SESSION_START);
        assert_eq!(back.id, 1);
    }

    #[test]
    fn outgoing_distinguishes_response_from_notification() {
        let line =
            serde_json::to_string(&Notification::segment_added(&przykladowy_segment())).unwrap();
        match serde_json::from_str::<Outgoing>(&line).unwrap() {
            Outgoing::Notification(n) => match n.params {
                NotificationParams::SegmentAdded(segment) => assert_eq!(segment.text, "cześć"),
                other => panic!("zły wariant: {other:?}"),
            },
            Outgoing::Response(_) => panic!("powiadomienie rozpoznane jako odpowiedź"),
        }

        let line = serde_json::to_string(&Response::ok(7)).unwrap();
        match serde_json::from_str::<Outgoing>(&line).unwrap() {
            Outgoing::Response(r) => assert_eq!(r.id, 7),
            Outgoing::Notification(_) => panic!("odpowiedź rozpoznana jako powiadomienie"),
        }
    }
}

#[cfg(test)]
mod state_tests {
    use super::*;

    #[test]
    fn state_changed_roundtrip() {
        let line = serde_json::to_string(&Notification::state_changed(SessionState::Recording, 3))
            .unwrap();
        assert!(line.contains("\"recording\""));
        match serde_json::from_str::<Outgoing>(&line).unwrap() {
            Outgoing::Notification(n) => match n.params {
                NotificationParams::StateChanged { state, backlog } => {
                    assert_eq!(state, SessionState::Recording);
                    assert_eq!(backlog, 3);
                }
                other => panic!("zły wariant: {other:?}"),
            },
            Outgoing::Response(_) => panic!("rozpoznane jako odpowiedź"),
        }
    }

    #[test]
    fn segment_added_nie_myli_sie_ze_stanem() {
        let segment = SegmentAdded {
            seq: 1,
            text: "cześć".to_string(),
            audio_start_ms: 0,
            audio_end_ms: 1_000,
        };
        let line = serde_json::to_string(&Notification::segment_added(&segment)).unwrap();
        match serde_json::from_str::<Outgoing>(&line).unwrap() {
            Outgoing::Notification(n) => {
                assert!(matches!(n.params, NotificationParams::SegmentAdded(_)));
            }
            Outgoing::Response(_) => panic!("rozpoznane jako odpowiedź"),
        }
    }
}

#[cfg(test)]
mod segment_added_tests {
    use super::*;

    #[test]
    fn segment_added_niesie_os_czasu_audio() {
        let line = serde_json::to_string(&Notification::segment_added(&SegmentAdded {
            seq: 42,
            text: "To jest treść.".to_string(),
            audio_start_ms: 18_340,
            audio_end_ms: 21_120,
        }))
        .unwrap();
        assert!(line.contains(NOTIFY_SEGMENT_ADDED));

        match serde_json::from_str::<Outgoing>(&line).unwrap() {
            Outgoing::Notification(n) => match n.params {
                NotificationParams::SegmentAdded(segment) => {
                    assert_eq!(segment.seq, 42);
                    assert_eq!(segment.text, "To jest treść.");
                    assert_eq!(segment.audio_start_ms, 18_340);
                    assert_eq!(segment.audio_end_ms, 21_120);
                }
                other => panic!("zły wariant: {other:?}"),
            },
            Outgoing::Response(_) => panic!("rozpoznane jako odpowiedź"),
        }
    }

    #[test]
    fn audio_level_nie_myli_sie_z_segmentem_ani_ze_stanem() {
        let line = serde_json::to_string(&Notification::audio_level(0.25, 0.8)).unwrap();
        match serde_json::from_str::<Outgoing>(&line).unwrap() {
            Outgoing::Notification(n) => match n.params {
                NotificationParams::AudioLevel { rms, peak } => {
                    assert!((rms - 0.25).abs() < 1e-6);
                    assert!((peak - 0.8).abs() < 1e-6);
                }
                other => panic!("zły wariant: {other:?}"),
            },
            Outgoing::Response(_) => panic!("rozpoznane jako odpowiedź"),
        }
    }

    #[test]
    fn subskrypcja_poziomow_jest_jawna() {
        // `audio.level` leci ~20 razy na sekundę, więc dostaje je wyłącznie
        // klient, który o to poprosił (docs/02).
        let line =
            serde_json::to_string(&Request::subscribe(3, Subscription { levels: true })).unwrap();
        let back: Request = serde_json::from_str(&line).unwrap();
        assert_eq!(back.method, METHOD_SESSION_SUBSCRIBE);
        assert!(back.params.unwrap().levels);
    }
}
