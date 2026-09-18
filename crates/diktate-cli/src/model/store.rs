//! Gdzie leżą modele i który z nich jest już na dysku.
//!
//! Katalog docelowy to `~/.diktate/models`, ale plik pobrany wcześniej
//! ręcznie zwykle leży w `~/models/whisper` — stąd dwa katalogi w kolejności
//! szukania. Diktate niczego stamtąd nie przenosi: cudzy katalog nie jest
//! jego własnością, a pół giga skopiowane bez pytania to nie uprzejmość.

use std::path::PathBuf;

/// `~/.diktate/models` — katalog, do którego pobieramy.
#[allow(dead_code)]
pub fn models_dir() -> PathBuf {
    home().join(".diktate/models")
}

/// `~/models/whisper` — miejsce z czasów przed kreatorem.
#[allow(dead_code)]
pub fn legacy_models_dir() -> PathBuf {
    home().join("models/whisper")
}

#[allow(dead_code)]
fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Ścieżka modelu, jeśli jest na dysku — w katalogu docelowym albo starym.
#[allow(dead_code)]
pub fn find(file: &str) -> Option<PathBuf> {
    find_in(&[models_dir(), legacy_models_dir()], file)
}

/// Jak [`find`], ale dla wskazanych katalogów — rozdzielone, żeby dało się
/// to przetestować bez dotykania `$HOME`.
pub fn find_in(dirs: &[PathBuf], file: &str) -> Option<PathBuf> {
    dirs.iter()
        .map(|dir| dir.join(file))
        .find(|path| path.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Katalog roboczy testu — testy biegną równolegle w jednym procesie,
    /// więc każdy musi mieć własny.
    fn temp_dir(nazwa: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "diktate-test-{}-{nazwa}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn znajduje_model_w_pierwszym_katalogu() {
        let nowy = temp_dir("nowy");
        let stary = temp_dir("stary");
        std::fs::write(nowy.join("ggml-small-q5_1.bin"), b"x").unwrap();

        let znaleziony = find_in(&[nowy.clone(), stary], "ggml-small-q5_1.bin");

        assert_eq!(znaleziony, Some(nowy.join("ggml-small-q5_1.bin")));
    }

    #[test]
    fn sięga_do_starego_katalogu_gdy_w_nowym_nie_ma() {
        // Kto pobrał model wcześniej ręcznie, ma go w ~/models/whisper.
        // Pobieranie tego samego pliku drugi raz byłoby marnowaniem
        // pół giga transferu.
        let nowy = temp_dir("pusty-nowy");
        let stary = temp_dir("pełny-stary");
        std::fs::write(stary.join("ggml-small.bin"), b"x").unwrap();

        let znaleziony = find_in(&[nowy, stary.clone()], "ggml-small.bin");

        assert_eq!(znaleziony, Some(stary.join("ggml-small.bin")));
    }

    #[test]
    fn brak_pliku_to_brak_wyniku() {
        let dir = temp_dir("brak");
        assert_eq!(find_in(&[dir], "ggml-base.bin"), None);
    }
}
