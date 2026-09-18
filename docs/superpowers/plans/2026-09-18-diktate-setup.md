# `diktate setup` — plan wdrożenia

> **Dla wykonawców:** WYMAGANY SUB-SKILL: `superpowers:subagent-driven-development`
> (albo `superpowers:executing-plans`). Kroki mają pola wyboru (`- [ ]`) do odhaczania.

**Cel:** `diktate setup` prowadzi dialogiem przez wybór modelu (z pobraniem
z HuggingFace) i parametry konfiguracji, po czym zapisuje `daemon.toml`
i `gtk.toml`; `diktate model list|pull` robi to samo bez dialogu.

**Architektura:** wszystko w istniejącym crate `diktate-cli`, w dwóch
modułach: `model/` (katalog, skan dysku, pobieranie) i `setup/` (profile,
różnice, zapis, dialog). Kształt konfiguracji pochodzi wyłącznie
z `diktate-config` — kreator nie ma własnej listy pól. Wykrycie działającego
daemona wymaga przeniesienia `pidfile` do `diktate-proto`.

**Stos:** Rust 2021, clap 4, `diktate-config` (serde + toml 0.8),
`ureq` 2 z rustls, `sha2` 0.10.

**Spec:** `docs/superpowers/specs/2026-09-18-diktate-setup-design.md`
oraz `docs/04-konfiguracja.md#kreator-diktate-setup` (nadrzędne).

## Ograniczenia globalne

- Komentarze, komunikaty błędów i teksty dialogu: **po polsku**, z pełną
  ortografią. Nazwy typów i funkcji: po angielsku.
- Komentarz `//!` na górze każdego nowego modułu wyjaśnia zasadę działania,
  nie przepisuje kodu.
- Katalog modeli: `~/.diktate/models/`. Domyślny model:
  `ggml-small-q5_1.bin`.
- Repo HuggingFace: `ggerganov/whisper.cpp`, URL pliku
  `https://huggingface.co/ggerganov/whisper.cpp/resolve/main/<plik>`.
- Kolejność ścieżki modelu: `$DIKTATE_MODEL` → `whisper.model`
  z `daemon.toml` → domyślna.
- Zapis konfiguracji **atomowy**: plik tymczasowy w tym samym katalogu,
  potem `rename` (`docs/04`, „Trzy pliki, trzy właściciele").
- Testy muszą działać bez mikrofonu, bez modelu, bez sieci i bez GTK.
- `ureq` bierzemy z rustls (`default-features = false`,
  `features = ["tls", "json"]` nie jest potrzebne — wystarczy `tls`), bo
  OpenSSL w tym środowisku nie ma nagłówków (`CLAUDE.md`, „Pułapki”).
- Po każdym zadaniu: `cargo test --workspace` i
  `cargo clippy --workspace --all-targets` bez ostrzeżeń.
- Każde zadanie kończy się osobnym commitem; wiadomości po polsku,
  z linią `Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>`.

---

### Task 1: `pidfile` przenosi się do `diktate-proto`

Kreator musi wykryć działającego daemona, a `pidfile.rs` leży dziś
w binarce `diktate-daemon`, z której nie da się importować. `diktate-proto`
trzyma już `socket_path()`, więc lokalizacje sesyjne to jego rola.

**Pliki:**
- Utwórz: `crates/diktate-proto/src/pidfile.rs` (przeniesiony w całości,
  razem z testami, z `crates/diktate-daemon/src/pidfile.rs`)
- Zmień: `crates/diktate-proto/src/lib.rs` (dodanie `pub mod pidfile;`)
- Zmień: `crates/diktate-proto/Cargo.toml` (dodanie `anyhow` i `libc`)
- Zmień: `crates/diktate-daemon/src/main.rs:11` (`mod pidfile;` →
  `use diktate_proto::pidfile;`)
- Usuń: `crates/diktate-daemon/src/pidfile.rs`

**Interfejsy:**
- Produkuje: `diktate_proto::pidfile::{pid_path, process_alive, running,
  running_at, write_current, terminate, remove}` — sygnatury bez zmian.

- [ ] **Krok 1: przenieś plik i podłącz moduł**

```bash
git mv crates/diktate-daemon/src/pidfile.rs crates/diktate-proto/src/pidfile.rs
```

W `crates/diktate-proto/src/lib.rs`, zaraz pod komentarzem `//!`:

```rust
pub mod pidfile;
```

W `crates/diktate-proto/Cargo.toml`, w `[dependencies]`:

```toml
anyhow.workspace = true
libc.workspace = true
```

- [ ] **Krok 2: przestaw daemona na nowe miejsce**

W `crates/diktate-daemon/src/main.rs` usuń linię `mod pidfile;` i dopisz
obok pozostałych importów:

```rust
use diktate_proto::pidfile;
```

Reszta wywołań (`pidfile::running()` itd.) zostaje bez zmian.

- [ ] **Krok 3: uruchom testy**

Run: `cargo test --workspace`
Oczekiwane: PASS, w tym testy pidfile'a liczone teraz w `diktate-proto`.

- [ ] **Krok 4: commit**

```bash
git add -A
git commit -m "refactor(proto): pidfile przenosi się z daemona do proto"
```

---

### Task 2: katalog `~/.diktate/models` i domyślna ścieżka modelu

**Pliki:**
- Zmień: `crates/diktate-core/src/transcribe.rs:118-125` (`default_model_path`)
- Utwórz: `crates/diktate-cli/src/model/mod.rs`
- Utwórz: `crates/diktate-cli/src/model/store.rs`
- Zmień: `crates/diktate-cli/src/main.rs` (dodanie `mod model;`)

**Interfejsy:**
- Produkuje:
  - `diktate_core::transcribe::default_model_path() -> PathBuf`
    (bez zmiany sygnatury, zmienia się wynik)
  - `model::store::models_dir() -> PathBuf`
  - `model::store::legacy_models_dir() -> PathBuf`
  - `model::store::find(file: &str) -> Option<PathBuf>` — szuka pliku
    najpierw w `models_dir()`, potem w `legacy_models_dir()`
  - `model::store::find_in(dirs: &[PathBuf], file: &str) -> Option<PathBuf>`
    — wersja testowalna, bez dotykania `$HOME`

- [ ] **Krok 1: napisz test skanu dysku**

W `crates/diktate-cli/src/model/store.rs`, na dole pliku:

```rust
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
```

- [ ] **Krok 2: uruchom test i sprawdź, że nie kompiluje się / nie przechodzi**

Run: `cargo test -p diktate-cli store::`
Oczekiwane: FAIL — brak funkcji `find_in`.

- [ ] **Krok 3: napisz `store.rs`**

```rust
//! Gdzie leżą modele i który z nich jest już na dysku.
//!
//! Katalog docelowy to `~/.diktate/models`, ale plik pobrany wcześniej
//! ręcznie zwykle leży w `~/models/whisper` — stąd dwa katalogi w kolejności
//! szukania. Diktate niczego stamtąd nie przenosi: cudzy katalog nie jest
//! jego własnością, a pół giga skopiowane bez pytania to nie uprzejmość.

use std::path::PathBuf;

/// `~/.diktate/models` — katalog, do którego pobieramy.
pub fn models_dir() -> PathBuf {
    home().join(".diktate/models")
}

/// `~/models/whisper` — miejsce z czasów przed kreatorem.
pub fn legacy_models_dir() -> PathBuf {
    home().join("models/whisper")
}

fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Ścieżka modelu, jeśli jest na dysku — w katalogu docelowym albo starym.
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
```

`crates/diktate-cli/src/model/mod.rs`:

```rust
//! Modele whispera: co jest do wyboru, co leży na dysku i jak pobrać resztę.

pub mod store;
```

W `crates/diktate-cli/src/main.rs`, pod komentarzem `//!`:

```rust
mod model;
```

- [ ] **Krok 4: uruchom testy**

Run: `cargo test -p diktate-cli store::`
Oczekiwane: PASS (3 testy).

- [ ] **Krok 5: zmień domyślną ścieżkę modelu w rdzeniu**

`crates/diktate-core/src/transcribe.rs`, funkcja `default_model_path`:

```rust
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
```

- [ ] **Krok 6: cały workspace**

Run: `cargo test --workspace && cargo clippy --workspace --all-targets`
Oczekiwane: PASS, zero ostrzeżeń.

- [ ] **Krok 7: commit**

```bash
git add -A
git commit -m "feat(cli): katalog ~/.diktate/models i skan już pobranych modeli"
```

---

### Task 3: kuratorowany katalog modeli

**Pliki:**
- Utwórz: `crates/diktate-cli/src/model/catalog.rs`
- Zmień: `crates/diktate-cli/src/model/mod.rs` (`pub mod catalog;`)

**Interfejsy:**
- Konsumuje: `model::store::find`
- Produkuje:
  - `struct Model { name: &'static str, file: &'static str, bytes: u64,
    sha256: &'static str, run_ms: u32, polish: Polish, recommended: bool }`
  - `enum Polish { Słaba, Dobra, Najlepsza }` z metodą
    `fn opis(self) -> &'static str`
  - `const MODELS: &[Model]`
  - `fn find(name: &str) -> Option<&'static Model>`
  - `fn recommended() -> &'static Model`
  - `impl Model { fn url(&self) -> String; fn installed(&self) ->
    Option<PathBuf> }`

- [ ] **Krok 1: napisz testy spójności katalogu**

```rust
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
```

- [ ] **Krok 2: uruchom i sprawdź, że nie przechodzi**

Run: `cargo test -p diktate-cli catalog::`
Oczekiwane: FAIL — brak modułu.

- [ ] **Krok 3: napisz `catalog.rs`**

```rust
//! Modele, które mają sens na tym sprzęcie — lista wkompilowana, nie z sieci.
//!
//! Odpytywanie API HuggingFace przy samym wyświetleniu listy psułoby kreator
//! bez internetu i pokazywałoby warianty, których nikt tutaj nie chce.
//! Czasy przebiegu i oceny polszczyzny pochodzą z pomiarów opisanych
//! w `CLAUDE.md` („Wydajność transkrypcji”); dodanie modelu to jedna linijka.

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
        bytes: 59_700_000,
        sha256: "<uzupełnić w kroku 4>",
        run_ms: 2_100,
        polish: Polish::Słaba,
        recommended: false,
    },
    Model {
        name: "small-q5_1",
        file: "ggml-small-q5_1.bin",
        bytes: 190_000_000,
        sha256: "<uzupełnić w kroku 4>",
        run_ms: 4_500,
        polish: Polish::Dobra,
        recommended: true,
    },
    Model {
        name: "small",
        file: "ggml-small.bin",
        bytes: 487_000_000,
        sha256: "<uzupełnić w kroku 4>",
        run_ms: 4_900,
        polish: Polish::Dobra,
        recommended: false,
    },
    Model {
        name: "large-v3-turbo-q5_0",
        file: "ggml-large-v3-turbo-q5_0.bin",
        bytes: 574_000_000,
        sha256: "<uzupełnić w kroku 4>",
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
```

- [ ] **Krok 4: uzupełnij prawdziwe sumy i rozmiary**

HuggingFace podaje sumę SHA-256 każdego pliku w nagłówku `X-Linked-Etag`
albo w `ETag` (dla plików LFS to `sha256:<suma>`), a rozmiar w
`X-Linked-Size`. Dla każdego modelu z listy:

```bash
curl -sIL https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small-q5_1.bin \
  | grep -i -E 'x-linked-etag|x-linked-size|content-length'
```

Jeśli model jest już na dysku, suma z pliku jest równie dobra:

```bash
sha256sum ~/.diktate/models/ggml-small-q5_1.bin
stat -c %s ~/.diktate/models/ggml-small-q5_1.bin
```

Wstaw wartości w miejsce `<uzupełnić w kroku 4>` i popraw `bytes` na
dokładne. Gdyby któregoś pliku nie dało się sprawdzić (brak sieci), usuń ten
wpis z katalogu — model bez sumy kontrolnej jest gorszy niż jego brak, bo
pobieranie nie ma czego weryfikować.

- [ ] **Krok 5: uruchom testy**

Run: `cargo test -p diktate-cli catalog::`
Oczekiwane: PASS (5 testów).

- [ ] **Krok 6: commit**

```bash
git add -A
git commit -m "feat(cli): kuratorowany katalog modeli whispera"
```

---

### Task 4: pobieranie z weryfikacją sumy

**Pliki:**
- Utwórz: `crates/diktate-cli/src/model/download.rs`
- Zmień: `crates/diktate-cli/src/model/mod.rs` (`pub mod download;`)
- Zmień: `crates/diktate-cli/Cargo.toml` (`ureq`, `sha2`)

**Interfejsy:**
- Konsumuje: `model::catalog::Model`, `model::store::models_dir`
- Produkuje:
  - `trait Source { fn fetch(&self, url: &str, from: u64) ->
    Result<Box<dyn Read>>; }` — `from` to bajt początkowy (nagłówek `Range`)
  - `struct Http;` implementujące `Source` przez `ureq`
  - `fn download(source: &dyn Source, model: &Model, dir: &Path,
    progress: &mut dyn FnMut(u64, u64)) -> Result<PathBuf>`

- [ ] **Krok 1: dopisz zależności**

`crates/diktate-cli/Cargo.toml`, w `[dependencies]`:

```toml
# rustls, nie natywne TLS: OpenSSL w tym środowisku nie ma nagłówków
# (patrz CLAUDE.md, „Pułapki tego środowiska”), a pobranie jednego pliku
# nie jest powodem, żeby wracać do tamtego problemu.
ureq = { version = "2", default-features = false, features = ["tls"] }
sha2 = "0.10"
```

- [ ] **Krok 2: napisz testy pobierania**

W `download.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
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
```

- [ ] **Krok 3: uruchom i sprawdź, że nie przechodzi**

Run: `cargo test -p diktate-cli download::`
Oczekiwane: FAIL — brak modułu.

- [ ] **Krok 4: napisz `download.rs`**

```rust
//! Pobieranie wag z HuggingFace — z wznawianiem i sumą kontrolną.
//!
//! Plik rośnie jako `<nazwa>.part` i dostaje nazwę docelową dopiero po
//! zgodnej sumie SHA-256. Zerwane łącze zostawia więc ślad, z którego da się
//! dociągnąć resztę, ale nigdy pliku, który wygląda na gotowy model.
//!
//! Sieć siedzi za `Source`, żeby testy pobierania biegły na danych
//! z pamięci — bez internetu i bez półgigowych plików.

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
```

W testach dopisz brakujące importy (`use crate::model::catalog::Polish;`).

- [ ] **Krok 5: uruchom testy**

Run: `cargo test -p diktate-cli download::`
Oczekiwane: PASS (3 testy).

- [ ] **Krok 6: commit**

```bash
git add -A
git commit -m "feat(cli): pobieranie modeli z wznawianiem i sumą SHA-256"
```

---

### Task 5: podkomendy `diktate model list` i `model pull`

**Pliki:**
- Zmień: `crates/diktate-cli/src/main.rs` (nowe warianty `Command`)
- Utwórz: `crates/diktate-cli/src/model/cmd.rs`
- Zmień: `crates/diktate-cli/src/model/mod.rs` (`pub mod cmd;`)

**Interfejsy:**
- Konsumuje: `catalog::{MODELS, find, Model}`, `download::{download, Http}`,
  `store::models_dir`
- Produkuje:
  - `model::cmd::list() -> Result<()>`
  - `model::cmd::pull(name: &str) -> Result<PathBuf>`
  - `model::cmd::pull_model(model: &Model) -> Result<PathBuf>` — używane
    też przez kreator w zadaniu 7
  - `model::cmd::format_size(bytes: u64) -> String` (np. `181 MB`)

- [ ] **Krok 1: napisz test formatowania rozmiaru**

W `cmd.rs`:

```rust
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
```

- [ ] **Krok 2: uruchom i sprawdź, że nie przechodzi**

Run: `cargo test -p diktate-cli cmd::`
Oczekiwane: FAIL — brak modułu.

- [ ] **Krok 3: napisz `cmd.rs`**

```rust
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
        let procent = if total > 0 { done * 100 / total } else { 0 };
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
```

- [ ] **Krok 4: podłącz podkomendy w `main.rs`**

W `enum Command` dopisz:

```rust
    /// Modele whispera: lista i pobieranie.
    Model {
        #[command(subcommand)]
        action: ModelAction,
    },
```

oraz obok:

```rust
#[derive(Subcommand)]
enum ModelAction {
    /// Wypisz znane modele i zaznacz te, które już masz.
    List,
    /// Pobierz model do ~/.diktate/models.
    Pull {
        /// Nazwa z `diktate model list`.
        name: String,
    },
}
```

W `match cli.command` dopisz:

```rust
        Command::Model { action } => match action {
            ModelAction::List => model::cmd::list(),
            ModelAction::Pull { name } => model::cmd::pull(&name).map(|_| ()),
        },
```

- [ ] **Krok 5: uruchom testy i komendę**

Run: `cargo test -p diktate-cli && cargo run -p diktate-cli -- model list`
Oczekiwane: testy PASS; `model list` wypisuje tabelę z zaznaczeniem tego,
co jest na dysku.

- [ ] **Krok 6: commit**

```bash
git add -A
git commit -m "feat(cli): diktate model list i model pull"
```

---

### Task 6: profile rytmu, różnice i zapis konfiguracji

To jądro kreatora bez ani jednego `stdin` — całość da się przetestować.

**Pliki:**
- Utwórz: `crates/diktate-cli/src/setup/mod.rs`
- Utwórz: `crates/diktate-cli/src/setup/profile.rs`
- Utwórz: `crates/diktate-cli/src/setup/diff.rs`
- Utwórz: `crates/diktate-cli/src/setup/write.rs`
- Zmień: `crates/diktate-cli/src/main.rs` (`mod setup;`)
- Zmień: `crates/diktate-cli/Cargo.toml` (`diktate-config`, `toml`)

**Interfejsy:**
- Konsumuje: `diktate_config::{DaemonConfig, GtkConfig, VadConfig}`
- Produkuje:
  - `setup::profile::{Rhythm, RHYTHMS}` gdzie
    `struct Rhythm { name: &'static str, long_segment_ms: u32,
    silence_ms_to_end_long: u32, silence_ms_to_end: u32, opis: &'static str }`
  - `setup::profile::apply(vad: &mut VadConfig, r: &Rhythm)`
  - `setup::profile::detect(vad: &VadConfig) -> Option<&'static Rhythm>`
  - `setup::profile::too_fast_for(r: &Rhythm, run_ms: u32) -> bool`
  - `setup::diff::changes<T: Serialize>(przed: &T, po: &T) -> Vec<Change>`
    gdzie `struct Change { field: String, before: String, after: String }`
  - `setup::write::daemon_toml(cfg: &DaemonConfig) -> String`
  - `setup::write::gtk_toml(cfg: &GtkConfig) -> String`
  - `setup::write::save(path: &Path, text: &str) -> Result<()>` — kopia
    `.bak`, zapis atomowy

- [ ] **Krok 1: dopisz zależności**

`crates/diktate-cli/Cargo.toml`:

```toml
diktate-config.workspace = true
toml = "0.8"
```

- [ ] **Krok 2: napisz testy profili**

`setup/profile.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn każdy_profil_przechodzi_walidację_daemona() {
        // Kreator nie może zaproponować ustawień, których daemon nie przyjmie.
        for rhythm in RHYTHMS {
            let mut cfg = DaemonConfig::default();
            apply(&mut cfg.vad, rhythm);
            let toml = toml::to_string(&cfg).unwrap();
            DaemonConfig::from_toml(&toml)
                .unwrap_or_else(|e| panic!("profil {} odrzucony: {e}", rhythm.name));
        }
    }

    #[test]
    fn rozpoznaje_własny_profil_po_progach() {
        let mut vad = VadConfig::default();
        apply(&mut vad, &RHYTHMS[2]);
        assert_eq!(detect(&vad).map(|r| r.name), Some(RHYTHMS[2].name));
    }

    #[test]
    fn ręcznie_ustawione_progi_to_profil_własny() {
        // Plik napisany ręcznie nie może zostać po cichu podmieniony
        // na najbliższy profil.
        let mut vad = VadConfig::default();
        vad.long_segment_ms = 3_333;
        assert!(detect(&vad).is_none());
    }

    #[test]
    fn szybki_profil_jest_za_szybki_dla_wolnego_modelu() {
        // small-q5_1 liczy segment ~2,8 s; przy progu 2000 ms segment ma
        // ~2,9 s, więc kolejka zaczyna rosnąć (CLAUDE.md, „Wydajność”).
        let szybki = &RHYTHMS[0];
        assert!(too_fast_for(szybki, 4_500));
        assert!(!too_fast_for(szybki, 1_200));
    }
}
```

- [ ] **Krok 3: uruchom i sprawdź, że nie przechodzi**

Run: `cargo test -p diktate-cli profile::`
Oczekiwane: FAIL — brak modułu.

- [ ] **Krok 4: napisz `profile.rs`**

```rust
//! Rytm segmentacji jako jeden wybór zamiast trzech liczb.
//!
//! Progi VAD w oderwaniu od siebie nie znaczą nic: skrócenie pauzy nie
//! przyspiesza niczego, dopóki próg długości segmentu zostaje wysoki
//! (`docs/04`, „Dlaczego progi VAD nazywają się właśnie tak”). Dlatego krótka
//! ścieżka kreatora pyta o profil, a nie o pola.

use diktate_config::{DaemonConfig, VadConfig};

pub struct Rhythm {
    pub name: &'static str,
    pub long_segment_ms: u32,
    pub silence_ms_to_end_long: u32,
    pub silence_ms_to_end: u32,
    pub opis: &'static str,
}

pub const RHYTHMS: &[Rhythm] = &[
    Rhythm {
        name: "szybki",
        long_segment_ms: 2_000,
        silence_ms_to_end_long: 200,
        silence_ms_to_end: 1_200,
        opis: "tekst pojawia się najszybciej; wymaga szybkiego modelu",
    },
    Rhythm {
        name: "zrównoważony",
        long_segment_ms: 2_500,
        silence_ms_to_end_long: 200,
        silence_ms_to_end: 1_500,
        opis: "domyślny — sprawdzony dla small-q5_1",
    },
    Rhythm {
        name: "dokładny",
        long_segment_ms: 4_000,
        silence_ms_to_end_long: 400,
        silence_ms_to_end: 2_000,
        opis: "dłuższe segmenty, więcej kontekstu dla modelu",
    },
];

pub fn apply(vad: &mut VadConfig, rhythm: &Rhythm) {
    vad.long_segment_ms = rhythm.long_segment_ms;
    vad.silence_ms_to_end_long = rhythm.silence_ms_to_end_long;
    vad.silence_ms_to_end = rhythm.silence_ms_to_end;
}

/// Który profil odpowiada tym progom — `None` znaczy „ustawione ręcznie”
/// i jest odpowiedzią pełnoprawną, nie błędem.
pub fn detect(vad: &VadConfig) -> Option<&'static Rhythm> {
    RHYTHMS.iter().find(|r| {
        r.long_segment_ms == vad.long_segment_ms
            && r.silence_ms_to_end_long == vad.silence_ms_to_end_long
            && r.silence_ms_to_end == vad.silence_ms_to_end
    })
}

/// Czy przy modelu o takim czasie przebiegu ten profil przewróci kolejkę.
///
/// Segment ma średnio mniej więcej `long_segment_ms` plus pauza; koszt
/// przebiegu podajemy dla 20 s materiału, więc na segment przypada
/// `run_ms * długość / 20 s`. Kolejka rośnie, gdy koszt przekracza długość.
pub fn too_fast_for(rhythm: &Rhythm, run_ms: u32) -> bool {
    let segment_ms = rhythm.long_segment_ms + rhythm.silence_ms_to_end_long;
    let koszt_ms = run_ms as u64 * segment_ms as u64 / 20_000;
    koszt_ms * 100 > segment_ms as u64 * 85
}

/// Wygodny skrót dla dialogu: profil ustawiony w całej konfiguracji.
pub fn current(cfg: &DaemonConfig) -> Option<&'static Rhythm> {
    detect(&cfg.vad)
}
```

- [ ] **Krok 5: uruchom testy profili**

Run: `cargo test -p diktate-cli profile::`
Oczekiwane: PASS (4 testy).

- [ ] **Krok 6: napisz testy różnic**

`setup/diff.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use diktate_config::DaemonConfig;

    #[test]
    fn brak_zmian_to_pusta_lista() {
        let cfg = DaemonConfig::default();
        assert!(changes(&cfg, &cfg).unwrap().is_empty());
    }

    #[test]
    fn jedna_zmiana_z_obiema_wartościami() {
        let przed = DaemonConfig::default();
        let mut po = przed.clone();
        po.vad.long_segment_ms = 4_000;

        let zmiany = changes(&przed, &po).unwrap();

        assert_eq!(zmiany.len(), 1);
        assert_eq!(zmiany[0].field, "vad.long_segment_ms");
        assert_eq!(zmiany[0].before, "2500");
        assert_eq!(zmiany[0].after, "4000");
    }

    #[test]
    fn zmiana_w_dwóch_sekcjach() {
        let przed = DaemonConfig::default();
        let mut po = przed.clone();
        po.whisper.language = "en".to_string();
        po.audio.device = "hw:1".to_string();

        let zmiany = changes(&przed, &po).unwrap();
        let pola: Vec<&str> = zmiany.iter().map(|z| z.field.as_str()).collect();

        assert_eq!(pola, vec!["audio.device", "whisper.language"]);
    }
}
```

- [ ] **Krok 7: napisz `diff.rs`**

```rust
//! Lista różnic między konfiguracją bieżącą a tą po dialogu.
//!
//! Porównujemy nie pola struktur, tylko ich postać w TOML-u — dzięki temu
//! dodanie pola w `diktate-config` nie wymaga tknięcia tego pliku, a kreator
//! nie ma własnej, rozjeżdżającej się listy pól.

use anyhow::Result;
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    /// Nazwa z kropką, tak jak w pliku: `vad.long_segment_ms`.
    pub field: String,
    pub before: String,
    pub after: String,
}

pub fn changes<T: Serialize>(before: &T, after: &T) -> Result<Vec<Change>> {
    let before = flatten(&toml::Value::try_from(before)?);
    let after = flatten(&toml::Value::try_from(after)?);
    let mut out = Vec::new();
    for (field, po) in &after {
        let przed = before.get(field).cloned().unwrap_or_default();
        if &przed != po {
            out.push(Change {
                field: field.clone(),
                before: przed,
                after: po.clone(),
            });
        }
    }
    out.sort_by(|a, b| a.field.cmp(&b.field));
    Ok(out)
}

/// Spłaszcza tabele TOML-a do par `sekcja.pole` → wartość.
fn flatten(value: &toml::Value) -> std::collections::BTreeMap<String, String> {
    let mut out = std::collections::BTreeMap::new();
    walk(value, String::new(), &mut out);
    out
}

fn walk(
    value: &toml::Value,
    prefix: String,
    out: &mut std::collections::BTreeMap<String, String>,
) {
    match value {
        toml::Value::Table(table) => {
            for (key, inner) in table {
                let path = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                walk(inner, path, out);
            }
        }
        toml::Value::String(s) => {
            out.insert(prefix, s.clone());
        }
        inne => {
            out.insert(prefix, inne.to_string());
        }
    }
}
```

- [ ] **Krok 8: uruchom testy różnic**

Run: `cargo test -p diktate-cli diff::`
Oczekiwane: PASS (3 testy).

- [ ] **Krok 9: napisz testy zapisu**

`setup/write.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use diktate_config::{DaemonConfig, GtkConfig};

    #[test]
    fn zapisany_daemon_toml_da_się_wczytać_z_powrotem() {
        let mut cfg = DaemonConfig::default();
        cfg.whisper.language = "en".to_string();
        cfg.vad.long_segment_ms = 4_000;

        let text = daemon_toml(&cfg);
        let wczytany = DaemonConfig::from_toml(&text).unwrap();

        assert_eq!(wczytany.whisper.language, "en");
        assert_eq!(wczytany.vad.long_segment_ms, 4_000);
    }

    #[test]
    fn zapisany_gtk_toml_da_się_wczytać_z_powrotem() {
        let mut cfg = GtkConfig::default();
        cfg.tray.enabled = false;
        let wczytany = GtkConfig::from_toml(&gtk_toml(&cfg)).unwrap();
        assert!(!wczytany.tray.enabled);
    }

    #[test]
    fn plik_ma_polskie_komentarze() {
        // Plik po kreatorze ma nadawać się do dalszej ręcznej edycji,
        // a nie być zrzutem serializera.
        let text = daemon_toml(&DaemonConfig::default());
        assert!(text.contains("# "), "brak komentarzy w wygenerowanym pliku");
        assert!(text.contains("cisza domykająca"));
    }

    #[test]
    fn zapis_zostawia_kopię_poprzedniej_wersji() {
        let dir = std::env::temp_dir()
            .join(format!("diktate-write-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("daemon.toml");
        std::fs::write(&path, "stara zawartość").unwrap();

        save(&path, "nowa zawartość").unwrap();

        assert_eq!(std::fs::read_to_string(&path).unwrap(), "nowa zawartość");
        assert_eq!(
            std::fs::read_to_string(dir.join("daemon.toml.bak")).unwrap(),
            "stara zawartość"
        );
    }
}
```

- [ ] **Krok 10: napisz `write.rs`**

```rust
//! Zapis konfiguracji: plik z komentarzami, kopia zapasowa, `rename`.
//!
//! Ręczne komentarze użytkownika nie są zachowywane — plik powstaje od nowa,
//! a poprzednia wersja ląduje w `.bak`. Świadoma rezygnacja z `toml_edit`:
//! zachowanie cudzego układu kosztuje osobną ścieżkę zapisu, a plik jest
//! w całości generowalny. Sam zapis jest atomowy (`docs/04`): inaczej
//! obserwator katalogu potrafi złapać plik w połowie pisania.

use std::path::Path;

use anyhow::{Context, Result};
use diktate_config::{DaemonConfig, GtkConfig};

pub fn daemon_toml(cfg: &DaemonConfig) -> String {
    let mut out = String::new();
    out.push_str("# daemon.toml — wygenerowane przez `diktate setup`.\n");
    out.push_str("# Opis wszystkich pól: docs/04-konfiguracja.md\n\n");

    out.push_str("[audio]\n");
    out.push_str(&pole("device", &cfg.audio.device, "urządzenie wejściowe; \"default\" to systemowe"));
    out.push_str(&liczba("pre_roll_ms", cfg.audio.pre_roll_ms, "ile ciszy sprzed mowy dokleić, żeby nie uciąć pierwszej sylaby"));
    out.push_str(&liczba("level_interval_ms", cfg.audio.level_interval_ms, "odstęp między odczytami poziomu dla wskaźnika"));

    out.push_str("\n[vad]\n");
    out.push_str(&format!("threshold = {}  # próg decyzyjny Silero, 0.0–1.0\n", cfg.vad.threshold));
    out.push_str(&liczba("silence_ms_to_end", cfg.vad.silence_ms_to_end, "cisza domykająca krótką wypowiedź"));
    out.push_str(&liczba("silence_ms_to_end_long", cfg.vad.silence_ms_to_end_long, "cisza domykająca wypowiedź już rozpędzoną"));
    out.push_str(&liczba("long_segment_ms", cfg.vad.long_segment_ms, "od tej długości obowiązuje krótszy próg ciszy"));
    out.push_str(&liczba("min_speech_ms", cfg.vad.min_speech_ms, "krótsze odrzucane jako trzask"));
    out.push_str(&liczba("max_segment_ms", cfg.vad.max_segment_ms, "twarde cięcie; okno whispera to 30 s"));

    out.push_str("\n[whisper]\n");
    out.push_str(&pole("model", &cfg.whisper.model, "ścieżka ggml; puste = $DIKTATE_MODEL, potem ~/.diktate/models"));
    out.push_str(&pole("language", &cfg.whisper.language, "kod ISO albo \"auto\""));
    out.push_str(&pole("initial_prompt", &cfg.whisper.initial_prompt, "terminologia i nazwy własne"));
    out.push_str(&liczba("beam_size", cfg.whisper.beam_size, "1 = dekodowanie zachłanne"));
    out.push_str(&liczba("threads", cfg.whisper.threads, "0 = decyzja whispera"));

    out.push_str("\n[queue]\n");
    out.push_str(&format!(
        "max_pending_segments = {}  # ile segmentów może czekać na transkrypcję\n",
        cfg.queue.max_pending_segments
    ));
    out
}

pub fn gtk_toml(cfg: &GtkConfig) -> String {
    let mut out = String::new();
    out.push_str("# gtk.toml — wygenerowane przez `diktate setup`.\n");
    out.push_str("# Opis wszystkich pól: docs/04-konfiguracja.md\n\n");

    out.push_str("[window]\n");
    out.push_str(&liczba("width", cfg.window.width as u32, "szerokość okna w pikselach"));
    out.push_str(&liczba("height", cfg.window.height as u32, "wysokość okna w pikselach"));
    out.push_str(&flaga("show_level_meter", cfg.window.show_level_meter, "wskaźnik poziomu — brak wychyleń od razu mówi, że mikrofon jest nie ten"));

    out.push_str("\n[daemon]\n");
    out.push_str(&flaga("autostart", cfg.daemon.autostart, "klient sam uruchamia daemona, gdy ten nie działa"));
    out.push_str(&format!(
        "start_timeout_ms = {}  # ile czekać na gniazdo; mieści wczytanie modelu\n",
        cfg.daemon.start_timeout_ms
    ));

    out.push_str("\n[dictation]\n");
    out.push_str(&flaga("insert_at_cursor", cfg.dictation.insert_at_cursor, "segment wchodzi w miejscu karetki, nie na końcu notatki"));
    out.push_str(&flaga("auto_space", cfg.dictation.auto_space, "spacja na stykach wypowiedzi"));
    out.push_str(&flaga("strip_trailing_ellipsis", cfg.dictation.strip_trailing_ellipsis, "usuwanie wielokropka po pauzie na oddech"));
    out.push_str(&flaga("follow_caret", cfg.dictation.follow_caret, "przewijanie do punktu wstawiania"));

    out.push_str("\n[shortcuts]\n");
    out.push_str(&pole("copy", &cfg.shortcuts.copy, "kopiowanie notatki"));
    out.push_str(&pole("undo", &cfg.shortcuts.undo, "cofnięcie"));
    out.push_str(&pole("redo", &cfg.shortcuts.redo, "ponowienie"));
    out.push_str(&pole("toggle_recording", &cfg.shortcuts.toggle_recording, "przełączenie nagrywania wewnątrz okna; skrót globalny rejestruje środowisko graficzne"));

    out.push_str("\n[tray]\n");
    out.push_str(&flaga("enabled", cfg.tray.enabled, "ikona w zasobniku"));
    out.push_str(&flaga("show_backlog", cfg.tray.show_backlog, "licznik zaległości na ikonie"));
    out
}

fn pole(nazwa: &str, wartość: &str, opis: &str) -> String {
    format!("{nazwa} = {}  # {opis}\n", toml::Value::String(wartość.to_string()))
}

fn liczba(nazwa: &str, wartość: u32, opis: &str) -> String {
    format!("{nazwa} = {wartość}  # {opis}\n")
}

fn flaga(nazwa: &str, wartość: bool, opis: &str) -> String {
    format!("{nazwa} = {wartość}  # {opis}\n")
}

/// Kopia zapasowa, potem zapis atomowy: plik tymczasowy obok i `rename`.
pub fn save(path: &Path, text: &str) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .with_context(|| format!("nie udało się utworzyć {}", dir.display()))?;
    }
    if path.exists() {
        let bak = path.with_extension(
            path.extension()
                .map(|e| format!("{}.bak", e.to_string_lossy()))
                .unwrap_or_else(|| "bak".to_string()),
        );
        std::fs::copy(path, &bak)
            .with_context(|| format!("nie udało się zapisać kopii {}", bak.display()))?;
    }
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, text)
        .with_context(|| format!("nie udało się zapisać {}", tmp.display()))?;
    std::fs::rename(&tmp, path)
        .with_context(|| format!("nie udało się nazwać {}", path.display()))?;
    Ok(())
}
```

`setup/mod.rs`:

```rust
//! Kreator konfiguracji: `diktate setup`.

pub mod diff;
pub mod profile;
pub mod write;
```

W `main.rs` dopisz `mod setup;`.

- [ ] **Krok 11: uruchom testy**

Run: `cargo test -p diktate-cli`
Oczekiwane: PASS — profile (4), różnice (3), zapis (4) plus wcześniejsze.

- [ ] **Krok 12: commit**

```bash
git add -A
git commit -m "feat(cli): profile rytmu, lista różnic i zapis konfiguracji"
```

---

### Task 7: dialog i podkomenda `diktate setup`

**Pliki:**
- Utwórz: `crates/diktate-cli/src/setup/dialog.rs`
- Utwórz: `crates/diktate-cli/src/setup/run.rs`
- Zmień: `crates/diktate-cli/src/setup/mod.rs`
- Zmień: `crates/diktate-cli/src/main.rs`

**Interfejsy:**
- Konsumuje: `setup::{profile, diff, write}`, `model::{catalog, cmd}`,
  `diktate_proto::pidfile`, `diktate_core` (lista urządzeń)
- Produkuje:
  - `setup::dialog::ask_text(pytanie: &str, domyślna: &str) -> Result<String>`
  - `setup::dialog::ask_bool(pytanie: &str, domyślna: bool) -> Result<bool>`
  - `setup::dialog::ask_u32(pytanie: &str, domyślna: u32) -> Result<u32>`
  - `setup::dialog::ask_choice(pytanie: &str, opcje: &[String],
    domyślna: usize) -> Result<usize>`
  - `setup::run::run(advanced: bool) -> Result<()>`

- [ ] **Krok 1: napisz test parsowania odpowiedzi**

Dialog czyta ze standardowego wejścia, ale rozstrzyganie odpowiedzi jest
czyste i to ono ma test. W `dialog.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pusta_odpowiedź_to_wartość_domyślna() {
        assert_eq!(parse_text("", "pl"), "pl");
        assert_eq!(parse_text("  \n", "pl"), "pl");
        assert_eq!(parse_text("en", "pl"), "en");
    }

    #[test]
    fn tak_i_nie_po_polsku() {
        assert_eq!(parse_bool("", true), Some(true));
        assert_eq!(parse_bool("t", false), Some(true));
        assert_eq!(parse_bool("tak", false), Some(true));
        assert_eq!(parse_bool("n", true), Some(false));
        assert_eq!(parse_bool("nie", true), Some(false));
        assert_eq!(parse_bool("może", true), None);
    }

    #[test]
    fn numer_opcji_liczony_od_jedynki() {
        assert_eq!(parse_choice("2", 3, 0), Some(1));
        assert_eq!(parse_choice("", 3, 2), Some(2));
        assert_eq!(parse_choice("0", 3, 0), None);
        assert_eq!(parse_choice("4", 3, 0), None);
    }
}
```

- [ ] **Krok 2: uruchom i sprawdź, że nie przechodzi**

Run: `cargo test -p diktate-cli dialog::`
Oczekiwane: FAIL — brak modułu.

- [ ] **Krok 3: napisz `dialog.rs`**

```rust
//! Pytania w terminalu: jedno na raz, z odpowiedzią domyślną pod Enterem.
//!
//! Rozstrzyganie odpowiedzi (`parse_*`) jest oddzielone od czytania wejścia,
//! bo to ono ma testy — sam odczyt jest cienki i nie ma w nim czego psuć.

use std::io::{BufRead, Write};

use anyhow::{Context, Result};

fn read_line() -> Result<String> {
    print!("> ");
    std::io::stdout().flush()?;
    let mut line = String::new();
    std::io::stdin()
        .lock()
        .read_line(&mut line)
        .context("nie udało się odczytać odpowiedzi")?;
    Ok(line)
}

pub fn parse_text(odpowiedź: &str, domyślna: &str) -> String {
    let trimmed = odpowiedź.trim();
    if trimmed.is_empty() {
        domyślna.to_string()
    } else {
        trimmed.to_string()
    }
}

pub fn parse_bool(odpowiedź: &str, domyślna: bool) -> Option<bool> {
    match odpowiedź.trim().to_lowercase().as_str() {
        "" => Some(domyślna),
        "t" | "tak" | "y" | "yes" => Some(true),
        "n" | "nie" | "no" => Some(false),
        _ => None,
    }
}

pub fn parse_choice(odpowiedź: &str, ile: usize, domyślna: usize) -> Option<usize> {
    let trimmed = odpowiedź.trim();
    if trimmed.is_empty() {
        return Some(domyślna);
    }
    let numer: usize = trimmed.parse().ok()?;
    if numer >= 1 && numer <= ile {
        Some(numer - 1)
    } else {
        None
    }
}

pub fn ask_text(pytanie: &str, domyślna: &str) -> Result<String> {
    println!("\n{pytanie} [{domyślna}]");
    Ok(parse_text(&read_line()?, domyślna))
}

pub fn ask_u32(pytanie: &str, domyślna: u32) -> Result<u32> {
    loop {
        let odpowiedź = ask_text(pytanie, &domyślna.to_string())?;
        match odpowiedź.parse() {
            Ok(wartość) => return Ok(wartość),
            Err(_) => println!("To nie jest liczba. Spróbuj jeszcze raz."),
        }
    }
}

pub fn ask_bool(pytanie: &str, domyślna: bool) -> Result<bool> {
    let podpowiedź = if domyślna { "T/n" } else { "t/N" };
    loop {
        println!("\n{pytanie} [{podpowiedź}]");
        match parse_bool(&read_line()?, domyślna) {
            Some(wartość) => return Ok(wartość),
            None => println!("Odpowiedz „tak” albo „nie”."),
        }
    }
}

pub fn ask_choice(pytanie: &str, opcje: &[String], domyślna: usize) -> Result<usize> {
    loop {
        println!("\n{pytanie}");
        for (i, opcja) in opcje.iter().enumerate() {
            let znak = if i == domyślna { "*" } else { " " };
            println!(" {znak} {}) {opcja}", i + 1);
        }
        println!("Enter = {}", domyślna + 1);
        match parse_choice(&read_line()?, opcje.len(), domyślna) {
            Some(wybór) => return Ok(wybór),
            None => println!("Podaj numer z listy."),
        }
    }
}
```

- [ ] **Krok 4: uruchom testy dialogu**

Run: `cargo test -p diktate-cli dialog::`
Oczekiwane: PASS (3 testy).

- [ ] **Krok 5: napisz `run.rs` — przebieg kreatora**

```rust
//! Przebieg kreatora: pytania, podgląd zmian, zapis, podpowiedź restartu.
//!
//! Domyślne odpowiedzi to wartości **dzisiejsze**, a nie fabryczne: kreator
//! uruchomiony po raz drugi ma poprawiać to, co jest, a nie kasować strojenie.

use anyhow::Result;
use diktate_config::{DaemonConfig, GtkConfig};

use crate::model::{catalog, cmd};
use crate::setup::{diff, dialog, profile, write};

pub fn run(advanced: bool) -> Result<()> {
    println!("Kreator konfiguracji Diktate.");
    println!("Enter przyjmuje wartość w nawiasie. Nic nie zapisujemy bez pytania.");

    let daemon_przed = DaemonConfig::load()?;
    let gtk_przed = GtkConfig::load()?;
    let mut daemon = daemon_przed.clone();
    let mut gtk = gtk_przed.clone();

    // --- model ---
    let opcje: Vec<String> = catalog::MODELS
        .iter()
        .map(|m| {
            let stan = if m.installed().is_some() { " [masz]" } else { "" };
            format!(
                "{:<22} {:>7}  {:>4.1} s/20 s  polszczyzna: {}{stan}",
                m.name,
                cmd::format_size(m.bytes),
                m.run_ms as f32 / 1000.0,
                m.polish.opis()
            )
        })
        .collect();
    let domyślny = catalog::MODELS
        .iter()
        .position(|m| daemon.whisper.model.contains(m.file))
        .unwrap_or_else(|| {
            catalog::MODELS
                .iter()
                .position(|m| m.recommended)
                .unwrap_or(0)
        });
    let wybór = dialog::ask_choice("Który model whispera?", &opcje, domyślny)?;
    let model = &catalog::MODELS[wybór];
    let path = cmd::pull_model(model)?;
    daemon.whisper.model = path.display().to_string();

    // --- język ---
    daemon.whisper.language =
        dialog::ask_text("Język transkrypcji (kod ISO albo \"auto\")", &daemon.whisper.language)?;

    // --- urządzenie ---
    let urządzenia = input_devices();
    if urządzenia.is_empty() {
        daemon.audio.device =
            dialog::ask_text("Urządzenie wejściowe", &daemon.audio.device)?;
    } else {
        let mut opcje = vec!["default (urządzenie systemowe)".to_string()];
        opcje.extend(urządzenia.iter().cloned());
        let teraz = opcje
            .iter()
            .position(|o| o.starts_with(&daemon.audio.device))
            .unwrap_or(0);
        let wybór = dialog::ask_choice("Które urządzenie wejściowe?", &opcje, teraz)?;
        daemon.audio.device = if wybór == 0 {
            "default".to_string()
        } else {
            urządzenia[wybór - 1].clone()
        };
    }

    // --- rytm ---
    let opcje: Vec<String> = profile::RHYTHMS
        .iter()
        .map(|r| format!("{:<14} {} ms — {}", r.name, r.long_segment_ms, r.opis))
        .collect();
    let teraz = profile::current(&daemon)
        .and_then(|r| profile::RHYTHMS.iter().position(|x| x.name == r.name))
        .unwrap_or(1);
    let wybór = dialog::ask_choice("Rytm segmentacji?", &opcje, teraz)?;
    let rhythm = &profile::RHYTHMS[wybór];
    if profile::too_fast_for(rhythm, model.run_ms) {
        println!(
            "\nUwaga: przy modelu {} segment w tym rytmie kosztuje więcej, niż trwa.",
            model.name
        );
        println!("Kolejka będzie rosła, a tekst pojawiał się coraz później, nie wcześniej.");
        if !dialog::ask_bool("Mimo to ustawić ten rytm?", false)? {
            return run(advanced);
        }
    }
    profile::apply(&mut daemon.vad, rhythm);

    // --- GTK ---
    gtk.tray.enabled = dialog::ask_bool("Ikona w zasobniku systemowym?", gtk.tray.enabled)?;
    gtk.shortcuts.toggle_recording = dialog::ask_text(
        "Skrót przełączający nagrywanie w oknie (globalny rejestruje środowisko graficzne)",
        &gtk.shortcuts.toggle_recording,
    )?;

    if advanced {
        zapytaj_o_resztę(&mut daemon, &mut gtk)?;
    }

    // --- podgląd i zapis ---
    let zmiany_daemon = diff::changes(&daemon_przed, &daemon)?;
    let zmiany_gtk = diff::changes(&gtk_przed, &gtk)?;
    if zmiany_daemon.is_empty() && zmiany_gtk.is_empty() {
        println!("\nNic się nie zmieniło — nie ma czego zapisywać.");
        return Ok(());
    }
    println!("\nZmiany:");
    for zmiana in zmiany_daemon.iter().chain(zmiany_gtk.iter()) {
        println!("  {}: {} → {}", zmiana.field, zmiana.before, zmiana.after);
    }
    if !dialog::ask_bool("Zapisać?", true)? {
        println!("Nic nie zapisano.");
        return Ok(());
    }

    let daemon_path = DaemonConfig::path();
    let gtk_path = GtkConfig::path();
    write::save(&daemon_path, &write::daemon_toml(&daemon))?;
    write::save(&gtk_path, &write::gtk_toml(&gtk))?;

    println!("\nZapisano:");
    println!("  {}", daemon_path.display());
    println!("  {}", gtk_path.display());
    println!("  model: {}", daemon.whisper.model);
    println!("\nWszystkie pola: diktate setup --advanced");

    if let Some(pid) = diktate_proto::pidfile::running() {
        println!(
            "\nDaemon (proces {pid}) nadal chodzi ze starą konfiguracją. \
             Bez restartu zmiany wyglądają, jakby nie działały:"
        );
        println!("  diktate-daemon --restart");
    }
    Ok(())
}

/// Nazwy urządzeń wejściowych. Pusta lista znaczy tyle, że nie da się ich
/// wymienić (brak serwera dźwięku) — wtedy pytamy o nazwę tekstem.
pub fn input_devices() -> Vec<String> {
    diktate_core::input_device_names().unwrap_or_default()
}

/// `--advanced`: każde pozostałe pole po kolei, z bieżącą wartością
/// w nawiasie.
fn zapytaj_o_resztę(daemon: &mut DaemonConfig, gtk: &mut GtkConfig) -> Result<()> {
    println!("\n--- Pozostałe pola daemon.toml ---");
    daemon.audio.pre_roll_ms = dialog::ask_u32(
        "audio.pre_roll_ms — ile ciszy sprzed mowy dokleić",
        daemon.audio.pre_roll_ms,
    )?;
    daemon.audio.level_interval_ms = dialog::ask_u32(
        "audio.level_interval_ms — odstęp odczytów poziomu",
        daemon.audio.level_interval_ms,
    )?;
    daemon.vad.min_speech_ms = dialog::ask_u32(
        "vad.min_speech_ms — krótsze wypowiedzi odrzucamy jako trzask",
        daemon.vad.min_speech_ms,
    )?;
    daemon.vad.max_segment_ms = dialog::ask_u32(
        "vad.max_segment_ms — twarde cięcie (okno whispera to 30 000 ms)",
        daemon.vad.max_segment_ms,
    )?;
    daemon.vad.silence_ms_to_end = dialog::ask_u32(
        "vad.silence_ms_to_end — cisza domykająca krótką wypowiedź",
        daemon.vad.silence_ms_to_end,
    )?;
    daemon.vad.silence_ms_to_end_long = dialog::ask_u32(
        "vad.silence_ms_to_end_long — cisza domykająca wypowiedź rozpędzoną",
        daemon.vad.silence_ms_to_end_long,
    )?;
    daemon.vad.long_segment_ms = dialog::ask_u32(
        "vad.long_segment_ms — od tej długości obowiązuje krótszy próg",
        daemon.vad.long_segment_ms,
    )?;
    daemon.whisper.initial_prompt = dialog::ask_text(
        "whisper.initial_prompt — terminologia i nazwy własne",
        &daemon.whisper.initial_prompt,
    )?;
    daemon.whisper.beam_size = dialog::ask_u32(
        "whisper.beam_size — 1 to dekodowanie zachłanne",
        daemon.whisper.beam_size,
    )?;
    daemon.whisper.threads = dialog::ask_u32(
        "whisper.threads — 0 zostawia decyzję whisperowi",
        daemon.whisper.threads,
    )?;
    daemon.queue.max_pending_segments = dialog::ask_u32(
        "queue.max_pending_segments — ile segmentów może czekać",
        daemon.queue.max_pending_segments as u32,
    )? as usize;

    println!("\n--- Pozostałe pola gtk.toml ---");
    gtk.window.width = dialog::ask_u32("window.width", gtk.window.width as u32)? as i32;
    gtk.window.height = dialog::ask_u32("window.height", gtk.window.height as u32)? as i32;
    gtk.window.show_level_meter =
        dialog::ask_bool("window.show_level_meter — wskaźnik poziomu?", gtk.window.show_level_meter)?;
    gtk.daemon.autostart =
        dialog::ask_bool("daemon.autostart — klient sam uruchamia daemona?", gtk.daemon.autostart)?;
    gtk.daemon.start_timeout_ms = dialog::ask_u32(
        "daemon.start_timeout_ms — ile czekać na gniazdo",
        gtk.daemon.start_timeout_ms as u32,
    )? as u64;
    gtk.dictation.insert_at_cursor = dialog::ask_bool(
        "dictation.insert_at_cursor — wstawiać w miejscu karetki?",
        gtk.dictation.insert_at_cursor,
    )?;
    gtk.dictation.auto_space =
        dialog::ask_bool("dictation.auto_space — spacja na stykach?", gtk.dictation.auto_space)?;
    gtk.dictation.strip_trailing_ellipsis = dialog::ask_bool(
        "dictation.strip_trailing_ellipsis — usuwać końcowy wielokropek?",
        gtk.dictation.strip_trailing_ellipsis,
    )?;
    gtk.dictation.follow_caret = dialog::ask_bool(
        "dictation.follow_caret — przewijać do punktu wstawiania?",
        gtk.dictation.follow_caret,
    )?;
    gtk.shortcuts.copy = dialog::ask_text("shortcuts.copy", &gtk.shortcuts.copy)?;
    gtk.shortcuts.undo = dialog::ask_text("shortcuts.undo", &gtk.shortcuts.undo)?;
    gtk.shortcuts.redo = dialog::ask_text("shortcuts.redo", &gtk.shortcuts.redo)?;
    gtk.tray.show_backlog =
        dialog::ask_bool("tray.show_backlog — licznik zaległości na ikonie?", gtk.tray.show_backlog)?;
    Ok(())
}
```

- [ ] **Krok 6: dodaj listę urządzeń w rdzeniu**

`crates/diktate-core/src/capture.rs` — dopisz obok `build_stream`:

```rust
/// Nazwy urządzeń wejściowych widzianych przez cpal.
///
/// Kreator wymienia je przy pytaniu o mikrofon. Brak serwera dźwięku nie
/// jest tu błędem krytycznym — lista bywa pusta i wtedy pytamy o nazwę
/// tekstem.
pub fn input_device_names() -> Result<Vec<String>> {
    let host = cpal::default_host();
    let devices = host
        .input_devices()
        .context("nie udało się wymienić urządzeń wejściowych")?;
    Ok(devices.filter_map(|d| d.name().ok()).collect())
}
```

Dopisz `use cpal::traits::{DeviceTrait, HostTrait};` jeśli któregoś brakuje,
i wyeksportuj funkcję w `crates/diktate-core/src/lib.rs`:

```rust
pub use capture::input_device_names;
```

- [ ] **Krok 7: podłącz podkomendę**

W `main.rs`, w `enum Command`:

```rust
    /// Kreator konfiguracji: model, język, mikrofon, rytm, skróty.
    Setup {
        /// Zapytaj o każde pole daemon.toml i gtk.toml, nie tylko o podstawy.
        #[arg(long)]
        advanced: bool,
    },
```

W `match`:

```rust
        Command::Setup { advanced } => setup::run::run(advanced),
```

W `setup/mod.rs` dopisz `pub mod dialog;` i `pub mod run;`.

- [ ] **Krok 8: testy i uruchomienie na sucho**

Run: `cargo test --workspace && cargo clippy --workspace --all-targets`
Oczekiwane: PASS, zero ostrzeżeń.

Run: `XDG_CONFIG_HOME=/tmp/diktate-proba cargo run -p diktate-cli -- setup`
Oczekiwane: dialog przechodzi do końca, a `/tmp/diktate-proba/diktate/`
zawiera `daemon.toml` i `gtk.toml` z komentarzami. Sprawdź, że wynik wczyta
się z powrotem:

```bash
XDG_CONFIG_HOME=/tmp/diktate-proba cargo run -p diktate-cli -- record --wav testdata/jfk.wav --language en
```

- [ ] **Krok 9: commit**

```bash
git add -A
git commit -m "feat(cli): diktate setup — kreator konfiguracji"
```

---

### Task 8: domknięcie dokumentacji i sprawdzenie całości

Specyfikacja (`docs/01`, `docs/04`, `docs/08`, `docs/09`, `README.md`,
`CLAUDE.md`) została zaktualizowana **przed** wdrożeniem, w commicie
`604633c`. To zadanie tylko sprawdza, czy kod się z nią zgadza, i domyka
rozjazdy, jeśli jakieś powstały.

**Pliki:**
- Zmień (jeśli trzeba): `README.md`, `docs/04-konfiguracja.md`, `CLAUDE.md`

- [ ] **Krok 1: sprawdź zgodność ze specyfikacją**

Przejdź `docs/04-konfiguracja.md`, sekcja „Kreator `diktate setup`”, i dla
każdego zdania wskaż miejsce w kodzie, które je realizuje. Gdzie kod odbiega
— popraw kod, a nie dokument, chyba że odchylenie było świadome i wtedy
popraw dokument, wpisując powód.

- [ ] **Krok 2: sprawdź przykłady z README**

```bash
cargo run -p diktate-cli -- model list
cargo run -p diktate-cli -- setup --advanced   # przerwij Ctrl+C po kilku pytaniach
```

Oczekiwane: komendy działają dokładnie tak, jak opisuje je README.

- [ ] **Krok 3: pełna weryfikacja**

```bash
cargo test --workspace
cargo clippy --workspace --all-targets
cargo build --workspace
```

Oczekiwane: testy przechodzą, clippy milczy, build się udaje.

- [ ] **Krok 4: commit**

```bash
git add -A
git commit -m "docs: dokumentacja zrównana ze stanem kreatora"
```
