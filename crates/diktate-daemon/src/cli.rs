//! Rozpoznanie trybu uruchomienia z argumentów wiersza poleceń.
//!
//! Daemon jest jeden na sesję użytkownika, więc poza zwykłym startem
//! potrzebne są dwa tryby administracyjne: `--restart` (zastąp działającego
//! poprzednika) i `--stop` (tylko go ubij). Rozpoznanie trybu jest tu
//! wydzielone, żeby dało się je przetestować bez uruchamiania procesu.

use anyhow::{bail, Result};

/// Tryb, w jakim uruchomiono binarkę.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    /// Zwykły start: odmawia, gdy inny daemon już działa.
    Run,
    /// Ubija działającego poprzednika i startuje na jego miejsce.
    Restart,
    /// Ubija działającego daemona i kończy pracę.
    Stop,
    /// Wypisuje pomoc i kończy pracę — wypisaniem zajmuje się `main`.
    Help,
}

/// Tekst pomocy — jedyne źródło prawdy o dostępnych przełącznikach.
pub const USAGE: &str = "\
Użycie: diktate-daemon [OPCJA]

    --restart    zastąp działającego daemona nowym procesem
    --stop       zatrzymaj działającego daemona i zakończ
    --help       pokaż tę pomoc

Bez opcji daemon startuje normalnie i odmawia, gdy inny już działa.";

/// Rozpoznaje tryb z argumentów (bez nazwy programu).
pub fn parse<I, S>(args: I) -> Result<Command>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut command = None;
    for arg in args {
        let arg = arg.as_ref();
        let next = match arg {
            "--restart" => Command::Restart,
            "--stop" => Command::Stop,
            "--help" | "-h" => Command::Help,
            other => bail!("nieznana opcja {other}\n\n{USAGE}"),
        };
        if command.is_some_and(|previous| previous != next) {
            bail!("podaj jedną opcję naraz\n\n{USAGE}");
        }
        command = Some(next);
    }
    Ok(command.unwrap_or(Command::Run))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_slice(args: &[&str]) -> Result<Command> {
        parse(args.iter().copied())
    }

    #[test]
    fn brak_argumentow_to_zwykly_start() {
        assert_eq!(parse_slice(&[]).unwrap(), Command::Run);
    }

    #[test]
    fn restart_jest_rozpoznawany() {
        assert_eq!(parse_slice(&["--restart"]).unwrap(), Command::Restart);
    }

    #[test]
    fn stop_jest_rozpoznawany() {
        assert_eq!(parse_slice(&["--stop"]).unwrap(), Command::Stop);
    }

    #[test]
    fn nieznana_opcja_jest_bledem_z_pomoca() {
        // Cicha akceptacja literówki znaczyłaby, że `--restrat` startuje
        // drugiego daemona zamiast zastąpić pierwszego — dokładnie ta
        // pomyłka, której ta flaga ma zapobiegać.
        let err = parse_slice(&["--restrat"]).expect_err("literówka musi być błędem");
        let message = format!("{err:#}");
        assert!(
            message.contains("--restrat"),
            "komunikat ma wskazać winny argument, a jest: {message}"
        );
        assert!(
            message.contains("--restart"),
            "komunikat ma podpowiedzieć poprawne opcje, a jest: {message}"
        );
    }

    #[test]
    fn help_jest_osobnym_trybem_a_nie_wyjsciem_w_srodku_parsowania() {
        // Rozpoznanie argumentów ma być czystą funkcją — decyzję o wypisaniu
        // pomocy i zakończeniu podejmuje `main`, nie parser.
        assert_eq!(parse_slice(&["--help"]).unwrap(), Command::Help);
        assert_eq!(parse_slice(&["-h"]).unwrap(), Command::Help);
    }

    #[test]
    fn dwie_opcje_naraz_sa_bledem() {
        // `--restart --stop` nie ma spójnego znaczenia; lepiej odmówić, niż
        // zgadywać, którą intencję miał użytkownik.
        let err = parse_slice(&["--restart", "--stop"]).expect_err("dwa tryby naraz nie mają sensu");
        assert!(format!("{err:#}").contains("jedn"));
    }
}
