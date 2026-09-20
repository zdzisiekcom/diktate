//! Wiązania do whisper.cpp — whisper i Parakeet TDT w jednym procesie.
//!
//! Crate jest celowo cienki: cała wiedza o układzie struktur whisper.cpp
//! siedzi w `src/shim.cpp` (patrz komentarz w `src/shim.h`), a tutaj są
//! tylko deklaracje funkcji operujących na wskaźnikach i liczbach. Dzięki
//! temu aktualizacja whisper.cpp nie może po cichu przesunąć pól struktury
//! pod nogami Rusta — najwyżej nie skompiluje się warstwa w C++.
//!
//! Źródła whisper.cpp bierze `build.rs`: submoduł `vendor/whisper.cpp`
//! przypięty do wydania, a gdy ustawiono `WHISPER_CPP_DIR` — wskazany klon.

use std::os::raw::{c_char, c_float, c_int};

/// Nieprzezroczysty uchwyt kontekstu whispera z `shim.h`.
#[repr(C)]
pub struct DkWhisper {
    _private: [u8; 0],
}

/// Nieprzezroczysty uchwyt kontekstu parakeeta z `shim.h`.
#[repr(C)]
pub struct DkParakeet {
    _private: [u8; 0],
}

extern "C" {
    pub fn dk_whisper_open(model_path: *const c_char, use_gpu: bool) -> *mut DkWhisper;
    pub fn dk_whisper_close(ctx: *mut DkWhisper);
    pub fn dk_whisper_transcribe(
        ctx: *mut DkWhisper,
        pcm: *const c_float,
        n_samples: c_int,
        language: *const c_char,
        initial_prompt: *const c_char,
        beam_size: c_int,
        threads: c_int,
    ) -> c_int;
    pub fn dk_whisper_text(ctx: *const DkWhisper) -> *const c_char;
    pub fn dk_whisper_error(ctx: *const DkWhisper) -> *const c_char;

    pub fn dk_parakeet_open(model_path: *const c_char, use_gpu: bool) -> *mut DkParakeet;
    pub fn dk_parakeet_close(ctx: *mut DkParakeet);
    pub fn dk_parakeet_transcribe(
        ctx: *mut DkParakeet,
        pcm: *const c_float,
        n_samples: c_int,
        threads: c_int,
    ) -> c_int;
    pub fn dk_parakeet_text(ctx: *const DkParakeet) -> *const c_char;
    pub fn dk_parakeet_error(ctx: *const DkParakeet) -> *const c_char;

    pub fn dk_parakeet_min_audio_s() -> c_float;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test istnieje po to, żeby cokolwiek w tym crate zostało **zlinkowane**:
    /// sam `rlib` linkowania nie wymusza, więc błąd w kolejności bibliotek
    /// albo brakujący symbol wyszedłby dopiero przy budowaniu daemona.
    #[test]
    fn biblioteka_sie_linkuje_i_odpowiada() {
        let minimum = unsafe { dk_parakeet_min_audio_s() };
        assert_eq!(minimum, 3.0);
    }

    /// Otwarcie nieistniejącego modelu ma zwrócić pusty wskaźnik, a nie
    /// wywrócić proces — daemon opiera na tym swój komunikat o brakującym
    /// modelu.
    #[test]
    fn brak_pliku_modelu_daje_pusty_wskaznik() {
        let path = std::ffi::CString::new("/nie/ma/takiego/modelu.bin").unwrap();
        let ctx = unsafe { dk_parakeet_open(path.as_ptr(), false) };
        assert!(ctx.is_null());
    }
}
