//! Transkrypcja przez whisper.cpp.
//!
//! Model ładowany raz i trzymany w pamięci. Parametry dekodowania pochodzą
//! z `daemon.toml` (`docs/04`) — w kodzie nie ma dla nich magicznych
//! stałych, są tylko wartości domyślne.
//!
//! Wywołania idą przez warstwę pośrednią z `diktate-whispercpp-sys`, a nie
//! przez odwzorowanie `whisper_full_params` w Ruście: ta struktura ma 61 pól
//! i jest przekazywana przez wartość, więc każda zmiana w whisper.cpp
//! psułaby nas po cichu. Szczegóły w `crates/diktate-whispercpp-sys/src/shim.h`.

use anyhow::{anyhow, bail, Context, Result};
use std::ffi::{CStr, CString};
use std::path::Path;
use std::sync::Mutex;

use diktate_whispercpp_sys as sys;

/// Silnik zamieniający próbki na tekst. Potok sesji zna tylko ten trait, więc
/// backend (whisper albo Parakeet — `docs/12`) da się podmienić bez ruszania
/// VAD-u, segmentera ani kolejki.
///
/// `Send + Sync`, bo transkryber jest dzielony (przez `Arc`) z wątkiem
/// transkrypcji sesji.
pub trait Transcribe: Send + Sync {
    /// Transkrybuje mono 16 kHz ([`crate::SAMPLE_RATE`]). Pusty wynik znaczy
    /// „nie rozpoznano mowy" i nie jest błędem.
    ///
    /// `prompt` to podpowiedź kontekstowa złożona przez klienta
    /// (`docs/07#kontekst-dla-whispera`). Pusty prompt znaczy „weź własny
    /// domyślny", a backend, który promptu nie przyjmuje, ma prawo go
    /// zignorować — ale musi to powiedzieć w komentarzu.
    fn transcribe(&self, samples: &[f32], prompt: &str) -> Result<String>;
}

/// Uchwyt kontekstu whisper.cpp.
///
/// `whisper_full` nie jest bezpieczne dla wielu wątków na tym samym
/// kontekście, więc dostęp idzie przez `Mutex`. W praktyce transkrybuje
/// jeden wątek sesji, ale trait wymaga `Sync` i lepiej to zagwarantować
/// zamkiem niż obietnicą.
struct WhisperCtx(*mut sys::DkWhisper);

// SAFETY: wskaźnik jest własnością tej struktury i nigdy nie wycieka; cały
// dostęp do niego jest szeregowany `Mutex`-em w `Transcriber`.
unsafe impl Send for WhisperCtx {}

impl Drop for WhisperCtx {
    fn drop(&mut self) {
        unsafe { sys::dk_whisper_close(self.0) };
    }
}

pub struct Transcriber {
    ctx: Mutex<WhisperCtx>,
    language: String,
    /// Podpowiedź kontekstowa doklejana do promptu: terminologia, nazwy
    /// własne. Puste = bez podpowiedzi.
    initial_prompt: String,
    /// Szerokość wiązki; 1 oznacza dekodowanie zachłanne.
    beam_size: u32,
    /// Liczba wątków; 0 zostawia decyzję whisperowi.
    threads: u32,
}

impl Transcriber {
    /// Ładuje model `ggml-*.bin` spod podanej ścieżki.
    pub fn new(model_path: impl AsRef<Path>) -> Result<Self> {
        let path = model_path.as_ref();
        if !path.exists() {
            return Err(anyhow!("nie znaleziono modelu: {}", path.display()));
        }
        let c_path = cstring(path_str(path)?)?;
        let ctx = unsafe { sys::dk_whisper_open(c_path.as_ptr(), true) };
        if ctx.is_null() {
            bail!("nie udało się wczytać modelu {}", path.display());
        }
        Ok(Self {
            ctx: Mutex::new(WhisperCtx(ctx)),
            language: "pl".to_string(),
            initial_prompt: String::new(),
            beam_size: 1,
            threads: 0,
        })
    }

    pub fn with_language(mut self, language: impl Into<String>) -> Self {
        self.language = language.into();
        self
    }

    pub fn with_initial_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.initial_prompt = prompt.into();
        self
    }

    /// Szerokość wiązki; 0 i 1 znaczą to samo — dekodowanie zachłanne.
    pub fn with_beam_size(mut self, beam_size: u32) -> Self {
        self.beam_size = beam_size;
        self
    }

    pub fn with_threads(mut self, threads: u32) -> Self {
        self.threads = threads;
        self
    }

    /// Transkrybuje mono 16 kHz. Zwraca sklejony tekst wszystkich segmentów.
    ///
    /// Pusty `prompt` cofa się do `initial_prompt` z konfiguracji daemona —
    /// tak działają klienty, które kontekstu nie przysyłają (`diktate-cli`,
    /// przykłady), i tak wyglądało zachowanie przed wprowadzeniem kontekstu.
    pub fn transcribe(&self, samples: &[f32], prompt: &str) -> Result<String> {
        if samples.is_empty() {
            return Ok(String::new());
        }
        let language = cstring(&self.language)?;
        let chosen = if prompt.is_empty() {
            self.initial_prompt.as_str()
        } else {
            prompt
        };
        let prompt = cstring(chosen)?;

        let guard = self
            .ctx
            .lock()
            .map_err(|_| anyhow!("kontekst whispera został zatruty przez panikę"))?;
        let code = unsafe {
            sys::dk_whisper_transcribe(
                guard.0,
                samples.as_ptr(),
                samples.len() as i32,
                language.as_ptr(),
                prompt.as_ptr(),
                self.beam_size as i32,
                self.threads as i32,
            )
        };
        if code != 0 {
            let message = unsafe { read_cstr(sys::dk_whisper_error(guard.0)) };
            bail!("transkrypcja nie powiodła się: {message}");
        }
        let text = unsafe { read_cstr(sys::dk_whisper_text(guard.0)) };
        Ok(join_segments(&text))
    }
}

impl Transcribe for Transcriber {
    fn transcribe(&self, samples: &[f32], prompt: &str) -> Result<String> {
        Transcriber::transcribe(self, samples, prompt)
    }
}

/// Składa segmenty (po jednym w wierszu) w jedno zdanie. Whisper zwraca
/// segmenty z wiodącą spacją, stąd przycinanie każdego z osobna.
pub(crate) fn join_segments(text: &str) -> String {
    let mut out = String::new();
    for segment in text.lines() {
        let segment = segment.trim();
        if segment.is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(segment);
    }
    out
}

pub(crate) fn path_str(path: &Path) -> Result<&str> {
    path.to_str()
        .ok_or_else(|| anyhow!("ścieżka modelu nie jest poprawnym UTF-8"))
}

pub(crate) fn cstring(value: &str) -> Result<CString> {
    CString::new(value).context("tekst dla whisper.cpp zawiera bajt zerowy")
}

/// SAFETY: wskaźnik pochodzi z warstwy pośredniej i wskazuje bufor żyjący do
/// następnego wywołania na tym kontekście; kopiujemy go od razu.
pub(crate) unsafe fn read_cstr(ptr: *const std::os::raw::c_char) -> String {
    if ptr.is_null() {
        return String::new();
    }
    CStr::from_ptr(ptr).to_string_lossy().into_owned()
}

/// Domyślna ścieżka modelu: `$DIKTATE_MODEL`, a bez niej
/// `~/.diktate/models/ggml-small-q5_1.bin` — katalog, do którego pobiera
/// `diktate setup` (`docs/04-konfiguracja.md#kreator-diktate-setup`).
/// Pole `whisper.model` z `daemon.toml` rozstrzyga wcześniej i nie
/// przechodzi tędy.
pub fn default_model_path() -> std::path::PathBuf {
    if let Some(path) = std::env::var_os("DIKTATE_MODEL") {
        return std::path::PathBuf::from(path);
    }
    models_dir().join("ggml-small-q5_1.bin")
}

/// `~/.diktate/models` — wspólny katalog modeli obu backendów.
pub(crate) fn models_dir() -> std::path::PathBuf {
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    home.unwrap_or_else(|| std::path::PathBuf::from("."))
        .join(".diktate/models")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Atrapa silnika: zapisuje prompty, które dostała. Trait musi dać się
    /// zaimplementować bez modelu — inaczej nie da się testować sesji.
    struct Atrapa {
        prompty: Mutex<Vec<String>>,
    }

    impl Transcribe for Atrapa {
        fn transcribe(&self, _samples: &[f32], prompt: &str) -> Result<String> {
            self.prompty.lock().unwrap().push(prompt.to_string());
            Ok("tekst".to_string())
        }
    }

    #[test]
    fn trait_przekazuje_prompt_do_silnika() {
        let atrapa = Atrapa {
            prompty: Mutex::new(Vec::new()),
        };
        let silnik: &dyn Transcribe = &atrapa;
        silnik.transcribe(&[0.0], "Terminy: ggml.").unwrap();
        assert_eq!(atrapa.prompty.lock().unwrap().as_slice(), ["Terminy: ggml."]);
    }

    #[test]
    fn join_segments_przycina_i_skleja_spacja() {
        assert_eq!(join_segments(" Ala ma kota.\n A kot ma Alę."), "Ala ma kota. A kot ma Alę.");
    }

    #[test]
    fn join_segments_pomija_puste_wiersze() {
        assert_eq!(join_segments("\n  \nTekst\n\n"), "Tekst");
    }

    #[test]
    fn join_segments_pustego_wejscia_daje_pusty_tekst() {
        assert_eq!(join_segments(""), "");
    }

    #[test]
    fn cstring_odrzuca_bajt_zerowy() {
        assert!(cstring("a\0b").is_err());
    }
}
