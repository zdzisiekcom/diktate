//! Pobieranie wag z HuggingFace — z wznawianiem i sumą kontrolną.
//!
//! Plik rośnie jako `<nazwa>.part` i dostaje nazwę docelową dopiero po
//! zgodnej sumie SHA-256. Zerwane łącze zostawia więc ślad, z którego da się
//! dociągnąć resztę, ale nigdy pliku, który wygląda na gotowy model.
//!
//! Sieć siedzi za `Source`, żeby testy pobierania biegły na danych
//! z pamięci — bez internetu i bez półgigowych plików.
#![allow(dead_code)]

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};

use crate::model::catalog::Model;

/// Skąd biorą się bajty. `from` to numer pierwszego potrzebnego bajtu —
/// źródło ma pominąć wcześniejsze (nagłówek `Range`).
pub trait Source {
    fn fetch(&self, url: &str, from: u64) -> Result<Box<dyn Read>>;
}

/// Prawdziwe pobieranie po HTTPS.
pub struct Http;

impl Source for Http {
    fn fetch(&self, url: &str, from: u64) -> Result<Box<dyn Read>> {
        let mut request = ureq::get(url);
        if from > 0 {
            request = request.set("Range", &format!("bytes={from}-"));
        }
        let response = request
            .call()
            .with_context(|| format!("nie udało się pobrać {url}"))?;
        if from > 0 && response.status() != 206 {
            bail!(
                "serwer nie obsłużył wznowienia (odpowiedź {}), usuń plik .part i pobierz od nowa",
                response.status()
            );
        }
        Ok(Box::new(response.into_reader()))
    }
}

const BUFFER: usize = 64 * 1024;

/// Pobiera model do `dir`. `progress` dostaje (ile bajtów gotowych, ile razem)
/// — pierwsze wywołanie mówi, od którego bajtu ruszyliśmy.
pub fn download(
    source: &dyn Source,
    model: &Model,
    dir: &Path,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<PathBuf> {
    std::fs::create_dir_all(dir)
        .with_context(|| format!("nie udało się utworzyć {}", dir.display()))?;
    let target = dir.join(model.file);
    let part = dir.join(format!("{}.part", model.file));

    let done = part.metadata().map(|m| m.len()).unwrap_or(0);
    progress(done, model.bytes);

    let mut reader = source.fetch(&model.url(), done)?;
    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(&part)
        .with_context(|| format!("nie udało się otworzyć {}", part.display()))?;
    file.seek(SeekFrom::Start(done))?;

    let mut done = done;
    let mut buffer = vec![0u8; BUFFER];
    loop {
        let read = reader.read(&mut buffer).context("zerwane pobieranie")?;
        if read == 0 {
            break;
        }
        file.write_all(&buffer[..read])?;
        done += read as u64;
        progress(done, model.bytes);
    }
    file.flush()?;
    drop(file);

    let suma = sha256_file(&part)?;
    if suma != model.sha256 {
        // Plik zostaje jako `.part`, nie jako model: skoro nie wiadomo, co to
        // jest, nie może udawać wag.
        bail!(
            "suma kontrolna pobranego pliku nie zgadza się z katalogiem \
             (oczekiwano {}, jest {}) — plik został jako {}",
            model.sha256,
            suma,
            part.display()
        );
    }

    std::fs::rename(&part, &target)
        .with_context(|| format!("nie udało się nazwać {}", target.display()))?;
    Ok(target)
}

fn sha256_file(path: &Path) -> Result<String> {
    let mut file = File::open(path)
        .with_context(|| format!("nie udało się odczytać {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; BUFFER];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex(&hasher.finalize()))
}

/// Suma z danych w pamięci — używana przez testy katalogu i pobierania.
pub fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex(&hasher.finalize())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::catalog::Polish;
    use std::io::Cursor;

    /// Źródło z pamięci — testy nie dotykają sieci.
    struct Fake {
        data: Vec<u8>,
    }

    impl Source for Fake {
        fn fetch(&self, _url: &str, from: u64) -> Result<Box<dyn Read>> {
            Ok(Box::new(Cursor::new(self.data[from as usize..].to_vec())))
        }
    }

    fn temp_dir(nazwa: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join(format!("diktate-dl-{}-{nazwa}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn model(data: &[u8]) -> Model {
        Model {
            name: "test",
            file: "ggml-test.bin",
            bytes: data.len() as u64,
            sha256: Box::leak(sha256_hex(data).into_boxed_str()),
            run_ms: 0,
            polish: Polish::Dobra,
            recommended: false,
        }
    }

    #[test]
    fn pobiera_i_weryfikuje_sumę() {
        let dane = b"model udajacy wagi".to_vec();
        let dir = temp_dir("pelne");
        let m = model(&dane);

        let path = download(&Fake { data: dane.clone() }, &m, &dir, &mut |_, _| {}).unwrap();

        assert_eq!(std::fs::read(&path).unwrap(), dane);
        assert_eq!(path, dir.join("ggml-test.bin"));
    }

    #[test]
    fn wznawia_z_pliku_częściowego() {
        // Zerwane łącze zostawia `.part`; ponowne pobranie ma dociągnąć
        // resztę, a nie zaczynać pół giga od nowa.
        let dane = b"model udajacy wagi".to_vec();
        let dir = temp_dir("wznowienie");
        std::fs::write(dir.join("ggml-test.bin.part"), &dane[..5]).unwrap();
        let m = model(&dane);

        let mut pobrano_od = u64::MAX;
        let path = download(&Fake { data: dane.clone() }, &m, &dir, &mut |done, _| {
            pobrano_od = pobrano_od.min(done);
        })
        .unwrap();

        assert_eq!(std::fs::read(&path).unwrap(), dane);
        assert_eq!(pobrano_od, 5, "pobieranie ruszyło od początku, nie od 5 bajtu");
    }

    #[test]
    fn niezgodna_suma_nie_zostawia_pliku_docelowego() {
        // Półpobrany albo podmieniony ggml-*.bin jest gorszy niż jego brak:
        // whisper przewraca się na nim komunikatem, który niczego nie tłumaczy.
        let dir = temp_dir("zla-suma");
        let mut m = model(b"prawdziwe dane");
        m.sha256 = "0000000000000000000000000000000000000000000000000000000000000000";

        let err = download(
            &Fake { data: b"prawdziwe dane".to_vec() },
            &m,
            &dir,
            &mut |_, _| {},
        )
        .unwrap_err();

        assert!(err.to_string().contains("suma"), "komunikat: {err}");
        assert!(!dir.join("ggml-test.bin").exists());
    }
}
