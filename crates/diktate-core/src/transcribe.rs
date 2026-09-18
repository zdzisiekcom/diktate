//! Transkrypcja przez whisper.cpp (`whisper-rs`).
//!
//! Etap 0: backend CPU, jeden przebieg po całym nagraniu, język ustawiony na
//! polski na sztywno. Model ładowany raz i trzymany w pamięci.

use anyhow::{anyhow, Context, Result};
use std::path::Path;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

pub struct Transcriber {
    ctx: WhisperContext,
    language: String,
}

impl Transcriber {
    /// Ładuje model `ggml-*.bin` spod podanej ścieżki.
    pub fn new(model_path: impl AsRef<Path>) -> Result<Self> {
        let path = model_path.as_ref();
        if !path.exists() {
            return Err(anyhow!("nie znaleziono modelu: {}", path.display()));
        }
        let path_str = path
            .to_str()
            .ok_or_else(|| anyhow!("ścieżka modelu nie jest poprawnym UTF-8"))?;
        let ctx = WhisperContext::new_with_params(path_str, WhisperContextParameters::default())
            .with_context(|| format!("nie udało się wczytać modelu {}", path.display()))?;
        Ok(Self {
            ctx,
            language: "pl".to_string(),
        })
    }

    pub fn with_language(mut self, language: impl Into<String>) -> Self {
        self.language = language.into();
        self
    }

    /// Transkrybuje mono 16 kHz. Zwraca sklejony tekst wszystkich segmentów.
    pub fn transcribe(&self, samples: &[f32]) -> Result<String> {
        if samples.is_empty() {
            return Ok(String::new());
        }

        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_language(Some(&self.language));
        params.set_translate(false);
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);

        let mut state = self
            .ctx
            .create_state()
            .context("nie udało się utworzyć stanu whispera")?;
        state
            .full(params, samples)
            .context("transkrypcja nie powiodła się")?;

        let count = state
            .full_n_segments()
            .context("nie udało się odczytać liczby segmentów")?;
        let mut text = String::new();
        for i in 0..count {
            let segment = state
                .full_get_segment_text(i)
                .with_context(|| format!("nie udało się odczytać segmentu {i}"))?;
            text.push_str(segment.trim());
            text.push(' ');
        }
        Ok(text.trim().to_string())
    }
}

/// Domyślna ścieżka modelu — do etapu 0 stała, bez konfiguracji.
pub fn default_model_path() -> std::path::PathBuf {
    if let Some(path) = std::env::var_os("DIKTATE_MODEL") {
        return std::path::PathBuf::from(path);
    }
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    home.unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("models/whisper/ggml-base.bin")
}
