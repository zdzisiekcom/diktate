//! Transkrypcja przez whisper.cpp (`whisper-rs`).
//!
//! Model ładowany raz i trzymany w pamięci. Parametry dekodowania pochodzą
//! z `daemon.toml` (`docs/04`) — w kodzie nie ma dla nich magicznych
//! stałych, są tylko wartości domyślne.

use anyhow::{anyhow, Context, Result};
use std::path::Path;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

pub struct Transcriber {
    ctx: WhisperContext,
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
        let path_str = path
            .to_str()
            .ok_or_else(|| anyhow!("ścieżka modelu nie jest poprawnym UTF-8"))?;
        let ctx = WhisperContext::new_with_params(path_str, WhisperContextParameters::default())
            .with_context(|| format!("nie udało się wczytać modelu {}", path.display()))?;
        Ok(Self {
            ctx,
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
    pub fn transcribe(&self, samples: &[f32]) -> Result<String> {
        if samples.is_empty() {
            return Ok(String::new());
        }

        let strategy = if self.beam_size > 1 {
            SamplingStrategy::BeamSearch {
                beam_size: self.beam_size as i32,
                patience: 0.0,
            }
        } else {
            SamplingStrategy::Greedy { best_of: 1 }
        };
        let mut params = FullParams::new(strategy);
        params.set_language(Some(&self.language));
        if !self.initial_prompt.is_empty() {
            params.set_initial_prompt(&self.initial_prompt);
        }
        if self.threads > 0 {
            params.set_n_threads(self.threads as i32);
        }
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

/// Domyślna ścieżka modelu: `$DIKTATE_MODEL`, a bez niej
/// `~/.diktate/models/ggml-small-q5_1.bin` — katalog, do którego pobiera
/// `diktate setup` (`docs/04-konfiguracja.md#kreator-diktate-setup`).
/// Pole `whisper.model` z `daemon.toml` rozstrzyga wcześniej i nie
/// przechodzi tędy.
pub fn default_model_path() -> std::path::PathBuf {
    if let Some(path) = std::env::var_os("DIKTATE_MODEL") {
        return std::path::PathBuf::from(path);
    }
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    home.unwrap_or_else(|| std::path::PathBuf::from("."))
        .join(".diktate/models/ggml-small-q5_1.bin")
}
