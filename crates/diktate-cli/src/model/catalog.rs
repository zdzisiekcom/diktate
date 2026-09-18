//! Modele, które mają sens na tym sprzęcie — lista wkompilowana, nie z sieci.
//!
//! Odpytywanie API HuggingFace przy samym wyświetleniu listy psułoby kreator
//! bez internetu i pokazywałoby warianty, których nikt tutaj nie chce.
//! Czasy przebiegu i oceny polszczyzny pochodzą z pomiarów opisanych
//! w `CLAUDE.md` („Wydajność transkrypcji"); dodanie modelu to jedna linijka.
#![allow(dead_code)]

use std::path::PathBuf;

use crate::model::store;

/// Jak model radzi sobie z polszczyzną — jedyna ocena, która przy wyborze
/// naprawdę waży, bo czas widać w tabeli obok.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Polish {
    Słaba,
    Dobra,
    Najlepsza,
}

impl Polish {
    pub fn opis(self) -> &'static str {
        match self {
            Polish::Słaba => "słaba — myli słowa",
            Polish::Dobra => "dobra",
            Polish::Najlepsza => "najlepsza",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Model {
    /// Nazwa w komendach i w dialogu.
    pub name: &'static str,
    /// Nazwa pliku — ta sama na dysku i w repozytorium HuggingFace.
    pub file: &'static str,
    pub bytes: u64,
    pub sha256: &'static str,
    /// Zmierzony czas przebiegu na 20 s polskiej mowy (Vulkan, ten sprzęt).
    pub run_ms: u32,
    pub polish: Polish,
    pub recommended: bool,
}

impl Model {
    pub fn url(&self) -> String {
        format!("https://huggingface.co/ggerganov/whisper.cpp/resolve/main/{}", self.file)
    }

    /// Ścieżka, jeśli model już leży na dysku.
    pub fn installed(&self) -> Option<PathBuf> {
        store::find(self.file)
    }
}

pub const MODELS: &[Model] = &[
    Model {
        name: "base-q5_1",
        file: "ggml-base-q5_1.bin",
        bytes: 59_707_625,
        sha256: "422f1ae452ade6f30a004d7e5c6a43195e4433bc370bf23fac9cc591f01a8898",
        run_ms: 2_100,
        polish: Polish::Słaba,
        recommended: false,
    },
    Model {
        name: "small-q5_1",
        file: "ggml-small-q5_1.bin",
        bytes: 190_085_487,
        sha256: "ae85e4a935d7a567bd102fe55afc16bb595bdb618e11b2fc7591bc08120411bb",
        run_ms: 4_500,
        polish: Polish::Dobra,
        recommended: true,
    },
    Model {
        name: "small",
        file: "ggml-small.bin",
        bytes: 487_601_967,
        sha256: "1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b",
        run_ms: 4_900,
        polish: Polish::Dobra,
        recommended: false,
    },
    Model {
        name: "large-v3-turbo-q5_0",
        file: "ggml-large-v3-turbo-q5_0.bin",
        bytes: 574_041_195,
        sha256: "394221709cd5ad1f40c46e6031ca61bce88931e6e088c188294c6d5a55ffa7e2",
        run_ms: 15_500,
        polish: Polish::Najlepsza,
        recommended: false,
    },
];

pub fn find(name: &str) -> Option<&'static Model> {
    MODELS.iter().find(|m| m.name == name)
}

/// Model proponowany domyślnie. Katalog gwarantuje dokładnie jeden taki wpis
/// (pilnuje tego test), więc brak wyniku byłby błędem w samym katalogu.
pub fn recommended() -> &'static Model {
    MODELS
        .iter()
        .find(|m| m.recommended)
        .expect("katalog modeli bez rekomendacji — sprzeczne z testem spójności")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nazwy_są_unikalne() {
        let mut nazwy: Vec<&str> = MODELS.iter().map(|m| m.name).collect();
        nazwy.sort_unstable();
        let przed = nazwy.len();
        nazwy.dedup();
        assert_eq!(nazwy.len(), przed, "powtórzona nazwa modelu w katalogu");
    }

    #[test]
    fn dokładnie_jeden_model_jest_rekomendowany() {
        // Dialog pokazuje rekomendację jako domyślną odpowiedź pod Enterem.
        // Dwie rekomendacje albo zero oznaczają, że nie ma czego zaproponować.
        let ile = MODELS.iter().filter(|m| m.recommended).count();
        assert_eq!(ile, 1);
    }

    #[test]
    fn sumy_kontrolne_mają_długość_sha256() {
        for model in MODELS {
            assert_eq!(model.sha256.len(), 64, "zła suma dla {}", model.name);
            assert!(
                model.sha256.chars().all(|c| c.is_ascii_hexdigit()),
                "suma {} nie jest szesnastkowa",
                model.name
            );
        }
    }

    #[test]
    fn url_wskazuje_na_repozytorium_whisper_cpp() {
        let model = recommended();
        assert_eq!(
            model.url(),
            format!(
                "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/{}",
                model.file
            )
        );
    }

    #[test]
    fn szukanie_po_nazwie() {
        assert!(find("small-q5_1").is_some());
        assert!(find("nie-ma-takiego").is_none());
    }
}
