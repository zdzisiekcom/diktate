//! Podkomendy `diktate model list` i `diktate model pull`.
//!
//! Ten sam pobieracz obsługuje kreator — różnica jest wyłącznie w tym, kto
//! wybiera nazwę: człowiek w dialogu czy argument wiersza poleceń.

use std::io::Write;
use std::path::PathBuf;

use anyhow::{bail, Result};

use crate::model::catalog::{self, Model, MODELS};
use crate::model::download::{download, Http};
use crate::model::store;

/// Rozmiar w jednostkach, w jakich podaje go HuggingFace (MB = 2^20).
pub fn format_size(bytes: u64) -> String {
    if bytes >= 1024 * 1024 {
        format!("{} MB", bytes / (1024 * 1024))
    } else {
        format!("{} kB", bytes / 1024)
    }
}

pub fn list() -> Result<()> {
    println!("Katalog modeli: {}", store::models_dir().display());
    println!();
    for model in MODELS {
        let stan = if model.installed().is_some() {
            "masz"
        } else if model.recommended {
            "zalecany"
        } else {
            ""
        };
        println!(
            "  {:<22} {:>7}  {:>5.1} s/20 s  polszczyzna: {:<18} {}",
            model.name,
            format_size(model.bytes),
            model.run_ms as f32 / 1000.0,
            model.polish.opis(),
            stan
        );
    }
    println!();
    println!("Pobranie: diktate model pull <nazwa>");
    Ok(())
}

pub fn pull(name: &str) -> Result<PathBuf> {
    let Some(model) = catalog::find(name) else {
        let nazwy: Vec<&str> = MODELS.iter().map(|m| m.name).collect();
        bail!(
            "nie znam modelu „{name}”. Dostępne: {}",
            nazwy.join(", ")
        );
    };
    pull_model(model)
}

/// Pobiera model, jeśli nie ma go jeszcze na dysku. Zwraca ścieżkę pliku.
pub fn pull_model(model: &Model) -> Result<PathBuf> {
    if let Some(path) = model.installed() {
        println!("{} jest już na dysku: {}", model.name, path.display());
        return Ok(path);
    }
    println!(
        "Pobieram {} ({}) do {}…",
        model.name,
        format_size(model.bytes),
        store::models_dir().display()
    );
    let mut ostatni = 0u64;
    let path = download(&Http, model, &store::models_dir(), &mut |done, total| {
        // Pasek przerysowywany co procent — częściej to tylko miganie
        // i zaśmiecony terminal, gdy ktoś przekierowuje wyjście do pliku.
        let procent = done.checked_mul(100).and_then(|x| x.checked_div(total)).unwrap_or(0);
        if procent != ostatni {
            ostatni = procent;
            let pełne = (procent / 2) as usize;
            print!(
                "\r  [{}{}] {procent:>3}%",
                "#".repeat(pełne),
                " ".repeat(50 - pełne)
            );
            let _ = std::io::stdout().flush();
        }
    })?;
    println!("\r  gotowe: {}{}", path.display(), " ".repeat(20));
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rozmiar_po_ludzku() {
        assert_eq!(format_size(190_000_000), "181 MB");
        assert_eq!(format_size(1_500), "1 kB");
    }

    #[test]
    fn pull_nieznanej_nazwy_wypisuje_dostępne() {
        let err = pull("nie-ma-takiego").unwrap_err().to_string();
        assert!(err.contains("nie-ma-takiego"));
        assert!(err.contains("small-q5_1"), "komunikat nie podpowiada nazw: {err}");
    }
}
