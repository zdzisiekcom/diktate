//! Typy protokołu IPC — jedyne źródło kształtu wiadomości.
//!
//! Etap 0: dwie metody (`session.start`, `session.stop`) i jedno powiadomienie
//! (`segment.final`). Transport: NDJSON po gnieździe uniksowym, jedna wiadomość
//! na linię.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Ścieżka gniazda: `$XDG_RUNTIME_DIR/diktate.sock`, z odwrotem na `/tmp`.
pub fn socket_path() -> PathBuf {
    match std::env::var_os("XDG_RUNTIME_DIR") {
        Some(dir) => PathBuf::from(dir).join("diktate.sock"),
        None => PathBuf::from("/tmp/diktate.sock"),
    }
}

pub const METHOD_SESSION_START: &str = "session.start";
pub const METHOD_SESSION_STOP: &str = "session.stop";
pub const NOTIFY_SEGMENT_FINAL: &str = "segment.final";

/// Wiadomość idąca do daemona.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    pub jsonrpc: String,
    pub id: u64,
    pub method: String,
}

impl Request {
    pub fn new(id: u64, method: &str) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            method: method.to_string(),
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
    pub fn segment_final(text: impl Into<String>) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            method: NOTIFY_SEGMENT_FINAL.to_string(),
            params: NotificationParams::SegmentFinal { text: text.into() },
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum NotificationParams {
    SegmentFinal { text: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_roundtrip() {
        let line = serde_json::to_string(&Request::new(1, METHOD_SESSION_START)).unwrap();
        let back: Request = serde_json::from_str(&line).unwrap();
        assert_eq!(back.method, METHOD_SESSION_START);
        assert_eq!(back.id, 1);
    }

    #[test]
    fn outgoing_distinguishes_response_from_notification() {
        let line = serde_json::to_string(&Notification::segment_final("cześć")).unwrap();
        match serde_json::from_str::<Outgoing>(&line).unwrap() {
            Outgoing::Notification(n) => {
                let NotificationParams::SegmentFinal { text } = n.params;
                assert_eq!(text, "cześć");
            }
            Outgoing::Response(_) => panic!("powiadomienie rozpoznane jako odpowiedź"),
        }

        let line = serde_json::to_string(&Response::ok(7)).unwrap();
        match serde_json::from_str::<Outgoing>(&line).unwrap() {
            Outgoing::Response(r) => assert_eq!(r.id, 7),
            Outgoing::Notification(_) => panic!("odpowiedź rozpoznana jako powiadomienie"),
        }
    }
}
