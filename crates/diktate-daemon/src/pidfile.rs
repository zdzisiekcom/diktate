//! Ślad po działającym daemonie: plik z numerem procesu obok gniazda.
//!
//! Daemon jest jeden na sesję użytkownika, ale samo gniazdo tego nie
//! pilnuje — `UnixListener::bind` po usunięciu pliku gniazda uda się także
//! wtedy, gdy poprzedni daemon nadal żyje, a wtedy dwa procesy chodzą
//! równolegle i ten starszy trzyma nieaktualną konfigurację. Plik PID
//! zamyka tę dziurę: po nim wiadomo, kogo zastąpić (`--restart`) albo
//! zatrzymać (`--stop`).
//!
//! Plik bywa nieaktualny — po `kill -9` albo po restarcie maszyny zostaje
//! numer procesu, którego już nie ma, albo, gorzej, numer przejęty przez
//! zupełnie inny program. Dlatego samo istnienie pliku nic nie znaczy:
//! liczy się dopiero [`running`], które sprawdza, czy proces o tym numerze
//! faktycznie żyje.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};

/// Ścieżka pliku PID — obok gniazda, w tym samym katalogu sesji.
pub fn pid_path() -> PathBuf {
    match std::env::var_os("XDG_RUNTIME_DIR") {
        Some(dir) => PathBuf::from(dir).join("diktate.pid"),
        None => PathBuf::from("/tmp/diktate.pid"),
    }
}

/// Czy proces o tym numerze istnieje.
pub fn process_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    // Samo istnienie `/proc/<pid>` nie wystarcza: zakończony proces, którego
    // rodzic jeszcze nie zebrał, zostaje jako zombie i ma swój katalog, choć
    // już nie pracuje. Stan procesu trzymany jest w `stat` jako pierwsze pole
    // za nazwą — a że nazwa jest w nawiasach i sama może zawierać nawias,
    // szukamy OSTATNIEGO `)`.
    let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
        return false;
    };
    let Some((_, after_name)) = stat.rsplit_once(')') else {
        return false;
    };
    after_name.split_whitespace().next() != Some("Z")
}

/// Numer działającego daemona, jeśli taki jest. Nieaktualny plik (proces
/// już nie żyje) traktujemy jak jego brak — i sprzątamy przy okazji.
pub fn running() -> Option<u32> {
    running_at(&pid_path())
}

/// Jak [`running`], ale dla wskazanej ścieżki — rozdzielone, żeby dało się
/// to przetestować bez dotykania prawdziwego katalogu sesji.
pub fn running_at(path: &Path) -> Option<u32> {
    let pid: u32 = std::fs::read_to_string(path).ok()?.trim().parse().ok()?;
    if process_alive(pid) {
        Some(pid)
    } else {
        let _ = std::fs::remove_file(path);
        None
    }
}

/// Zapisuje numer bieżącego procesu.
pub fn write_current() -> Result<()> {
    let path = pid_path();
    std::fs::write(&path, std::process::id().to_string())
        .with_context(|| format!("nie udało się zapisać {}", path.display()))
}

/// Ile czekamy, aż proces sam zakończy pracę po `SIGTERM`, zanim sięgniemy
/// po `SIGKILL`. Daemon musi zdążyć domknąć trwającą sesję i zwolnić model,
/// ale użytkownik czeka przy terminalu — stąd sekundy, nie dziesiątki sekund.
const GRACE: Duration = Duration::from_secs(3);

/// Zatrzymuje proces o podanym numerze: najpierw grzecznie (`SIGTERM`),
/// a gdy po [`GRACE`] nadal żyje — `SIGKILL`.
///
/// Zwraca `true`, jeśli proces faktycznie zniknął.
pub fn terminate(pid: u32) -> bool {
    if !process_alive(pid) {
        return true;
    }
    send_signal(pid, libc::SIGTERM);
    if wait_gone(pid, GRACE) {
        return true;
    }
    eprintln!("diktate-daemon: proces {pid} nie zareagował na SIGTERM — wysyłam SIGKILL");
    send_signal(pid, libc::SIGKILL);
    wait_gone(pid, GRACE)
}

fn send_signal(pid: u32, signal: i32) {
    // SAFETY: `kill` z prawidłowym numerem procesu jest bezpieczne; błąd
    // (brak procesu, brak uprawnień) sygnalizowany jest kodem powrotu,
    // który i tak weryfikujemy przez `wait_gone`.
    unsafe {
        libc::kill(pid as libc::pid_t, signal);
    }
}

/// Odpytuje co 50 ms, aż proces zniknie albo minie `limit`.
fn wait_gone(pid: u32, limit: Duration) -> bool {
    let start = Instant::now();
    while start.elapsed() < limit {
        if !process_alive(pid) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    !process_alive(pid)
}

/// Usuwa plik PID. Brak pliku nie jest błędem.
pub fn remove() {
    let _ = std::fs::remove_file(pid_path());
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ścieżka robocza pojedynczego testu — testy w jednym procesie biegną
    /// równolegle, więc każdy musi mieć własny plik.
    fn temp_pid_file(nazwa: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("diktate-test-{}-{nazwa}.pid", std::process::id()));
        let _ = std::fs::remove_file(&path);
        path
    }

    #[test]
    fn nieaktualny_plik_pid_znaczy_tyle_co_jego_brak() {
        // Po `kill -9` albo restarcie maszyny plik zostaje, a procesu nie ma.
        // Gdyby `running_at` wierzyło samej zawartości pliku, `--restart`
        // próbowałby ubić nieistniejący proces, a zwykły start odmawiałby
        // w kółko bez powodu.
        let path = temp_pid_file("nieaktualny");
        std::fs::write(&path, "0").unwrap();
        assert_eq!(running_at(&path), None);
        assert!(!path.exists(), "nieaktualny plik ma zostać posprzątany");
    }

    #[test]
    fn plik_wskazujacy_zywy_proces_zwraca_jego_numer() {
        let path = temp_pid_file("zywy");
        let me = std::process::id();
        std::fs::write(&path, me.to_string()).unwrap();
        assert_eq!(running_at(&path), Some(me));
        assert!(path.exists(), "żywy proces — plik zostaje nietknięty");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn smieci_w_pliku_nie_wywracaja_odczytu() {
        let path = temp_pid_file("smieci");
        std::fs::write(&path, "to nie jest numer").unwrap();
        assert_eq!(running_at(&path), None);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn terminate_ubija_prawdziwy_proces() {
        // Bez atrap: startujemy prawdziwy proces, ubijamy go i sprawdzamy,
        // że zniknął. Atrapa sygnału nie dowiodłaby niczego o tym, czy
        // `--restart` naprawdę zwalnia gniazdo.
        let child = std::process::Command::new("sleep")
            .arg("60")
            .spawn()
            .expect("nie udało się uruchomić procesu testowego");
        let pid = child.id();
        assert!(process_alive(pid), "proces testowy powinien żyć");

        assert!(terminate(pid), "terminate powinien potwierdzić zniknięcie");
        assert!(!process_alive(pid), "proces testowy powinien zniknąć");

        // Sprzątamy wpis zombie, żeby nie został w tablicy procesów.
        let mut child = child;
        let _ = child.wait();
    }

    #[test]
    fn zombie_nie_liczy_sie_jako_zywy_proces() {
        // Proces zakończony, którego rodzic jeszcze nie zebrał, zostaje
        // w tablicy jako zombie — a `/proc/<pid>` wciąż istnieje. Gdyby
        // uznać go za żywego, `--restart` czekałby na kogoś, kto już nie
        // pracuje, a zwykły start odmawiałby startu przez nieboszczyka.
        let mut child = std::process::Command::new("sleep")
            .arg("60")
            .spawn()
            .expect("nie udało się uruchomić procesu testowego");
        let pid = child.id();
        send_signal(pid, libc::SIGKILL);
        // Czekamy, aż jądro przestawi stan na zombie — celowo BEZ `wait()`.
        let start = Instant::now();
        while process_alive(pid) && start.elapsed() < Duration::from_secs(3) {
            std::thread::sleep(Duration::from_millis(20));
        }

        assert!(
            PathBuf::from(format!("/proc/{pid}")).is_dir(),
            "warunek testu: katalog zombie ma jeszcze istnieć"
        );
        assert!(!process_alive(pid), "zombie nie jest żywym procesem");

        let _ = child.wait();
    }

    #[test]
    fn terminate_na_nieistniejacym_procesie_jest_bezpieczny() {
        // Nieaktualny plik PID prowadzi do numeru, którego nie ma —
        // `--restart` ma wtedy po prostu ruszyć dalej, a nie wywalić się.
        assert!(terminate(0));
    }

    #[test]
    fn biezacy_proces_zyje() {
        assert!(process_alive(std::process::id()));
    }

    #[test]
    fn numer_ktorego_nie_ma_nie_zyje() {
        // PID 0 to w Linuksie „grupa procesów wołającego", nigdy zwykły
        // proces — bezpieczny numer na test strony negatywnej, w
        // przeciwieństwie do dużej losowej liczby, którą system mógłby
        // komuś przydzielić w trakcie testu.
        assert!(!process_alive(0));
    }
}
