//! Transkrypcja przez model Parakeet TDT, w procesie daemona
//! (`docs/12-backendy-transkrypcji.md`).
//!
//! Parakeet i whisper mieszkają w tym samym drzewie whisper.cpp i dzielą
//! ggml, ale to dwa różne modele: whisper zawsze przepuszcza okno 30 s,
//! parakeet stałego okna nie ma. Stąd cała jego przewaga w dyktowaniu na
//! żywo i stąd też jedyne ograniczenie zakodowane w tym module:
//!
//! **Minimum 3 s audio.** Model sam wykrywa język i przy dwóch sekundach
//! bierze polski za czeski. Wymusić języka nie można — `parakeet_full_params`
//! nie ma takiego pola. Krótkiego segmentu nie porzucamy: dopełniamy go ciszą
//! i wysyłamy. Wynik bywa wtedy bzdurny, ale to widać w logu (`dopełniony`),
//! a nie znika bez śladu. Zjedzone „tak" byłoby gorsze od „tak"
//! przetłumaczonego na czeski, bo tego pierwszego nikt nie zauważy.
//!
//! Liczy na CPU, i to nie z braku alternatywy: enkoder parakeeta na iGPU
//! Meteor Lake jest 2,5× wolniejszy niż na rdzeniach, odwrotnie niż
//! u whispera. Dlatego `use_gpu` jest tu ustawione na `false` niezależnie od
//! tego, czy whisper w tym samym procesie korzysta z Vulkana.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{anyhow, bail, Result};

use diktate_whispercpp_sys as sys;

use crate::transcribe::{cstring, join_segments, models_dir, path_str, read_cstr, Transcribe};
use crate::SAMPLE_RATE;

/// Nazwa pliku modelu w `~/.diktate/models`.
const MODEL_FILE: &str = "ggml-parakeet-tdt-0.6b-v3-q8_0.bin";

/// Uchwyt kontekstu parakeeta. Jak przy whisperze: `parakeet_full` nie jest
/// bezpieczne dla wielu wątków na jednym kontekście, więc dostęp szereguje
/// `Mutex`.
struct ParakeetCtx(*mut sys::DkParakeet);

// SAFETY: wskaźnik jest własnością tej struktury i nigdy nie wycieka; cały
// dostęp do niego jest szeregowany `Mutex`-em w `ParakeetTranscriber`.
unsafe impl Send for ParakeetCtx {}

impl Drop for ParakeetCtx {
    fn drop(&mut self) {
        unsafe { sys::dk_parakeet_close(self.0) };
    }
}

pub struct ParakeetTranscriber {
    ctx: Mutex<ParakeetCtx>,
    /// Minimalna liczba próbek, jaką model przyjmuje bez mylenia języka.
    min_samples: usize,
    threads: u32,
    model: PathBuf,
}

impl ParakeetTranscriber {
    /// Ładuje model Parakeeta. `threads` równe 0 znaczy „tyle, ile rdzeni".
    pub fn open(model_path: impl AsRef<Path>, threads: u32) -> Result<Self> {
        let path = model_path.as_ref();
        if !path.exists() {
            return Err(anyhow!("nie znaleziono modelu parakeeta: {}", path.display()));
        }
        let c_path = cstring(path_str(path)?)?;
        // `false` to nie przeoczenie — patrz komentarz na górze modułu.
        let ctx = unsafe { sys::dk_parakeet_open(c_path.as_ptr(), false) };
        if ctx.is_null() {
            bail!("nie udało się wczytać modelu parakeeta {}", path.display());
        }
        let min_audio_s = unsafe { sys::dk_parakeet_min_audio_s() };
        Ok(Self {
            ctx: Mutex::new(ParakeetCtx(ctx)),
            min_samples: (min_audio_s * SAMPLE_RATE as f32).ceil() as usize,
            threads: resolve_threads(threads),
            model: path.to_path_buf(),
        })
    }

    pub fn model(&self) -> &Path {
        &self.model
    }

    pub fn threads(&self) -> u32 {
        self.threads
    }

    /// Minimalna długość materiału w sekundach; krótszy jest dopełniany ciszą.
    pub fn min_audio_s(&self) -> f32 {
        self.min_samples as f32 / SAMPLE_RATE as f32
    }
}

impl Transcribe for ParakeetTranscriber {
    fn transcribe(&self, samples: &[f32]) -> Result<String> {
        if samples.is_empty() {
            return Ok(String::new());
        }
        let padded = pad_to_min(samples, self.min_samples);
        if padded.len() > samples.len() {
            eprintln!(
                "parakeet: segment {:.1} s dopełniony ciszą do {:.1} s (minimum modelu) — \
                 wynik może być bzdurny, model sam wykrywa język",
                seconds(samples.len()),
                seconds(padded.len())
            );
        }

        let guard = self
            .ctx
            .lock()
            .map_err(|_| anyhow!("kontekst parakeeta został zatruty przez panikę"))?;
        let code = unsafe {
            sys::dk_parakeet_transcribe(
                guard.0,
                padded.as_ptr(),
                padded.len() as i32,
                self.threads as i32,
            )
        };
        if code != 0 {
            let message = unsafe { read_cstr(sys::dk_parakeet_error(guard.0)) };
            bail!("transkrypcja parakeetem nie powiodła się: {message}");
        }
        let text = unsafe { read_cstr(sys::dk_parakeet_text(guard.0)) };
        Ok(join_segments(&text))
    }
}

/// Domyślna ścieżka modelu parakeeta: `$DIKTATE_PARAKEET_MODEL`, a bez niej
/// `~/.diktate/models/ggml-parakeet-tdt-0.6b-v3-q8_0.bin`. Pole
/// `parakeet.model` z `daemon.toml` rozstrzyga wcześniej i nie przechodzi tędy.
pub fn default_parakeet_model_path() -> PathBuf {
    if let Some(path) = std::env::var_os("DIKTATE_PARAKEET_MODEL") {
        return PathBuf::from(path);
    }
    models_dir().join(MODEL_FILE)
}

/// Liczba wątków: 0 znaczy „tyle, ile widzi system".
///
/// Zmierzone na Core Ultra 7 165U (chunk 5 s, pełny czas ścienny): 4 wątki
/// 1640 ms, 8 — 1065 ms, 10 — 1115 ms, 14 — 1105 ms. Powyżej ośmiu jest
/// płasko, więc branie wszystkich wątków nic nie psuje i nie ma tu czego
/// stroić — `parakeet.threads` służy raczej zejściu w dół, gdy ktoś chce
/// zostawić rdzenie czemuś innemu.
fn resolve_threads(configured: u32) -> u32 {
    if configured > 0 {
        return configured;
    }
    std::thread::available_parallelism()
        .map(|n| n.get() as u32)
        .unwrap_or(4)
}

fn seconds(samples: usize) -> f32 {
    samples as f32 / SAMPLE_RATE as f32
}

/// Dopełnia ciszą do `min_samples`. Segment dostatecznie długi wraca bez
/// kopiowania.
fn pad_to_min(samples: &[f32], min_samples: usize) -> std::borrow::Cow<'_, [f32]> {
    if samples.len() >= min_samples {
        return std::borrow::Cow::Borrowed(samples);
    }
    let mut padded = Vec::with_capacity(min_samples);
    padded.extend_from_slice(samples);
    padded.resize(min_samples, 0.0);
    std::borrow::Cow::Owned(padded)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pad_to_min_dopelnia_cisza_do_progu() {
        let padded = pad_to_min(&[1.0, 2.0], 5);
        assert_eq!(&*padded, &[1.0, 2.0, 0.0, 0.0, 0.0]);
    }

    #[test]
    fn pad_to_min_nie_rusza_dostatecznie_dlugiego_segmentu() {
        let samples = vec![0.5f32; 10];
        let padded = pad_to_min(&samples, 5);
        assert_eq!(&*padded, &samples[..]);
        assert!(
            matches!(padded, std::borrow::Cow::Borrowed(_)),
            "dłuższy segment nie ma powodu być kopiowany"
        );
    }

    #[test]
    fn seconds_liczy_z_czestotliwosci_potoku() {
        assert_eq!(seconds(SAMPLE_RATE as usize * 3), 3.0);
    }

    #[test]
    fn resolve_threads_zachowuje_wartosc_z_konfiguracji() {
        assert_eq!(resolve_threads(10), 10);
    }

    #[test]
    fn resolve_threads_zeru_przypisuje_liczbe_watkow_systemu() {
        assert!(resolve_threads(0) >= 1);
    }

    #[test]
    fn minimum_modelu_to_trzy_sekundy() {
        // Wartość pochodzi z warstwy pośredniej, nie z tego dokumentu —
        // gdyby whisper.cpp ją zmienił, test to pokaże.
        assert_eq!(unsafe { sys::dk_parakeet_min_audio_s() }, 3.0);
    }

    #[test]
    fn domyslna_sciezka_modelu_honoruje_zmienna_srodowiskowa() {
        // Zmienna jest procesowa, więc test biegnie w osobnym procesie
        // tylko wtedy, gdy jest jedynym, który ją rusza — stąd odczyt
        // bez ustawiania.
        let path = default_parakeet_model_path();
        assert!(path.ends_with(MODEL_FILE) || std::env::var_os("DIKTATE_PARAKEET_MODEL").is_some());
    }
}
