//! Prompt whispera: szablon użytkownika, ogon dokumentu i strażnik echa.
//!
//! Zasada jest jedna — model dostaje dokładnie to, co użytkownik widzi
//! w polu szablonu, z `{kontekst}` podmienionym na tekst sprzed kursora.
//! Nic się nie dokleja po cichu. Sam prompt bywa jednak niebezpieczny:
//! whisper potrafi zamiast transkrypcji powtórzyć własną podpowiedź, więc
//! tu mieszka też wykrywanie echa (`docs/01#kontekst-między-segmentami`).

use std::collections::HashSet;
use std::sync::Mutex;

/// Miejsce w szablonie, w które wchodzi ogon dokumentu. Brak symbolu jest
/// poprawnym stanem: znaczy „prompt stały, bez kontekstu".
pub const CONTEXT_PLACEHOLDER: &str = "{kontekst}";

/// Skleja prompt dla whispera. Pusty szablon daje pusty prompt — daemon
/// zostaje wtedy przy `whisper.initial_prompt` z własnej konfiguracji.
pub fn render(template: &str, context: &str) -> String {
    if template.trim().is_empty() {
        return String::new();
    }
    template
        .replace(CONTEXT_PLACEHOLDER, context)
        .trim()
        .to_string()
}

/// Ostatnie `chars` znaków tekstu, docięte do granicy słowa od lewej.
///
/// Urwana sylaba na starcie promptu jest gorsza niż kilka znaków mniej:
/// model traktuje ją jako słowo i potrafi je powtórzyć w transkrypcji.
pub fn tail(text: &str, chars: usize) -> &str {
    if chars == 0 {
        return "";
    }
    let total = text.chars().count();
    if total <= chars {
        return text;
    }
    let start = text
        .char_indices()
        .nth(total - chars)
        .map(|(i, _)| i)
        .unwrap_or(0);
    let cut = &text[start..];
    // Cięcie wypadło w środku słowa, jeśli znak tuż przed nim nie jest
    // odstępem — wtedy porzucamy ten ogarek.
    let in_word = text[..start]
        .chars()
        .next_back()
        .is_some_and(|c| !c.is_whitespace());
    if in_word {
        match cut.find(char::is_whitespace) {
            Some(space) => cut[space..].trim_start(),
            None => "",
        }
    } else {
        cut.trim_start()
    }
}

/// Awaryjny limit długości promptu po stronie daemona. Obcina **początek**:
/// bliżej kursora znaczy ważniejsze, a whisper i tak mieści w prompcie
/// tylko ~224 tokeny.
pub fn clamp(prompt: &str, max_chars: usize) -> &str {
    let total = prompt.chars().count();
    if total <= max_chars {
        return prompt;
    }
    let start = prompt
        .char_indices()
        .nth(total - max_chars)
        .map(|(i, _)| i)
        .unwrap_or(0);
    &prompt[start..]
}

/// Czy segment jest powtórzeniem promptu. Podobieństwo Jaccarda na
/// trigramach słów: pojedyncze wspólne słowa nie wystarczą, dopiero wspólne
/// **sekwencje** znaczą, że model przepisał podpowiedź zamiast słuchać.
pub fn looks_like_echo(segment: &str, prompt: &str, threshold: f32) -> bool {
    let left = trigrams(segment);
    let right = trigrams(prompt);
    if left.is_empty() || right.is_empty() {
        return false;
    }
    let common = left.intersection(&right).count() as f32;
    let all = left.union(&right).count() as f32;
    common / all >= threshold
}

/// Czy tekst zapętlił się na jednej frazie. Whisper w pętli powtarza ten sam
/// kawałek kilkanaście razy — cztery wystąpienia tego samego trigramu to już
/// nie jest naturalna mowa.
pub fn is_looping(text: &str) -> bool {
    let words = normalized_words(text);
    if words.len() < 6 {
        return false;
    }
    let mut counts = std::collections::HashMap::new();
    for window in words.windows(3) {
        *counts.entry(window.join(" ")).or_insert(0usize) += 1;
    }
    counts.values().any(|count| *count >= 4)
}

fn trigrams(text: &str) -> HashSet<String> {
    let words = normalized_words(text);
    words.windows(3).map(|window| window.join(" ")).collect()
}

fn normalized_words(text: &str) -> Vec<String> {
    text.split_whitespace()
        .map(|word| {
            word.chars()
                .filter(|c| c.is_alphanumeric())
                .flat_map(char::to_lowercase)
                .collect::<String>()
        })
        .filter(|word| !word.is_empty())
        .collect()
}

/// Prompt dzielony między pętlę IPC (pisze) a wątek transkrypcji (czyta).
///
/// Slot należy do **połączenia**, nie do sesji: kontekst przysłany przed
/// pierwszym `session.start` ma obowiązywać od pierwszej wypowiedzi.
#[derive(Debug, Default)]
pub struct PromptSlot(Mutex<String>);

impl PromptSlot {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set(&self, text: &str) {
        match self.0.lock() {
            Ok(mut slot) => *slot = text.to_string(),
            // Zatruty zamek nie może wywrócić transkrypcji — prompt jest
            // podpowiedzią, a nie warunkiem działania.
            Err(err) => eprintln!("prompt: slot zatruty przez panikę: {err}"),
        }
    }

    pub fn get(&self) -> String {
        self.0.lock().map(|slot| slot.clone()).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn szablon_dostaje_kontekst_w_miejscu_symbolu() {
        let out = render("Terminy: ggml.\n{kontekst}", "Zbudowałem wiązania.");
        assert_eq!(out, "Terminy: ggml.\nZbudowałem wiązania.");
    }

    #[test]
    fn szablon_bez_symbolu_zostaje_sam_soba() {
        assert_eq!(render("Terminy: ggml.", "cokolwiek"), "Terminy: ggml.");
    }

    #[test]
    fn pusty_szablon_daje_pusty_prompt() {
        assert_eq!(render("", "kontekst"), "");
    }

    #[test]
    fn pusty_kontekst_nie_zostawia_pustej_linii() {
        assert_eq!(render("Terminy: ggml.\n{kontekst}", ""), "Terminy: ggml.");
    }

    #[test]
    fn ogon_liczy_znaki_a_nie_bajty() {
        // Same dwuznakowe w UTF-8 litery: cięcie po bajtach by tu spanikowało.
        let text = "ąęśćżźół ąęśćżźół";
        assert_eq!(tail(text, 8), "ąęśćżźół");
    }

    #[test]
    fn ogon_nie_zaczyna_sie_od_urwanego_slowa() {
        let text = "wczoraj zbudowałem wiązania rustowe";
        // Surowe 12 znaków dałoby "ania rustowe" — urwane słowo.
        assert_eq!(tail(text, 12), "rustowe");
    }

    #[test]
    fn ogon_krotszy_niz_limit_wraca_w_calosci() {
        assert_eq!(tail("krótko", 200), "krótko");
    }

    #[test]
    fn ogon_zerowy_jest_pusty() {
        assert_eq!(tail("cokolwiek", 0), "");
    }

    #[test]
    fn clamp_obcina_poczatek_a_zostawia_koniec() {
        assert_eq!(clamp("abcdefgh", 3), "fgh");
    }

    #[test]
    fn clamp_nie_rusza_krotkiego_promptu() {
        assert_eq!(clamp("abc", 900), "abc");
    }

    #[test]
    fn segment_powtarzajacy_prompt_jest_echem() {
        let prompt = "Transkrypcja notatek technicznych. Terminy: Kubernetes, Wayland, systemd.";
        let segment = "Transkrypcja notatek technicznych. Terminy: Kubernetes, Wayland, systemd.";
        assert!(looks_like_echo(segment, prompt, 0.8));
    }

    // Test najważniejszy w całym module: strażnik, który zjada prawdziwą
    // mowę, jest gorszy od whispera powtarzającego prompt — użytkownik
    // traci wtedy zdanie, którego nikt mu nie odda.
    #[test]
    fn zdanie_powtarzajace_kilka_slow_z_kontekstu_nie_jest_echem() {
        let prompt = "Wczoraj zbudowałem wiązania rustowe do whisper.cpp i zmierzyłem czasy.";
        let segment = "Wiązania rustowe okazały się szybsze, niż się spodziewałem.";
        assert!(!looks_like_echo(segment, prompt, 0.8));
    }

    #[test]
    fn krotki_segment_nigdy_nie_jest_echem() {
        // Dwa słowa nie dają ani jednego trigramu — nie ma czego porównywać,
        // a zgadywanie kosztowałoby prawdziwą wypowiedź.
        assert!(!looks_like_echo("tak jest", "tak jest zawsze i wszędzie", 0.8));
    }

    #[test]
    fn segment_bez_promptu_nie_jest_echem() {
        assert!(!looks_like_echo("cokolwiek tu padło dzisiaj", "", 0.8));
    }

    #[test]
    fn powtarzana_fraza_to_petla() {
        assert!(is_looping(
            "dziękuję za uwagę dziękuję za uwagę dziękuję za uwagę dziękuję za uwagę"
        ));
    }

    #[test]
    fn zwykle_zdanie_nie_jest_petla() {
        assert!(!is_looping(
            "Wczoraj zbudowałem wiązania rustowe i zmierzyłem czasy przebiegów."
        ));
    }

    #[test]
    fn trzy_powtorzenia_tuz_pod_progiem_to_jeszcze_nie_petla() {
        // Próg to cztery wystąpienia tego samego trigramu słów. Trzy — jedno
        // powtórzenie poniżej granicy — to zdarza się w naturalnej mowie
        // (np. potwierdzenie powtórzone dla pewności) i strażnik ma to
        // przepuścić, a nie potraktować jak zapętlony whisper.
        assert!(!is_looping(
            "dziękuję za uwagę dziękuję za uwagę dziękuję za uwagę"
        ));
    }

    #[test]
    fn slot_oddaje_to_co_przyjal() {
        let slot = PromptSlot::new();
        assert_eq!(slot.get(), "");
        slot.set("prompt");
        assert_eq!(slot.get(), "prompt");
        slot.set("");
        assert_eq!(slot.get(), "");
    }
}
