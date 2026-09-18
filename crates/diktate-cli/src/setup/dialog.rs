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

pub fn parse_text(answer: &str, default: &str) -> String {
    let trimmed = answer.trim();
    if trimmed.is_empty() {
        default.to_string()
    } else {
        trimmed.to_string()
    }
}

pub fn parse_bool(answer: &str, default: bool) -> Option<bool> {
    match answer.trim().to_lowercase().as_str() {
        "" => Some(default),
        "t" | "tak" | "y" | "yes" => Some(true),
        "n" | "nie" | "no" => Some(false),
        _ => None,
    }
}

pub fn parse_choice(answer: &str, count: usize, default: usize) -> Option<usize> {
    let trimmed = answer.trim();
    if trimmed.is_empty() {
        return Some(default);
    }
    let number: usize = trimmed.parse().ok()?;
    if number >= 1 && number <= count {
        Some(number - 1)
    } else {
        None
    }
}

pub fn ask_text(question: &str, default: &str) -> Result<String> {
    println!("\n{question} [{default}]");
    Ok(parse_text(&read_line()?, default))
}

pub fn ask_u32(question: &str, default: u32) -> Result<u32> {
    loop {
        let answer = ask_text(question, &default.to_string())?;
        match answer.parse() {
            Ok(value) => return Ok(value),
            Err(_) => println!("To nie jest liczba. Spróbuj jeszcze raz."),
        }
    }
}

pub fn ask_bool(question: &str, default: bool) -> Result<bool> {
    let hint = if default { "T/n" } else { "t/N" };
    loop {
        println!("\n{question} [{hint}]");
        match parse_bool(&read_line()?, default) {
            Some(value) => return Ok(value),
            None => println!("Odpowiedz „tak” albo „nie”."),
        }
    }
}

pub fn ask_choice(question: &str, options: &[String], default: usize) -> Result<usize> {
    loop {
        println!("\n{question}");
        for (i, option) in options.iter().enumerate() {
            let mark = if i == default { "*" } else { " " };
            println!(" {mark} {}) {option}", i + 1);
        }
        println!("Enter = {}", default + 1);
        match parse_choice(&read_line()?, options.len(), default) {
            Some(choice) => return Ok(choice),
            None => println!("Podaj numer z listy."),
        }
    }
}

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
