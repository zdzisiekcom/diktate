//! Filtr halucynacji whispera: odrzuca tekst, który nie jest transkrypcją,
//! tylko wytworem modelu na ciszy albo szumie.
//!
//! ZASADA: whisper wytrenowany na napisach z internetu, zapytany o ciszę lub
//! sam szum, potrafi „usłyszeć" formułki typowe dla materiału treningowego —
//! znaczniki dźwiękowe w nawiasach, podziękowania z końca filmu, wezwania do
//! subskrypcji — albo wyprodukować gołą interpunkcję. Filtr rozpoznaje te
//! przypadki po kształcie tekstu, nie po treści dźwięku, więc jest tani
//! i deterministyczny, ale z zasady niedoskonały. Dlatego każda reguła musi
//! błądzić w stronę zachowania tekstu: fałszywe odrzucenie prawdziwej
//! wypowiedzi jest gorsze niż nieziłapana halucynacja (patrz
//! `docs/07-zachowanie-dyktowania.md`, sekcja „Filtry: co nie trafia do
//! dokumentu" — odrzucenie ma być widoczne, a nie ciche zjedzenie tekstu).

/// Znane formułki z materiałów, na których trenowano whisper — pojawiają się
/// na ciszy albo szumie zamiast prawdziwej transkrypcji. Dopasowanie jest
/// bez rozróżniania wielkości liter i na całość tekstu (po obcięciu białych
/// znaków i interpunkcji na brzegach), nie na fragment zdania — dopisuj tu
/// nowe obserwowane przypadki.
const KNOWN_HALLUCINATIONS: &[&str] = &[
    "napisy stworzone przez społeczność amara.org",
    "napisy stworzone przez społeczność",
    "napisy stworzyła społeczność amara.org",
    "dziękujemy za uwagę",
    "dziękuję za uwagę",
    "dziękuję za obejrzenie",
    "dziękujemy za obejrzenie",
    "dzięki za obejrzenie",
    "subskrybuj",
    "subscribe",
    "please subscribe",
    "like and subscribe",
    "zapraszam do subskrypcji",
    "subskrybuj kanał",
    "thanks for watching",
    "thank you for watching",
    "napisy pl",
];

/// Czy `text` wygląda na halucynację whispera, a nie prawdziwą wypowiedź.
///
/// Reguły (dowolna wystarcza do odrzucenia):
/// 1. tekst po obcięciu białych znaków jest pusty;
/// 2. tekst w całości jest znacznikiem w nawiasach kwadratowych lub okrągłych
///    (np. `[muzyka]`, `(cisza)`) — nawias w środku zdania nie liczy się;
/// 3. tekst nie zawiera ani jednego znaku alfanumerycznego (sama interpunkcja
///    albo wielokropek);
/// 4. tekst pasuje (bez rozróżniania wielkości liter) do znanej formułki
///    z [`KNOWN_HALLUCINATIONS`].
pub fn is_hallucination(text: &str) -> bool {
    let trimmed = text.trim();

    if trimmed.is_empty() {
        return true;
    }
    if is_bracket_marker(trimmed) {
        return true;
    }
    if trimmed.chars().all(|c| !c.is_alphanumeric()) {
        return true;
    }
    if matches_known_hallucination(trimmed) {
        return true;
    }
    false
}

/// Tekst w całości opakowany w `[...]` albo `(...)` — typowy znacznik
/// dźwiękowy whispera, nie fragment zdania. Nawias, który tylko otwiera albo
/// tylko zamyka tekst (a nie obejmuje go całego), nie jest tu łapany celowo —
/// to normalna interpunkcja w środku prawdziwego zdania.
fn is_bracket_marker(text: &str) -> bool {
    (text.starts_with('[') && text.ends_with(']')) || (text.starts_with('(') && text.ends_with(')'))
}

/// Porównuje `text` z listą znanych formułek — po obcięciu interpunkcji
/// i białych znaków na brzegach oraz sprowadzeniu do małych liter.
fn matches_known_hallucination(text: &str) -> bool {
    let normalized = normalize_for_match(text);
    KNOWN_HALLUCINATIONS
        .iter()
        .any(|&phrase| normalized == phrase)
}

fn normalize_for_match(text: &str) -> String {
    text.trim_matches(|c: char| c.is_ascii_punctuation() || c == '…')
        .trim()
        .to_lowercase()
}

/// Usuwa końcowy wielokropek (`...` albo `…`) — whisper kończy nim wypowiedź
/// uciętą w pół zdania (np. pauzą na oddech), zgodnie z normalizacją opisaną
/// w `docs/07-zachowanie-dyktowania.md` (`strip_trailing_ellipsis`).
fn strip_trailing_ellipsis(text: &str) -> &str {
    let text = text.trim_end();
    if let Some(stripped) = text.strip_suffix('…') {
        return stripped.trim_end();
    }
    if let Some(stripped) = text.strip_suffix("...") {
        return stripped.trim_end();
    }
    text
}

/// Czyści surowy tekst segmentu przed wstawieniem do dokumentu.
///
/// Zwraca `None`, gdy [`is_hallucination`] uzna tekst za odrzucony. W innym
/// wypadku zwraca tekst obcięty z białych znaków na brzegach i z końcowego
/// wielokropka.
pub fn clean(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if is_hallucination(trimmed) {
        return None;
    }

    let result = strip_trailing_ellipsis(trimmed).trim();
    if result.is_empty() {
        // Teoretycznie nieosiągalne (tekst złożony wyłącznie z wielokropka
        // łapie już reguła interpunkcji w `is_hallucination`), ale zostaje
        // jako siatka bezpieczeństwa.
        return None;
    }
    Some(result.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- reguły odrzucania, każda osobno ---

    #[test]
    fn odrzuca_pusty_tekst() {
        assert!(is_hallucination(""));
        assert!(is_hallucination("   "));
        assert_eq!(clean(""), None);
        assert_eq!(clean("   "), None);
    }

    #[test]
    fn odrzuca_znacznik_w_nawiasach_kwadratowych() {
        assert!(is_hallucination("[muzyka]"));
        assert!(is_hallucination("[Muzyka]"));
        assert!(is_hallucination("[cisza]"));
        assert_eq!(clean("[muzyka]"), None);
    }

    #[test]
    fn odrzuca_znacznik_w_nawiasach_okraglych() {
        assert!(is_hallucination("(muzyka)"));
        assert!(is_hallucination("(cisza)"));
        assert_eq!(clean("(muzyka)"), None);
    }

    #[test]
    fn odrzuca_sama_interpunkcje() {
        assert!(is_hallucination("..."));
        assert!(is_hallucination("…"));
        assert!(is_hallucination("?!"));
        assert!(is_hallucination("- - -"));
        assert_eq!(clean("..."), None);
    }

    #[test]
    fn odrzuca_znane_formulki_bez_wzgledu_na_wielkosc_liter() {
        assert!(is_hallucination(
            "Napisy stworzone przez społeczność Amara.org"
        ));
        assert!(is_hallucination("DZIĘKUJEMY ZA UWAGĘ"));
        assert!(is_hallucination("Subscribe"));
        assert!(is_hallucination("thanks for watching"));
        assert_eq!(clean("Dziękujemy za uwagę."), None);
        assert_eq!(clean("Thanks for watching!"), None);
    }

    // --- clean: przycinanie i wielokropek ---

    #[test]
    fn clean_przycina_biale_znaki_i_koncowy_wielokropek() {
        assert_eq!(
            clean("  cześć, jak się masz...  "),
            Some("cześć, jak się masz".to_string())
        );
        assert_eq!(
            clean("poczekaj chwilę…"),
            Some("poczekaj chwilę".to_string())
        );
    }

    #[test]
    fn clean_nie_rusza_wielokropka_w_srodku_tekstu() {
        assert_eq!(
            clean("czekaj... nie, jednak tak"),
            Some("czekaj... nie, jednak tak".to_string())
        );
    }

    // --- przypadki, które NIE mogą zostać odrzucone ---

    #[test]
    fn nie_odrzuca_zwyklego_zdania_z_diakrytykami() {
        let zdanie = "Zażółć gęślą jaźń, proszę bardzo.";
        assert!(!is_hallucination(zdanie));
        assert_eq!(clean(zdanie), Some(zdanie.to_string()));
    }

    #[test]
    fn nie_odrzuca_zdania_z_nawiasem_w_srodku() {
        let zdanie = "to jest test (chyba) działa";
        assert!(!is_hallucination(zdanie));
        assert_eq!(clean(zdanie), Some(zdanie.to_string()));
    }

    #[test]
    fn nie_odrzuca_zdania_konczacego_sie_wykrzyknikiem() {
        let zdanie = "To jest naprawdę świetne!";
        assert!(!is_hallucination(zdanie));
        assert_eq!(clean(zdanie), Some(zdanie.to_string()));
    }

    #[test]
    fn nie_odrzuca_jednowyrazowej_odpowiedzi() {
        assert!(!is_hallucination("tak"));
        assert_eq!(clean("tak"), Some("tak".to_string()));
    }
}
