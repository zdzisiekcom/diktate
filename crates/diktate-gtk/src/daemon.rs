//! Uruchamianie daemona w tle, gdy jeszcze nie działa.
//!
//! Zasada: dla użytkownika Diktate jest jedną aplikacją. Kliknięcie ikony
//! ma otworzyć okno gotowe do dyktowania, a nie okno z komunikatem „brak
//! daemona" i poleceniem do wklejenia w terminal. Dlatego klient, który nie
//! zastał gniazda, sam podnosi proces daemona i czeka, aż ten się odezwie.
//!
//! Uruchomiony proces jest **odłączony**: własna grupa procesów sprawia, że
//! Ctrl+C w terminalu, z którego wyszedł klient, nie zabija daemona razem
//! z oknem. Daemon przeżywa zamknięcie okna — to celowe, bo kolejne
//! uruchomienie klienta zastaje gotowe gniazdo i wczytany model.
//!
//! Jeden wątek czeka na zakończenie potomka: bez tego martwy daemon
//! zostawałby zombim aż do końca życia okna, a jego kod wyjścia — jedyna
//! informacja, gdy proces przewrócił się na starcie (brak modelu, zajęty
//! mikrofon) — przepadałby bez śladu.

use anyhow::{Context, Result};
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Nazwa pliku wykonywalnego daemona.
pub const BINARY: &str = "diktate-daemon";

/// Zmienna wskazująca konkretny plik wykonywalny — używana w testach
/// i przy nietypowej instalacji.
pub const ENV_BINARY: &str = "DIKTATE_DAEMON";

/// Wybór pliku wykonywalnego: zmienna środowiskowa, potem katalog obok
/// klienta, na końcu `PATH`.
///
/// Katalog obok klienta jest ważniejszy niż `PATH`, bo to on rozstrzyga
/// najczęstszy przypadek pracy nad projektem: `target/debug/diktate-gtk`
/// ma uruchomić `target/debug/diktate-daemon`, a nie wersję zainstalowaną
/// w systemie z poprzedniego tygodnia.
///
/// Ścieżka ze zmiennej wraca nawet wtedy, gdy nic pod nią nie ma — błąd
/// „nie ma takiego pliku" z jawnie podaną ścieżką jest czytelniejszy niż
/// ciche sięgnięcie po inny plik wykonywalny.
fn pick_binary(
    env: Option<&OsStr>,
    exe_dir: Option<&Path>,
    exists: impl Fn(&Path) -> bool,
) -> PathBuf {
    if let Some(path) = env.filter(|value| !value.is_empty()) {
        return PathBuf::from(path);
    }
    if let Some(candidate) = exe_dir.map(|dir| dir.join(BINARY)) {
        if exists(&candidate) {
            return candidate;
        }
    }
    PathBuf::from(BINARY)
}

/// Ścieżka daemona dla bieżącego procesu.
pub fn binary_path() -> PathBuf {
    let env: Option<OsString> = std::env::var_os(ENV_BINARY);
    let exe = std::env::current_exe().ok();
    let exe_dir = exe.as_deref().and_then(Path::parent);
    pick_binary(env.as_deref(), exe_dir, |path| path.is_file())
}

/// Uruchamia daemona w tle i zwraca jego identyfikator procesu.
///
/// Wyjście diagnostyczne daemona zostaje na odziedziczonym `stderr` —
/// przy starcie z terminala widać wczytywanie modelu, a przy starcie
/// z menu trafia ono do journalu sesji.
pub fn spawn() -> Result<u32> {
    use std::os::unix::process::CommandExt;

    let binary = binary_path();
    let mut child = Command::new(&binary)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        // Własna grupa procesów: sygnał z terminala klienta nie dosięga
        // daemona.
        .process_group(0)
        .spawn()
        .with_context(|| format!("nie udało się uruchomić {}", binary.display()))?;

    let pid = child.id();
    std::thread::spawn(move || match child.wait() {
        Ok(status) if status.success() => {}
        Ok(status) => eprintln!("diktate-gtk: daemon zakończył się: {status}"),
        Err(err) => eprintln!("diktate-gtk: nie udało się odczekać na daemona: {err}"),
    });
    Ok(pid)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zmienna_srodowiskowa_wygrywa_z_katalogiem_obok() {
        let wybor = pick_binary(
            Some(OsStr::new("/opt/diktate/bin/diktate-daemon")),
            Some(Path::new("/home/ktos/target/debug")),
            |_| true,
        );
        assert_eq!(wybor, PathBuf::from("/opt/diktate/bin/diktate-daemon"));
    }

    #[test]
    fn pusta_zmienna_jest_traktowana_jak_brak() {
        let wybor = pick_binary(Some(OsStr::new("")), None, |_| false);
        assert_eq!(wybor, PathBuf::from(BINARY));
    }

    #[test]
    fn katalog_obok_klienta_gdy_plik_istnieje() {
        let wybor = pick_binary(None, Some(Path::new("/home/ktos/target/debug")), |path| {
            path == Path::new("/home/ktos/target/debug/diktate-daemon")
        });
        assert_eq!(wybor, PathBuf::from("/home/ktos/target/debug/diktate-daemon"));
    }

    #[test]
    fn sama_nazwa_gdy_obok_klienta_nic_nie_ma() {
        let wybor = pick_binary(None, Some(Path::new("/usr/bin")), |_| false);
        assert_eq!(wybor, PathBuf::from(BINARY));
        assert!(wybor.parent().is_none_or(|parent| parent.as_os_str().is_empty()));
    }
}
