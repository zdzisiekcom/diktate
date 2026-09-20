//! Korekta całej notatki lokalnym modelem językowym.
//!
//! Zasada jest jedna i pochodzi wprost z `docs/11-korekta-llm.md`: **narzędzie
//! zjadające prawdziwy tekst jest gorsze od przepuszczonego błędu**. Model
//! poproszony o poprawienie potrafi zamiast tego zredagować — na akapicie
//! zmienia jedną trzecią słów i oddaje tekst, który czyta się lepiej, ale nie
//! jest już wypowiedzią użytkownika. Zdarza mu się też zmyślić słowo tam,
//! gdzie nie wiedział, co usłyszał, i zrobić to bez żadnego sygnału.
//!
//! Dlatego między odpowiedzią serwera a dokumentem stoi [`review`]:
//!
//! 1. **Walidacja znakowa** — odpowiedź ze znakami spoza alfabetu polskiego,
//!    których nie było w wejściu, jest odrzucana. Bielik tego nie potrzebuje,
//!    ale po podmianie modelu na serwerze zabezpieczenie zadziała bez zmiany
//!    kodu — dokładnie ten błąd popełniał testowany `ornith`.
//! 2. **Odsetek zmienionych słów** — liczony na słowach, z progiem zależnym
//!    od długości notatki. Przekroczenie nie odrzuca korekty po cichu, tylko
//!    oddaje decyzję użytkownikowi.
//!
//! Sama korekta wchodzi do dokumentu jako **jedna operacja logu**, więc
//! `Ctrl+Z` cofa ją w całości (`docs/03-model-dokumentu.md`). Uruchamia się
//! wyłącznie ręcznie, nigdy sama w trakcie dyktowania.
//!
//! Warstwa sieciowa ([`request_body`], [`ask`]) jest oddzielona od warstwy
//! decyzji, bo tej drugiej nie wolno testować przez sieć: wyniki modelu
//! migoczą mimo `temperature: 0`, więc złote odpowiedzi w `cargo test` by
//! migotały razem z nimi.

use anyhow::{anyhow, bail, Context, Result};
use diktate_config::CorrectionConfig;
use serde_json::{json, Value};

/// Co zrobić z odpowiedzią modelu.
#[derive(Debug, Clone, PartialEq)]
pub enum Verdict {
    /// Tekst da się przyjąć bez pytania. Odsetek zmienionych słów w środku.
    Accept { ratio: f32 },
    /// Zmian jest więcej, niż przewiduje próg — decyduje użytkownik.
    Confirm { ratio: f32, limit: f32 },
    /// Odpowiedź odrzucona; powód nadaje się wprost do pokazania.
    Reject { reason: String },
}

/// Ocenia odpowiedź modelu względem tekstu, który poszedł do korekty.
pub fn review(config: &CorrectionConfig, before: &str, after: &str) -> Verdict {
    if after.trim().is_empty() {
        return Verdict::Reject {
            reason: "model oddał pustą odpowiedź".to_string(),
        };
    }
    if let Some(intruz) = foreign_char(before, after, &config.allowed_extra_chars) {
        return Verdict::Reject {
            reason: format!("model wstawił znak spoza alfabetu: „{intruz}”"),
        };
    }
    if after.trim() == before.trim() {
        return Verdict::Reject {
            reason: "model nie znalazł nic do poprawienia".to_string(),
        };
    }

    let changed = changed_words(before, after);
    let ratio = change_ratio(before, after);
    let limit = config.max_change_ratio(words(before).len());
    // Sam odsetek nie wystarcza przy krótkiej notatce: w zdaniu z sześciu
    // słów jedna uczciwa naprawa to już 17%, a to nie jest redakcja. Dopiero
    // odsetek **i** liczba zmienionych słów naraz znaczą, że model przestał
    // poprawiać.
    if ratio > limit && changed > config.min_changed_words {
        Verdict::Confirm { ratio, limit }
    } else {
        Verdict::Accept { ratio }
    }
}

/// Ocenia odpowiedź modelu na **polecenie redakcyjne**.
///
/// Od [`review`] różni się jedną rzeczą, ale zasadniczą: progi odsetka zmian
/// tu nie obowiązują. Przy korekcie przepisany akapit jest alarmem — model
/// miał poprawiać, a zaczął redagować. Przy redakcji przepisany akapit jest
/// **wykonanym poleceniem**: „zmień na bardziej formalny" ma zmienić dużo,
/// więc pytanie o zgodę przy 41% zmian byłoby czystym szumem. Zostaje reszta
/// zabezpieczeń — walidacja znakowa, pusta odpowiedź i brak zmian — a przed
/// utratą tekstu broni jedna operacja logu, którą `Ctrl+Z` cofa w całości.
pub fn review_redaction(config: &CorrectionConfig, before: &str, after: &str) -> Verdict {
    if after.trim().is_empty() {
        return Verdict::Reject {
            reason: "model oddał pustą odpowiedź".to_string(),
        };
    }
    if let Some(intruz) = foreign_char(before, after, &config.allowed_extra_chars) {
        return Verdict::Reject {
            reason: format!("model wstawił znak spoza alfabetu: „{intruz}”"),
        };
    }
    if after.trim() == before.trim() {
        return Verdict::Reject {
            reason: "model nie zmienił w tekście niczego".to_string(),
        };
    }
    Verdict::Accept {
        ratio: change_ratio(before, after),
    }
}

/// Odsetek zmienionych słów: odległość edycyjna liczona na słowach, podzielona
/// przez długość dłuższego z tekstów.
///
/// Na słowach, nie na znakach, bo to słowo jest jednostką, w której model
/// przepisuje wypowiedź — a przesunięcie jednej litery w długim wyrazie nie
/// znaczy tyle samo, co wymiana całego zwrotu. Ta sama miara stoi za liczbami
/// z `docs/11` („zmienia 33–37% tekstu”).
pub fn change_ratio(before: &str, after: &str) -> f32 {
    let longer = words(before).len().max(words(after).len());
    if longer == 0 {
        return 0.0;
    }
    changed_words(before, after) as f32 / longer as f32
}

/// Liczba słów, które model naprawdę wymienił.
///
/// Wielkość liter i interpunkcja na brzegach słowa są tu celowo niewidoczne:
/// wstawianie kropek i wielkich liter to **zadanie** korekty, a nie jej
/// skutek uboczny. Miara, która je liczy, alarmuje przy każdym poprawnie
/// wykonanym zadaniu i zaraz przestaje cokolwiek znaczyć.
fn changed_words(before: &str, after: &str) -> usize {
    let a: Vec<String> = words(before).iter().map(|w| normalize(w)).collect();
    let b: Vec<String> = words(after).iter().map(|w| normalize(w)).collect();
    let a: Vec<&str> = a.iter().map(String::as_str).collect();
    let b: Vec<&str> = b.iter().map(String::as_str).collect();
    levenshtein(&a, &b)
}

/// Słowo sprowadzone do porównywalnej postaci: małe litery, bez interpunkcji
/// na brzegach. Interpunkcja w środku (`small-q5_1`, `0,6`) zostaje — tam
/// jest częścią słowa, a nie ozdobą zdania.
fn normalize(word: &str) -> String {
    word.trim_matches(|c: char| !c.is_alphanumeric())
        .to_lowercase()
}

/// Odsetek zmian jako liczba całkowita procent — do komunikatów.
pub fn percent(ratio: f32) -> u32 {
    (ratio * 100.0).round() as u32
}

/// Pierwszy znak odpowiedzi, którego nie wolno przepuścić: nie należy do
/// alfabetu polskiego, cyfr ani interpunkcji, nie ma go w tekście wejściowym
/// i nie jest wypisany w `allowed_extra_chars`.
///
/// Wejście jest tu równoprawnym źródłem: jeśli użytkownik sam wpisał do
/// notatki strzałkę albo literę grecką, korekta nie może się o nią wywrócić.
fn foreign_char(before: &str, after: &str, extra: &str) -> Option<char> {
    after
        .chars()
        .find(|&c| !allowed_char(c) && !extra.contains(c) && !before.contains(c))
}

fn allowed_char(c: char) -> bool {
    if c.is_ascii_alphanumeric() || c.is_whitespace() {
        return true;
    }
    if POLISH_LETTERS.contains(c) {
        return true;
    }
    c.is_ascii_punctuation() || PUNCTUATION.contains(c)
}

/// Litery polskie spoza ASCII — obie wielkości.
const POLISH_LETTERS: &str = "ąćęłńóśźżĄĆĘŁŃÓŚŹŻ";

/// Interpunkcja typograficzna, której model używa poprawnie: półpauza,
/// myślnik, cudzysłowy drukarskie, wielokropek, niełamliwa spacja.
const PUNCTUATION: &str = "–—―…„”“»«‚’‘\u{00a0}";

fn words(text: &str) -> Vec<&str> {
    text.split_whitespace().collect()
}

/// Odległość edycyjna między ciągami słów. Dwa wiersze zamiast pełnej
/// macierzy: notatka bywa długa, a pamięć nie jest tu do niczego potrzebna.
fn levenshtein(a: &[&str], b: &[&str]) -> usize {
    if a.is_empty() {
        return b.len();
    }
    let mut previous: Vec<usize> = (0..=b.len()).collect();
    let mut current = vec![0usize; b.len() + 1];
    for (i, słowo_a) in a.iter().enumerate() {
        current[0] = i + 1;
        for (j, słowo_b) in b.iter().enumerate() {
            let koszt = usize::from(słowo_a != słowo_b);
            current[j + 1] = (previous[j] + koszt)
                .min(previous[j + 1] + 1)
                .min(current[j] + 1);
        }
        std::mem::swap(&mut previous, &mut current);
    }
    previous[b.len()]
}

// --- warstwa sieciowa ---

/// Treść żądania do API zgodnego z OpenAI.
///
/// `disable_thinking` wysyła **oba** równoważne wyciszenia rozumowania:
/// serwery rozumieją jedno albo drugie, a model z domyślnie włączonym
/// thinkingiem odpowiada bez tego 5–45× wolniej i potrafi wyczerpać
/// `max_tokens` na samym rozumowaniu, oddając pusty `content` (`docs/11`).
pub fn request_body(config: &CorrectionConfig, text: &str) -> Value {
    let user = config.user_template.replace("{text}", text);
    finish_body(
        config,
        json!({
            "model": config.model,
            "temperature": config.temperature,
            "max_tokens": config.max_tokens,
            "messages": [
                {"role": "system", "content": config.system_prompt},
                {"role": "user", "content": user},
            ],
        }),
    )
}

/// Domyka treść żądania wyciszeniem rozumowania — wspólne dla korekty
/// i redakcji, bo obie rozmawiają z tym samym serwerem.
fn finish_body(config: &CorrectionConfig, mut body: Value) -> Value {
    if config.disable_thinking {
        body["chat_template_kwargs"] = json!({"enable_thinking": false});
        body["reasoning_effort"] = json!("none");
    }
    body
}

/// Treść żądania redakcji: polecenie użytkownika i tekst celu wchodzą
/// osobnymi polami szablonu, a instrukcją systemową jest prompt redaktora.
///
/// Polecenie nie jest doklejane do promptu systemowego celowo: to **dane od
/// użytkownika**, a nie część instrukcji, i tak też ma je widzieć model.
pub fn redaction_body(config: &CorrectionConfig, instruction: &str, text: &str) -> Value {
    let user = config
        .redaction_user_template
        .replace("{instruction}", instruction)
        .replace("{text}", text);
    finish_body(
        config,
        json!({
            "model": config.model,
            "temperature": config.temperature,
            "max_tokens": config.max_tokens,
            "messages": [
                {"role": "system", "content": config.redaction_system_prompt},
                {"role": "user", "content": user},
            ],
        }),
    )
}

/// Wyjmuje poprawiony tekst z odpowiedzi serwera.
pub fn extract_text(response: &Value) -> Result<String> {
    if let Some(error) = response.get("error") {
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("serwer zgłosił błąd bez opisu");
        bail!("{message}");
    }
    let content = response
        .pointer("/choices/0/message/content")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("odpowiedź bez pola choices[0].message.content"))?;
    Ok(strip_wrapping(content))
}

/// Zdejmuje opakowanie, którym model bywa otacza odpowiedź: płot z grawisów
/// albo cudzysłów wokół całości. Sam tekst zostaje nietknięty — nie ma tu
/// żadnej redakcji, bo każda zmiana liczyłaby się potem jako „poprawka
/// modelu”, której model nie zrobił.
fn strip_wrapping(content: &str) -> String {
    let mut text = content.trim();
    if text.starts_with("```") {
        text = text.trim_start_matches("```");
        // Po płocie bywa nazwa języka — zdejmujemy resztę tej linii.
        if let Some(koniec_linii) = text.find('\n') {
            let pierwsza = &text[..koniec_linii];
            if !pierwsza.trim().is_empty() && !pierwsza.contains(' ') {
                text = &text[koniec_linii + 1..];
            }
        }
        text = text.trim_end_matches("```").trim();
    }
    if text.len() >= 2 {
        let znaki: Vec<char> = text.chars().collect();
        let (pierwszy, ostatni) = (znaki[0], znaki[znaki.len() - 1]);
        if matches!((pierwszy, ostatni), ('"', '"') | ('„', '”') | ('«', '»')) {
            text = text[pierwszy.len_utf8()..text.len() - ostatni.len_utf8()].trim();
        }
    }
    text.to_string()
}

/// Wysyła tekst do modelu i zwraca poprawioną wersję. **Blokuje wątek** —
/// wołane wyłącznie z wątku roboczego, nigdy z pętli GTK.
pub fn ask(config: &CorrectionConfig, text: &str) -> Result<String> {
    send(config, request_body(config, text))
}

/// Wysyła polecenie redakcyjne wraz z tekstem celu. **Blokuje wątek** — tak
/// samo jak [`ask`].
pub fn ask_redaction(config: &CorrectionConfig, instruction: &str, text: &str) -> Result<String> {
    send(config, redaction_body(config, instruction, text))
}

/// Jedna droga do serwera dla korekty i redakcji: różnią się wyłącznie
/// treścią żądania.
fn send(config: &CorrectionConfig, body: Value) -> Result<String> {
    let timeout = std::time::Duration::from_millis(config.timeout_ms);
    let agent = ureq::AgentBuilder::new().timeout(timeout).build();

    let mut request = agent
        .post(&config.endpoint)
        .set("Content-Type", "application/json");
    if !config.api_key.trim().is_empty() {
        request = request.set(
            "Authorization",
            &format!("Bearer {}", config.api_key.trim()),
        );
    }

    let response = match request.send_json(body) {
        Ok(response) => response,
        // Odpowiedź z kodem błędu niesie zwykle opis w treści — jest bardziej
        // użyteczny niż samo „400 Bad Request”.
        Err(ureq::Error::Status(code, response)) => {
            let treść = response.into_string().unwrap_or_default();
            let opis = serde_json::from_str::<Value>(&treść)
                .ok()
                .and_then(|json| {
                    json.pointer("/error/message")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
                .unwrap_or(treść);
            bail!("serwer odpowiedział {code}: {opis}");
        }
        Err(err) => {
            return Err(err)
                .with_context(|| format!("nie udało się połączyć z {}", config.endpoint))
        }
    };

    let json: Value = response
        .into_json()
        .context("odpowiedź serwera nie jest poprawnym JSON-em")?;
    extract_text(&json)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> CorrectionConfig {
        CorrectionConfig::default()
    }

    // --- co odrzucić być NIE MOŻE ---
    //
    // Tak jak przy filtrach halucynacji (`CLAUDE.md`), ważniejsze od testów
    // odrzucania są testy tego, co przejść musi.

    #[test]
    fn zwykla_poprawka_przechodzi() {
        let before = "ten wis per przetwarza całe okno";
        let after = "Ten Whisper przetwarza całe okno.";
        assert!(
            matches!(review(&config(), before, after), Verdict::Accept { .. }),
            "naprawa rozbitego słowa to dokładnie to, po co jest korekta"
        );
    }

    #[test]
    fn polskie_znaki_i_typografia_nie_sa_intruzem() {
        let before = "zażółć gęślą jaźń i powiedz ze to dziala";
        let after = "Zażółć gęślą jaźń — i powiedz, że to działa…";
        assert_eq!(foreign_char(before, after, ""), None);
    }

    #[test]
    fn nazwy_techniczne_i_liczby_przechodza_nietkniete() {
        let before = "model small-q5_1 ma 181 MB i wczytuje się 0,6 s";
        let after = "Model small-q5_1 ma 181 MB i wczytuje się 0,6 s.";
        assert!(matches!(
            review(&config(), before, after),
            Verdict::Accept { .. }
        ));
    }

    #[test]
    fn znak_ktory_byl_w_notatce_wraca_bez_przeszkody() {
        // Użytkownik sam wpisał strzałkę; korekta nie ma prawa się o nią
        // wywrócić tylko dlatego, że nie jest literą polską.
        let before = "segment → transkrypcja to caly przeplyw";
        let after = "Segment → transkrypcja to cały przepływ.";
        assert_eq!(foreign_char(before, after, ""), None);
    }

    // --- co odrzucić trzeba ---

    #[test]
    fn znak_spoza_alfabetu_jest_odrzucany() {
        // Błąd testowanego `ornith`: „trzydzie三十” w środku słowa.
        let verdict = review(&config(), "trzydzieści złotych", "trzydzie三十 złotych");
        match verdict {
            Verdict::Reject { reason } => assert!(reason.contains("三")),
            inne => panic!("znak spoza alfabetu musi być odrzucony, a jest {inne:?}"),
        }
    }

    #[test]
    fn pusta_odpowiedz_jest_odrzucana() {
        // Model z thinkingiem potrafi wyczerpać `max_tokens` na rozumowaniu
        // i oddać pusty `content` — to nie jest poprawiony tekst.
        assert!(matches!(
            review(&config(), "cokolwiek", "   "),
            Verdict::Reject { .. }
        ));
    }

    #[test]
    fn brak_zmian_nie_rusza_dokumentu() {
        let tekst = "Zdanie było już poprawne.";
        assert!(matches!(
            review(&config(), tekst, tekst),
            Verdict::Reject { .. }
        ));
    }

    #[test]
    fn redakcja_akapitu_pyta_zamiast_wchodzic_sama() {
        // Granica z `docs/11`: na dłuższym tekście model przestaje korygować,
        // a zaczyna redagować. Nie odrzucamy tego po cichu — pytamy.
        let before = "potem leci transkrypcja i jak segment się domknie to \
                      tekst idzie na ekran a użytkownik go widzi od razu";
        let after = "Następnie następuje transkrypcja. Gdy segment się domknie, \
                     tekst trafia na ekran, dzięki czemu użytkownik widzi go \
                     natychmiast.";
        let mut cfg = config();
        cfg.short_text_words = 5;
        match review(&cfg, before, after) {
            Verdict::Confirm { ratio, limit } => {
                assert!(ratio > limit, "{ratio} miało przekroczyć {limit}")
            }
            inne => panic!("przepisany akapit ma trafić pod decyzję użytkownika, a jest {inne:?}"),
        }
    }

    // --- redakcja poleceniem (docs/11) ---

    #[test]
    fn zadanie_redakcji_niesie_polecenie_i_tekst_osobno() {
        let body = redaction_body(&config(), "zmień na formalny", "dobra, zaczynamy");
        let system = body["messages"][0]["content"].as_str().unwrap();
        let user = body["messages"][1]["content"].as_str().unwrap();
        assert_eq!(system, config().redaction_system_prompt);
        assert!(user.contains("zmień na formalny"), "a jest {user}");
        assert!(user.contains("dobra, zaczynamy"), "a jest {user}");
    }

    #[test]
    fn redakcja_nie_pyta_o_zgode_przy_przepisanym_akapicie() {
        // Przy korekcie przepisanie akapitu jest alarmem; przy redakcji jest
        // wykonanym poleceniem. Progi odsetka zmian tu nie obowiązują.
        let before = "dobra to zaczynamy bo mamy mało czasu a roboty dużo";
        let after = "Uprzejmie informuję, że rozpoczynamy prace niezwłocznie, \
                     ponieważ czas jest ograniczony, a zakres zadań szeroki.";
        let mut cfg = config();
        cfg.short_text_words = 3;
        match review_redaction(&cfg, before, after) {
            Verdict::Accept { ratio } => assert!(ratio > cfg.max_change_ratio_long),
            inne => panic!("wykonane polecenie ma wejść bez pytania, a jest {inne:?}"),
        }
    }

    #[test]
    fn redakcja_ze_znakiem_spoza_alfabetu_jest_odrzucana() {
        let verdict = review_redaction(&config(), "trzydzieści złotych", "trzydzie三十 złotych");
        match verdict {
            Verdict::Reject { reason } => assert!(reason.contains("三")),
            inne => panic!("znak spoza alfabetu musi być odrzucony, a jest {inne:?}"),
        }
    }

    #[test]
    fn pusta_odpowiedz_redakcji_jest_odrzucana() {
        assert!(matches!(
            review_redaction(&config(), "cokolwiek", "  "),
            Verdict::Reject { .. }
        ));
    }

    #[test]
    fn redakcja_bez_zmian_nie_rusza_dokumentu() {
        let tekst = "Tekst został bez zmian.";
        assert!(matches!(
            review_redaction(&config(), tekst, tekst),
            Verdict::Reject { .. }
        ));
    }

    // --- miara zmian ---

    #[test]
    fn identyczny_tekst_ma_zerowy_odsetek_zmian() {
        assert_eq!(change_ratio("ala ma kota", "ala ma kota"), 0.0);
    }

    #[test]
    fn jedno_slowo_na_trzy_to_jedna_trzecia() {
        let ratio = change_ratio("ala ma kota", "ala ma psa");
        assert!((ratio - 1.0 / 3.0).abs() < 1e-6, "a jest {ratio}");
    }

    #[test]
    fn wielkie_litery_i_interpunkcja_nie_sa_zmiana_slowa() {
        // To jest zadanie korekty, a nie jej skutek uboczny. Miara, która je
        // liczy, alarmuje przy każdym poprawnie wykonanym zadaniu.
        assert_eq!(change_ratio("ala ma kota i psa", "Ala ma kota i psa."), 0.0);
    }

    #[test]
    fn jedna_naprawa_w_krotkim_zdaniu_nie_wymaga_pytania() {
        // Sam odsetek by tu zaalarmował: dwa słowa z sześciu to 33%. To jest
        // jednak dokładnie ta naprawa, po którą sięga się po korektę.
        let verdict = review(
            &config(),
            "ten wis per przetwarza całe okno",
            "Ten Whisper przetwarza całe okno.",
        );
        assert!(
            matches!(verdict, Verdict::Accept { .. }),
            "a jest {verdict:?}"
        );
    }

    #[test]
    fn skrocenie_tekstu_liczy_sie_jako_zmiana() {
        // Model oddający mniej słów, niż dostał, to znak redakcji — miara
        // musi to widzieć, a nie porównywać tylko wspólny początek.
        let ratio = change_ratio("raz dwa trzy cztery", "raz dwa");
        assert!((ratio - 0.5).abs() < 1e-6, "a jest {ratio}");
    }

    #[test]
    fn odsetek_w_procentach_zaokragla_sie_normalnie() {
        assert_eq!(percent(0.336), 34);
        assert_eq!(percent(0.0), 0);
    }

    // --- żądanie i odpowiedź ---

    #[test]
    fn zadanie_niesie_prompt_z_konfiguracji_i_tekst_w_szablonie() {
        let mut cfg = config();
        cfg.system_prompt = "Jesteś korektorem.".to_string();
        cfg.user_template = "Popraw:\n\n{text}".to_string();
        let body = request_body(&cfg, "ten wis per");

        assert_eq!(body["model"], json!(cfg.model));
        assert_eq!(body["messages"][0]["content"], json!("Jesteś korektorem."));
        assert_eq!(
            body["messages"][1]["content"],
            json!("Popraw:\n\nten wis per")
        );
    }

    #[test]
    fn wyciszenie_rozumowania_idzie_obiema_drogami() {
        let body = request_body(&config(), "cokolwiek");
        assert_eq!(
            body["chat_template_kwargs"]["enable_thinking"],
            json!(false)
        );
        assert_eq!(body["reasoning_effort"], json!("none"));

        let mut cfg = config();
        cfg.disable_thinking = false;
        let body = request_body(&cfg, "cokolwiek");
        assert!(body.get("reasoning_effort").is_none());
        assert!(body.get("chat_template_kwargs").is_none());
    }

    #[test]
    fn tekst_wychodzi_z_odpowiedzi_serwera() {
        let response = json!({
            "choices": [{"message": {"content": "  Ten Whisper.  "}}]
        });
        assert_eq!(extract_text(&response).unwrap(), "Ten Whisper.");
    }

    #[test]
    fn blad_serwera_wychodzi_z_opisem() {
        let response = json!({"error": {"message": "model not found"}});
        let err = extract_text(&response).expect_err("błąd musi się przebić");
        assert!(format!("{err:#}").contains("model not found"));
    }

    #[test]
    fn plot_z_grawisow_i_cudzyslow_wokol_calosci_znikaja() {
        let response = json!({
            "choices": [{"message": {"content": "```\nTen Whisper.\n```"}}]
        });
        assert_eq!(extract_text(&response).unwrap(), "Ten Whisper.");

        let response = json!({
            "choices": [{"message": {"content": "„Ten Whisper.”"}}]
        });
        assert_eq!(extract_text(&response).unwrap(), "Ten Whisper.");
    }

    /// Jedyny test dotykający serwera. Poza `cargo test --workspace`
    /// (`#[ignore]`), bo wymaga sieci, a odpowiedzi modelu migoczą mimo
    /// `temperature: 0` — sprawdza więc drogę, a nie treść:
    /// `cargo test -p diktate-gtk -- --ignored korekta`.
    #[test]
    #[ignore = "wymaga serwera z modelem"]
    fn korekta_przez_prawdziwy_serwer() {
        let config = config();
        let before = "ten wis per przetwarza cale okno trzydziestu sekund";
        let after = ask(&config, before).expect("serwer ma odpowiedzieć");
        println!("„{before}” → „{after}”");
        assert!(!after.trim().is_empty());
        assert_eq!(foreign_char(before, &after, ""), None);
    }

    /// Droga redakcji przez prawdziwy serwer — tak samo `#[ignore]`:
    /// `cargo test -p diktate-gtk -- --ignored redakcja`.
    #[test]
    #[ignore = "wymaga serwera z modelem"]
    fn redakcja_przez_prawdziwy_serwer() {
        let config = config();
        let before = "dobra to zaczynamy bo mamy mało czasu";
        let after = ask_redaction(&config, "zmień na bardziej formalny", before)
            .expect("serwer ma odpowiedzieć");
        println!("„{before}” → „{after}”");
        assert!(!after.trim().is_empty());
        assert_eq!(foreign_char(before, &after, ""), None);
    }

    #[test]
    fn cudzyslow_w_srodku_zdania_zostaje_na_miejscu() {
        // Zdejmujemy opakowanie całości, nie interpunkcję wypowiedzi.
        let response = json!({
            "choices": [{"message": {"content": "Powiedział „tak” i wyszedł."}}]
        });
        assert_eq!(
            extract_text(&response).unwrap(),
            "Powiedział „tak” i wyszedł."
        );
    }
}
