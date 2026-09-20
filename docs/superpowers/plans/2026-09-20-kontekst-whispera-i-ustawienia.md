# Kontekst dla whispera i okno ustawień — plan implementacji

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Whisper dostaje przy każdym segmencie prompt złożony z szablonu użytkownika i ogona tekstu sprzed kursora, a szablon edytuje się w nowym oknie ustawień GTK.

**Architecture:** Klient (GTK) skleja szablon z `gtk.toml` z ogonem dokumentu i wysyła gotowy prompt metodą IPC `session.context`. Daemon trzyma go w slocie per połączenie i podaje wątkowi transkrypcji przy każdym segmencie; segment będący echem promptu liczy ponownie z pustym promptem. Okno ustawień zapisuje `gtk.toml` przez pisarza przeniesionego z `diktate-cli` do `diktate-config`.

**Tech Stack:** Rust 2021, cargo workspace, GTK 4 + libadwaita 0.7 (`v1_5`), serde/serde_json, toml 0.8, whisper.cpp 1.9.4 przez `diktate-whispercpp-sys`.

**Spec:** `docs/superpowers/specs/2026-09-20-kontekst-whispera-i-ustawienia-design.md`

## Global Constraints

- Komentarze, komunikaty błędów, logi i teksty interfejsu **po polsku**, z pełnymi znakami diakrytycznymi. Nazwy typów, funkcji, zmiennych i klas CSS po angielsku.
- Komentarz `//!` na górze nowego modułu wyjaśnia zasadę działania, nie przepisuje kodu.
- Każdy widok GTK trzyma konwencje z `docs/06-gui.md#konwencje-widoków`. Jedyny nowy wyjątek: okno ustawień to `adw::PreferencesDialog` (wchodzi do `docs/06` w zadaniu 12).
- Logika bez wejścia/wyjścia ma testy jednostkowe działające **bez mikrofonu, bez modelu i bez GTK**.
- Przy filtrach i strażnikach ważniejsze od testów odrzucania są testy tego, czego odrzucić **nie wolno**.
- Budowanie: `cargo build --workspace` (Vulkan), testy `cargo test --workspace`, lint `cargo clippy --workspace --all-targets` — zero ostrzeżeń.
- Symbol zastępczy kontekstu w szablonie: dokładnie `{kontekst}`.
- Wartości domyślne: `max_prompt_chars = 900`, `repetition_guard = true`, `repetition_threshold = 0.8`, `context_chars = 200`, `prompt = ""`.
- Pusty szablon (`prompt = ""`) znaczy „nie wysyłaj kontekstu wcale” — zachowanie sprzed tej zmiany.
- Każde zadanie kończy się commitem; wiadomość po polsku, w trybie orzekającym, ze stopką `Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>`.

---

### Task 1: Moduł `prompt.rs` — składanie, obcinanie, wykrywanie echa

**Files:**
- Create: `crates/diktate-core/src/prompt.rs`
- Modify: `crates/diktate-core/src/lib.rs:8-15` (lista modułów)
- Test: w tym samym pliku, `#[cfg(test)] mod tests`

**Interfaces:**
- Consumes: nic.
- Produces:
  - `pub const CONTEXT_PLACEHOLDER: &str = "{kontekst}"`
  - `pub fn render(template: &str, context: &str) -> String`
  - `pub fn tail(text: &str, chars: usize) -> &str`
  - `pub fn clamp(prompt: &str, max_chars: usize) -> &str`
  - `pub fn looks_like_echo(segment: &str, prompt: &str, threshold: f32) -> bool`
  - `pub fn is_looping(text: &str) -> bool`
  - `pub struct PromptSlot` z `new() -> Self`, `set(&self, text: &str)`, `get(&self) -> String`

- [ ] **Step 1: Napisz testy, zanim powstanie moduł**

Utwórz `crates/diktate-core/src/prompt.rs` z samym nagłówkiem modułu i testami:

```rust
//! Prompt whispera: szablon użytkownika, ogon dokumentu i strażnik echa.
//!
//! Zasada jest jedna — model dostaje dokładnie to, co użytkownik widzi
//! w polu szablonu, z `{kontekst}` podmienionym na tekst sprzed kursora.
//! Nic się nie dokleja po cichu. Sam prompt bywa jednak niebezpieczny:
//! whisper potrafi zamiast transkrypcji powtórzyć własną podpowiedź, więc
//! tu mieszka też wykrywanie echa (`docs/01#kontekst-między-segmentami`).

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
    fn slot_oddaje_to_co_przyjal() {
        let slot = PromptSlot::new();
        assert_eq!(slot.get(), "");
        slot.set("prompt");
        assert_eq!(slot.get(), "prompt");
        slot.set("");
        assert_eq!(slot.get(), "");
    }
}
```

Dopisz moduł do `crates/diktate-core/src/lib.rs` (lista `pub mod`, kolejność alfabetyczna — między `parakeet` a `segmenter`):

```rust
pub mod prompt;
```

- [ ] **Step 2: Uruchom testy, żeby zobaczyć, że nie kompilują się z braku funkcji**

Run: `cargo test -p diktate-core prompt 2>&1 | head -30`
Expected: FAIL — `cannot find function \`render\` in this scope` i podobne.

- [ ] **Step 3: Dopisz implementację nad blokiem testów**

```rust
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
```

- [ ] **Step 4: Uruchom testy modułu**

Run: `cargo test -p diktate-core prompt`
Expected: PASS, 17 testów.

- [ ] **Step 5: Lint i commit**

```bash
cargo clippy -p diktate-core --all-targets
git add crates/diktate-core/src/prompt.rs crates/diktate-core/src/lib.rs
git commit -m "feat(core): moduł promptu — szablon, ogon i strażnik echa

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

### Task 2: `Transcribe` przyjmuje prompt jako argument

**Files:**
- Modify: `crates/diktate-core/src/transcribe.rs:25-28` (trait), `:104-133` (`Transcriber::transcribe`, `impl Transcribe`)
- Modify: `crates/diktate-core/src/parakeet.rs:94`
- Modify: `crates/diktate-core/src/session.rs:349`
- Modify: `crates/diktate-cli/src/main.rs:110`
- Modify: `crates/diktate-core/examples/bench-model.rs:53`, `crates/diktate-core/examples/parakeet-wav.rs:68`
- Test: `crates/diktate-core/src/transcribe.rs` (`#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: nic z zadania 1.
- Produces: `trait Transcribe { fn transcribe(&self, samples: &[f32], prompt: &str) -> Result<String>; }` — od tej chwili każde wywołanie transkrypcji podaje prompt (pusty `""` znaczy „użyj własnego domyślnego”).

- [ ] **Step 1: Napisz test atrapy, który wymusza nowy kształt traitu**

Dopisz na końcu `crates/diktate-core/src/transcribe.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Atrapa silnika: zapisuje prompty, które dostała. Trait musi dać się
    /// zaimplementować bez modelu — inaczej nie da się testować sesji.
    struct Atrapa {
        prompty: Mutex<Vec<String>>,
    }

    impl Transcribe for Atrapa {
        fn transcribe(&self, _samples: &[f32], prompt: &str) -> Result<String> {
            self.prompty.lock().unwrap().push(prompt.to_string());
            Ok("tekst".to_string())
        }
    }

    #[test]
    fn trait_przekazuje_prompt_do_silnika() {
        let atrapa = Atrapa {
            prompty: Mutex::new(Vec::new()),
        };
        let silnik: &dyn Transcribe = &atrapa;
        silnik.transcribe(&[0.0], "Terminy: ggml.").unwrap();
        assert_eq!(atrapa.prompty.lock().unwrap().as_slice(), ["Terminy: ggml."]);
    }
}
```

- [ ] **Step 2: Uruchom test i zobacz, że trait ma jeszcze stary kształt**

Run: `cargo test -p diktate-core transcribe 2>&1 | head -20`
Expected: FAIL — `method \`transcribe\` has 2 parameters but the declaration has 1`.

- [ ] **Step 3: Zmień trait i obie implementacje**

W `transcribe.rs` zmień deklarację traitu:

```rust
pub trait Transcribe: Send + Sync {
    /// Transkrybuje mono 16 kHz ([`crate::SAMPLE_RATE`]). Pusty wynik znaczy
    /// „nie rozpoznano mowy" i nie jest błędem.
    ///
    /// `prompt` to podpowiedź kontekstowa złożona przez klienta
    /// (`docs/07#kontekst-dla-whispera`). Pusty prompt znaczy „weź własny
    /// domyślny", a backend, który promptu nie przyjmuje, ma prawo go
    /// zignorować — ale musi to powiedzieć w komentarzu.
    fn transcribe(&self, samples: &[f32], prompt: &str) -> Result<String>;
}
```

W `Transcriber::transcribe` zmień sygnaturę i wybór promptu:

```rust
    /// Transkrybuje mono 16 kHz. Zwraca sklejony tekst wszystkich segmentów.
    ///
    /// Pusty `prompt` cofa się do `initial_prompt` z konfiguracji daemona —
    /// tak działają klienty, które kontekstu nie przysyłają (`diktate-cli`,
    /// przykłady), i tak wyglądało zachowanie przed wprowadzeniem kontekstu.
    pub fn transcribe(&self, samples: &[f32], prompt: &str) -> Result<String> {
        if samples.is_empty() {
            return Ok(String::new());
        }
        let language = cstring(&self.language)?;
        let chosen = if prompt.is_empty() {
            self.initial_prompt.as_str()
        } else {
            prompt
        };
        let prompt = cstring(chosen)?;
        // …reszta ciała bez zmian…
```

oraz `impl Transcribe for Transcriber`:

```rust
impl Transcribe for Transcriber {
    fn transcribe(&self, samples: &[f32], prompt: &str) -> Result<String> {
        Transcriber::transcribe(self, samples, prompt)
    }
}
```

W `parakeet.rs:94`:

```rust
    /// Prompt jest **ignorowany**: Parakeet nie przyjmuje podpowiedzi
    /// kontekstowej i nie ma udawać, że przyjmuje (`docs/12`).
    fn transcribe(&self, samples: &[f32], _prompt: &str) -> Result<String> {
```

W `session.rs:349` tymczasowo (właściwy slot dochodzi w zadaniu 3):

```rust
        let result = transcriber.transcribe(&segment.audio, "");
```

W `crates/diktate-cli/src/main.rs:110`, `examples/bench-model.rs:53` i `examples/parakeet-wav.rs:68` dopisz drugi argument `""`.

- [ ] **Step 4: Testy i budowanie całości**

Run: `cargo test -p diktate-core && cargo build --workspace --all-targets`
Expected: PASS; budowanie bez błędów.

- [ ] **Step 5: Commit**

```bash
git add crates/diktate-core crates/diktate-cli/src/main.rs
git commit -m "refactor(core): prompt jako argument transkrypcji

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

### Task 3: Sesja czyta slot promptu i pilnuje echa

**Files:**
- Modify: `crates/diktate-core/src/session.rs:49-75` (`SessionConfig`), `:182-233` (`Session::start`), `:338-360` (`run_transcriber`)
- Test: `crates/diktate-core/src/session.rs` (`#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: `diktate_core::prompt::{PromptSlot, clamp, looks_like_echo, is_looping}` (zadanie 1), `Transcribe::transcribe(&[f32], &str)` (zadanie 2).
- Produces:
  - `SessionConfig` z trzema nowymi polami: `max_prompt_chars: usize`, `repetition_guard: bool`, `repetition_threshold: f32`
  - `Session::start(transcriber, config, prompt: Arc<PromptSlot>, on_segment, on_backlog, on_level)` — slot wchodzi **trzecim** argumentem, przed domknięciami.

- [ ] **Step 1: Napisz testy wątku transkrypcji**

Dopisz do `#[cfg(test)] mod tests` w `session.rs`:

```rust
    use crate::prompt::PromptSlot;
    use crate::transcribe::Transcribe;
    use std::sync::mpsc::sync_channel;

    /// Atrapa silnika: oddaje z góry zadane teksty i zapisuje prompty.
    struct Atrapa {
        odpowiedzi: Mutex<std::collections::VecDeque<String>>,
        prompty: Mutex<Vec<String>>,
    }

    impl Atrapa {
        fn nowa(odpowiedzi: &[&str]) -> Self {
            Self {
                odpowiedzi: Mutex::new(odpowiedzi.iter().map(|s| s.to_string()).collect()),
                prompty: Mutex::new(Vec::new()),
            }
        }
    }

    impl Transcribe for Atrapa {
        fn transcribe(&self, _samples: &[f32], prompt: &str) -> anyhow::Result<String> {
            self.prompty.lock().unwrap().push(prompt.to_string());
            Ok(self
                .odpowiedzi
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| "nic".to_string()))
        }
    }

    fn przepusc(
        atrapa: &Atrapa,
        slot: &PromptSlot,
        config: SessionConfig,
        ile: usize,
    ) -> Vec<String> {
        let (tx, rx) = sync_channel(ile);
        for seq in 0..ile {
            tx.send((seq as u64, segment(0.1), Instant::now())).unwrap();
        }
        drop(tx);
        let wyniki = Arc::new(Mutex::new(Vec::new()));
        let zebrane = Arc::clone(&wyniki);
        let backlog = Backlog::new(|_| {});
        run_transcriber(
            rx,
            atrapa,
            slot,
            &config,
            move |segment: TranscribedSegment| zebrane.lock().unwrap().push(segment.text),
            &backlog,
        );
        let out = wyniki.lock().unwrap().clone();
        out
    }

    #[test]
    fn transkrypcja_bierze_prompt_ze_slotu() {
        let atrapa = Atrapa::nowa(&["pierwszy", "drugi"]);
        let slot = PromptSlot::new();
        slot.set("Terminy: ggml.");
        przepusc(&atrapa, &slot, SessionConfig::default(), 2);
        assert_eq!(
            atrapa.prompty.lock().unwrap().as_slice(),
            ["Terminy: ggml.", "Terminy: ggml."]
        );
    }

    #[test]
    fn prompt_dluzszy_niz_limit_jest_obcinany_od_poczatku() {
        let atrapa = Atrapa::nowa(&["tekst"]);
        let slot = PromptSlot::new();
        slot.set("abcdefghij");
        let config = SessionConfig {
            max_prompt_chars: 4,
            ..SessionConfig::default()
        };
        przepusc(&atrapa, &slot, config, 1);
        assert_eq!(atrapa.prompty.lock().unwrap().as_slice(), ["ghij"]);
    }

    #[test]
    fn echo_promptu_jest_liczone_ponownie_bez_kontekstu() {
        let echo = "Transkrypcja notatek technicznych. Terminy: Kubernetes, Wayland, systemd.";
        let atrapa = Atrapa::nowa(&[echo, "prawdziwa wypowiedź po drugim przebiegu"]);
        let slot = PromptSlot::new();
        slot.set(echo);
        let wyniki = przepusc(&atrapa, &slot, SessionConfig::default(), 1);
        assert_eq!(wyniki, ["prawdziwa wypowiedź po drugim przebiegu"]);
        // Drugi przebieg poszedł z pustym promptem — inaczej echo wróciłoby.
        assert_eq!(atrapa.prompty.lock().unwrap()[1], "");
    }

    #[test]
    fn zwykla_wypowiedz_idzie_jednym_przebiegiem() {
        let atrapa = Atrapa::nowa(&["Wiązania okazały się szybsze, niż zakładałem."]);
        let slot = PromptSlot::new();
        slot.set("Wczoraj zbudowałem wiązania rustowe do whisper.cpp i zmierzyłem czasy.");
        przepusc(&atrapa, &slot, SessionConfig::default(), 1);
        assert_eq!(atrapa.prompty.lock().unwrap().len(), 1);
    }

    #[test]
    fn wylaczony_straznik_nie_powtarza_przebiegu() {
        let echo = "Transkrypcja notatek technicznych. Terminy: Kubernetes, Wayland, systemd.";
        let atrapa = Atrapa::nowa(&[echo, "drugi przebieg"]);
        let slot = PromptSlot::new();
        slot.set(echo);
        let config = SessionConfig {
            repetition_guard: false,
            ..SessionConfig::default()
        };
        przepusc(&atrapa, &slot, config, 1);
        assert_eq!(atrapa.prompty.lock().unwrap().len(), 1);
    }
```

- [ ] **Step 2: Uruchom testy — nie kompilują się, bo `run_transcriber` ma inną sygnaturę**

Run: `cargo test -p diktate-core session 2>&1 | head -20`
Expected: FAIL — `this function takes 4 arguments but 6 arguments were supplied`.

- [ ] **Step 3: Rozszerz `SessionConfig`, `Session::start` i `run_transcriber`**

W `SessionConfig` dopisz pola i domyślne (obok istniejących):

```rust
    /// Twardy limit długości promptu. Whisper mieści w prompcie ~224 tokeny,
    /// a klient bywa cudzy — dłuższy prompt obcinamy zamiast ufać.
    pub max_prompt_chars: usize,
    /// Czy segment będący echem promptu liczyć ponownie bez kontekstu.
    pub repetition_guard: bool,
    /// Podobieństwo, od którego segment uchodzi za echo promptu.
    pub repetition_threshold: f32,
```

```rust
            max_prompt_chars: 900,
            repetition_guard: true,
            repetition_threshold: 0.8,
```

`Session::start` dostaje slot trzecim argumentem i podaje go wątkowi:

```rust
    pub fn start<F, G, L>(
        transcriber: Arc<dyn Transcribe>,
        config: SessionConfig,
        prompt: Arc<PromptSlot>,
        on_segment: F,
        on_backlog: G,
        on_level: L,
    ) -> Result<Self>
```

```rust
        let transcriber_backlog = Arc::clone(&backlog);
        let transcriber_handle = std::thread::spawn(move || {
            run_transcriber(
                segment_rx,
                transcriber.as_ref(),
                prompt.as_ref(),
                &config,
                on_segment,
                &transcriber_backlog,
            );
        });
```

`SessionConfig` wywodzi `Clone`, a wątek segmentujący bierze z niego tylko `segmenter` i `level_interval_ms` — sklonuj konfigurację przed uruchomieniem wątków i przenieś kopię do wątku transkrypcji.

`run_transcriber` czyta slot przed każdym segmentem i pilnuje echa:

```rust
fn run_transcriber<F, G: Fn(u32)>(
    segment_rx: Receiver<(u64, Segment, Instant)>,
    transcriber: &dyn Transcribe,
    prompt_slot: &PromptSlot,
    config: &SessionConfig,
    on_segment: F,
    backlog: &Backlog<G>,
) where
    F: Fn(TranscribedSegment),
{
    while let Ok((seq, segment, queued_at)) = segment_rx.recv() {
        let waited = queued_at.elapsed();
        let started = Instant::now();
        // Slot czytany przy każdym segmencie, nie raz na sesję: kontekst
        // przysłany w trakcie mówienia ma działać od następnej wypowiedzi.
        let stored = prompt_slot.get();
        let prompt = prompt::clamp(&stored, config.max_prompt_chars);
        let result = transcriber
            .transcribe(&segment.audio, prompt)
            .and_then(|text| {
                // Strażnik pętli: prompt potrafi skłonić model do przepisania
                // podpowiedzi zamiast transkrypcji. Drugi przebieg bez promptu
                // kosztuje jeden przebieg modelu — tanio wobec notatki pełnej
                // powtórzonego kontekstu (docs/01, docs/07).
                let echo = !prompt.is_empty()
                    && config.repetition_guard
                    && (prompt::looks_like_echo(&text, prompt, config.repetition_threshold)
                        || prompt::is_looping(&text));
                if echo {
                    eprintln!(
                        "sesja: segment {seq} wygląda na echo promptu — liczę ponownie bez kontekstu"
                    );
                    transcriber.transcribe(&segment.audio, "")
                } else {
                    Ok(text)
                }
            });
        // …reszta ciała (log czasów, filtry, on_segment) bez zmian…
```

Dopisz `use crate::prompt::{self, PromptSlot};` do importów modułu.

- [ ] **Step 4: Testy**

Run: `cargo test -p diktate-core`
Expected: PASS (w tym pięć nowych testów sesji).

- [ ] **Step 5: Commit**

```bash
cargo clippy -p diktate-core --all-targets
git add crates/diktate-core/src/session.rs
git commit -m "feat(core): sesja czyta prompt ze slotu i pilnuje echa

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

### Task 4: `session.context` w protokole

**Files:**
- Modify: `crates/diktate-proto/src/lib.rs:28-43` (stałe, `Subscription`), `:45-72` (`Request`), `:91-112` (`Response`)
- Test: `crates/diktate-proto/src/lib.rs` (`#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: nic.
- Produces:
  - `pub const METHOD_SESSION_CONTEXT: &str = "session.context";`
  - `pub struct ContextParams { pub text: String }`
  - `pub enum RequestParams { Subscribe(Subscription), Context(ContextParams) }` (untagged)
  - `Request::context(id: u64, text: impl Into<String>) -> Request`
  - `Response::ok_with(id: u64, result: serde_json::Value) -> Response`
  - `Request::params` ma typ `Option<RequestParams>`

- [ ] **Step 1: Napisz testy serializacji**

Dopisz do `#[cfg(test)] mod tests` w `diktate-proto`:

```rust
    #[test]
    fn parametry_z_polem_text_to_kontekst() {
        let request: Request =
            serde_json::from_str(r#"{"jsonrpc":"2.0","id":1,"method":"session.context","params":{"text":"abc"}}"#)
                .unwrap();
        match request.params {
            Some(RequestParams::Context(params)) => assert_eq!(params.text, "abc"),
            other => panic!("kontekst rozpoznany jako {other:?}"),
        }
    }

    // Bez `deny_unknown_fields` `Subscription` (same pola z `default`)
    // złapałaby każdy obiekt, także ten z kontekstem — i kontekst zniknąłby
    // po cichu jako subskrypcja bez poziomów.
    #[test]
    fn parametry_z_polem_levels_to_nadal_subskrypcja() {
        let request: Request =
            serde_json::from_str(r#"{"jsonrpc":"2.0","id":1,"method":"session.subscribe","params":{"levels":true}}"#)
                .unwrap();
        match request.params {
            Some(RequestParams::Subscribe(params)) => assert!(params.levels),
            other => panic!("subskrypcja rozpoznana jako {other:?}"),
        }
    }

    #[test]
    fn zadanie_kontekstu_serializuje_sie_z_tekstem() {
        let line = serde_json::to_string(&Request::context(7, "Terminy: ggml.")).unwrap();
        assert!(line.contains(r#""method":"session.context""#), "{line}");
        assert!(line.contains(r#""text":"Terminy: ggml.""#), "{line}");
    }

    #[test]
    fn odpowiedz_z_wynikiem_niesie_liczbe_znakow() {
        let response = Response::ok_with(7, serde_json::json!({ "chars": 14 }));
        let line = serde_json::to_string(&response).unwrap();
        assert!(line.contains(r#""chars":14"#), "{line}");
    }
```

- [ ] **Step 2: Uruchom testy**

Run: `cargo test -p diktate-proto 2>&1 | head -20`
Expected: FAIL — `cannot find type \`RequestParams\``.

- [ ] **Step 3: Implementacja**

```rust
pub const METHOD_SESSION_CONTEXT: &str = "session.context";
```

`Subscription` dostaje `deny_unknown_fields`:

```rust
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Subscription {
```

Nowe parametry i suma:

```rust
/// Kontekst transkrypcji: gotowy prompt złożony przez klienta
/// (`docs/02#kontekst-transkrypcji`). Pusty `text` czyści slot.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextParams {
    pub text: String,
}

/// Parametry żądania. `untagged`, bo JSON-RPC nie niesie znacznika wariantu —
/// rozróżnia je kształt obiektu, dlatego **oba** warianty mają
/// `deny_unknown_fields`: bez tego `Subscription` (same pola z wartością
/// domyślną) dopasowałaby się do dowolnego obiektu i zjadła kontekst.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RequestParams {
    Subscribe(Subscription),
    Context(ContextParams),
}
```

W `Request`: `pub params: Option<RequestParams>`, a `Request::subscribe` owija w `RequestParams::Subscribe(subscription)`. Dochodzi konstruktor:

```rust
    pub fn context(id: u64, text: impl Into<String>) -> Self {
        Self {
            params: Some(RequestParams::Context(ContextParams { text: text.into() })),
            ..Self::new(id, METHOD_SESSION_CONTEXT)
        }
    }
```

W `Response`:

```rust
    /// Odpowiedź z wynikiem innym niż pusty obiekt — `session.context`
    /// odsyła liczbę znaków, które daemon przyjął.
    pub fn ok_with(id: u64, result: serde_json::Value) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            result: Some(result),
            error: None,
        }
    }
```

- [ ] **Step 4: Napraw jedyne miejsce, które psuje zmiana typu `params`**

Zmiana typu `Request::params` wywraca gałąź subskrypcji w daemonie
(`crates/diktate-daemon/src/main.rs:327-331`). Napraw ją **w tym zadaniu**,
żeby każdy commit zostawiał kompilujący się workspace:

```rust
            METHOD_SESSION_SUBSCRIBE => {
                let wanted = matches!(
                    request.params,
                    Some(RequestParams::Subscribe(Subscription { levels: true }))
                );
                levels_wanted.store(wanted, Ordering::Relaxed);
                send(&writer, &Response::ok(request.id))?;
            }
```

Uzupełnij import: `diktate_proto::{RequestParams, Subscription}`.

- [ ] **Step 5: Testy i budowanie**

Run: `cargo test -p diktate-proto && cargo build --workspace`
Expected: PASS, workspace się buduje.

- [ ] **Step 6: Commit**

```bash
cargo clippy -p diktate-proto -p diktate-daemon --all-targets
git add crates/diktate-proto/src/lib.rs crates/diktate-daemon/src/main.rs
git commit -m "feat(proto): metoda session.context i parametry żądania

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

### Task 5: Konfiguracja — `[whisper.context]` i `[whisper]` w gtk.toml

**Files:**
- Modify: `crates/diktate-config/src/lib.rs:149-175` (`WhisperConfig`), `:267-277` (`GtkConfig`)
- Test: `crates/diktate-config/src/lib.rs` (`#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: nic.
- Produces:
  - `WhisperConfig { …, pub context: WhisperContextConfig }`
  - `pub struct WhisperContextConfig { pub max_prompt_chars: usize, pub repetition_guard: bool, pub repetition_threshold: f32 }`
  - `GtkConfig { …, pub whisper: GtkWhisperConfig }`
  - `pub struct GtkWhisperConfig { pub prompt: String, pub context_chars: usize }`

- [ ] **Step 1: Napisz testy**

```rust
    #[test]
    fn kontekst_whispera_ma_wartosci_domyslne() {
        let cfg = DaemonConfig::from_toml("").unwrap();
        assert_eq!(cfg.whisper.context.max_prompt_chars, 900);
        assert!(cfg.whisper.context.repetition_guard);
        assert!((cfg.whisper.context.repetition_threshold - 0.8).abs() < f32::EPSILON);
    }

    #[test]
    fn kontekst_whispera_czyta_sie_z_pliku() {
        let cfg = DaemonConfig::from_toml(
            "[whisper.context]\nmax_prompt_chars = 400\nrepetition_guard = false\n",
        )
        .unwrap();
        assert_eq!(cfg.whisper.context.max_prompt_chars, 400);
        assert!(!cfg.whisper.context.repetition_guard);
    }

    #[test]
    fn nieznane_pole_kontekstu_jest_bledem() {
        assert!(DaemonConfig::from_toml("[whisper.context]\nchars = 200\n").is_err());
    }

    #[test]
    fn szablon_promptu_ma_wartosci_domyslne() {
        let cfg = GtkConfig::from_toml("").unwrap();
        assert_eq!(cfg.whisper.prompt, "");
        assert_eq!(cfg.whisper.context_chars, 200);
    }

    #[test]
    fn szablon_promptu_czyta_sie_z_pliku() {
        let cfg = GtkConfig::from_toml(
            "[whisper]\nprompt = \"Terminy: ggml.\\n{kontekst}\"\ncontext_chars = 400\n",
        )
        .unwrap();
        assert_eq!(cfg.whisper.prompt, "Terminy: ggml.\n{kontekst}");
        assert_eq!(cfg.whisper.context_chars, 400);
    }
```

- [ ] **Step 2: Uruchom testy**

Run: `cargo test -p diktate-config 2>&1 | head -20`
Expected: FAIL — `no field \`context\` on type \`WhisperConfig\``.

- [ ] **Step 3: Implementacja**

```rust
/// Kontekst między segmentami po stronie daemona
/// (`docs/01#kontekst-między-segmentami`).
///
/// Długość samego ogona dokumentu **nie** jest tutaj: tnie go klient, bo to
/// on zna dokument. Daemon dostaje gotowy prompt i pilnuje tylko dwóch
/// rzeczy — żeby nie był absurdalnie długi i żeby model go nie przepisał
/// zamiast transkrypcji.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct WhisperContextConfig {
    /// Twardy limit długości promptu; nadmiar obcinany od początku.
    pub max_prompt_chars: usize,
    /// Czy segment będący echem promptu liczyć ponownie bez kontekstu.
    pub repetition_guard: bool,
    /// Podobieństwo, od którego segment uchodzi za echo.
    pub repetition_threshold: f32,
}

impl Default for WhisperContextConfig {
    fn default() -> Self {
        Self {
            max_prompt_chars: 900,
            repetition_guard: true,
            repetition_threshold: 0.8,
        }
    }
}
```

W `WhisperConfig` dopisz pole `pub context: WhisperContextConfig` (i `context: WhisperContextConfig::default()` w `Default`). **Pole tabelowe musi stać na końcu struktury** — TOML nie przyjmuje klucza prostego po tabeli.

```rust
/// Prompt whispera po stronie klienta (`docs/07#kontekst-dla-whispera`).
///
/// Szablon jest tym, co użytkownik widzi w oknie ustawień, i dokładnie tym,
/// co dostaje model: `{kontekst}` podmienia się na ogon tekstu sprzed
/// kursora. Pusty szablon znaczy „nie wysyłaj kontekstu wcale" — daemon
/// zostaje wtedy przy własnym `whisper.initial_prompt`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct GtkWhisperConfig {
    pub prompt: String,
    /// Ile znaków przed punktem wstawiania wchodzi w `{kontekst}`.
    pub context_chars: usize,
}

impl Default for GtkWhisperConfig {
    fn default() -> Self {
        Self {
            prompt: String::new(),
            context_chars: 200,
        }
    }
}
```

W `GtkConfig` dopisz `pub whisper: GtkWhisperConfig`.

- [ ] **Step 4: Testy**

Run: `cargo test -p diktate-config`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
cargo clippy -p diktate-config --all-targets
git add crates/diktate-config/src/lib.rs
git commit -m "feat(config): sekcje kontekstu whispera w daemon.toml i gtk.toml

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

### Task 6: Daemon obsługuje `session.context`

**Files:**
- Modify: `crates/diktate-daemon/src/main.rs:120-145` (`build_transcriber` — bez zmian), `:186-206` (`session_config`), `:209-230` (`serve`), `:236-330` (obsługa `session.start`), `:327-331` (obsługa `subscribe`), dopisanie gałęzi `METHOD_SESSION_CONTEXT`
- Modify: `scripts/smoke-ipc.py`

**Interfaces:**
- Consumes: `PromptSlot` (zadanie 1), `Session::start(..., prompt, ...)` (zadanie 3), `RequestParams`, `METHOD_SESSION_CONTEXT`, `Response::ok_with` (zadanie 4), `WhisperContextConfig` (zadanie 5).
- Produces: daemon odpowiadający na `session.context` wynikiem `{ "chars": <liczba znaków przyjętego promptu> }`.

- [ ] **Step 1: Przenieś ustawienia kontekstu do `session_config`**

```rust
        vad_threshold: config.vad.threshold,
        queue_capacity: config.queue.max_pending_segments,
        level_interval_ms: config.audio.level_interval_ms,
        max_prompt_chars: config.whisper.context.max_prompt_chars,
        repetition_guard: config.whisper.context.repetition_guard,
        repetition_threshold: config.whisper.context.repetition_threshold,
```

- [ ] **Step 2: Załóż slot na połączenie i podaj go sesji**

W `serve`, obok `levels_wanted`:

```rust
    // Prompt należy do połączenia, nie do sesji: kontekst przysłany przed
    // pierwszym `session.start` ma obowiązywać od pierwszej wypowiedzi,
    // a `session.stop` nie jest powodem, żeby go zapominać (docs/02).
    let prompt = Arc::new(PromptSlot::new());
```

W wywołaniu `Session::start` dodaj `Arc::clone(&prompt)` jako trzeci argument (przed domknięciem `move |segment|`).

- [ ] **Step 3: Dopisz gałąź metody**

Za gałęzią `METHOD_SESSION_SUBSCRIBE`:

```rust
            METHOD_SESSION_CONTEXT => {
                // Podpowiedź, nie zobowiązanie: żądanie bez parametrów albo
                // z parametrami innego kształtu czyści slot i kończy się
                // powodzeniem. Transkrypcja ma działać także wtedy, gdy nikt
                // kontekstu nie przysłał (docs/02).
                let text = match request.params {
                    Some(RequestParams::Context(params)) => params.text,
                    _ => String::new(),
                };
                prompt.set(&text);
                let chars = text.chars().count();
                eprintln!("diktate-daemon: kontekst — {chars} znaków");
                send(
                    &writer,
                    &Response::ok_with(request.id, serde_json::json!({ "chars": chars })),
                )?;
            }
```

Gałąź subskrypcji jest już naprawiona w zadaniu 4 — nie ruszaj jej.
Uzupełnij importy: `diktate_core::prompt::PromptSlot`,
`diktate_proto::METHOD_SESSION_CONTEXT` (`RequestParams` jest już
zaimportowane).

- [ ] **Step 4: Zbuduj i sprawdź test dymny**

```bash
cargo build --workspace
./target/debug/diktate-daemon --restart &
sleep 5 && python3 scripts/smoke-ipc.py
```

Expected: test dymny przechodzi jak dotąd.

- [ ] **Step 5: Dopisz krok kontekstu do testu dymnego**

W `scripts/smoke-ipc.py`, przed wysłaniem `session.start`, dołóż wywołanie `session.context` z tekstem `"Terminy: ggml, Wayland.\nOstatnie zdanie notatki."` i asercję, że odpowiedź niesie `result["chars"] == 48` (23 znaki + przełamanie + 24 znaki), a potem drugie wywołanie z pustym tekstem i asercją `chars == 0`. Trzymaj styl istniejących kroków w pliku (ta sama funkcja pomocnicza do żądania i odczytu odpowiedzi, komunikaty po polsku).

Run: `python3 scripts/smoke-ipc.py`
Expected: wszystkie kroki OK; w logu daemona `diktate-daemon: kontekst — 48 znaków`.

- [ ] **Step 6: Commit**

```bash
cargo clippy -p diktate-daemon --all-targets
git add crates/diktate-daemon/src/main.rs scripts/smoke-ipc.py
git commit -m "feat(daemon): przyjmuje kontekst transkrypcji od klienta

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

### Task 7: Klient GTK — wysyłka kontekstu i ogon dokumentu

**Files:**
- Modify: `crates/diktate-gtk/src/client.rs:134-150`
- Modify: `crates/diktate-gtk/src/view.rs` (nowa metoda `context_before`, obok `text()`)
- Test: `crates/diktate-gtk/src/view.rs` (`#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: `Request::context` (zadanie 4), `diktate_core::prompt::tail` (zadanie 1).
- Produces:
  - `Client::send_context(&self, text: &str) -> Result<()>`
  - `DocumentView::context_before(&self, chars: usize) -> String`

- [ ] **Step 1: Napisz testy ogona**

Dopisz do testów `view.rs` (te testy tworzą widżety, więc idą do istniejącego bloku z `gtk4::init`, wzorem sąsiednich testów bufora):

```rust
    #[test]
    fn kontekst_to_ogon_tekstu_przed_karetka() {
        let document = przygotuj();
        document.replace_all("Wczoraj zbudowałem wiązania rustowe do whisper.cpp.");
        assert_eq!(
            document.context_before(12),
            "whisper.cpp."
        );
    }

    #[test]
    fn kontekst_konczy_sie_na_poczatku_zaznaczenia() {
        let document = przygotuj();
        document.replace_all("pierwsze zdanie. drugie zdanie.");
        zaznacz(&document, 17, 31);
        assert_eq!(document.context_before(200), "pierwsze zdanie. ");
    }

    #[test]
    fn krotki_tekst_wchodzi_w_kontekst_w_calosci() {
        let document = przygotuj();
        document.replace_all("krótko");
        assert_eq!(document.context_before(200), "krótko");
    }

    #[test]
    fn pusta_notatka_daje_pusty_kontekst() {
        let document = przygotuj();
        assert_eq!(document.context_before(200), "");
    }
```

- [ ] **Step 2: Uruchom testy**

Run: `cargo test -p diktate-gtk context 2>&1 | head -20`
Expected: FAIL — `no method named \`context_before\``.

- [ ] **Step 3: Implementacja w `view.rs`**

Obok `pub fn text(&self)`:

```rust
    /// Ogon tekstu przed punktem wstawiania — materiał na `{kontekst}`
    /// w prompcie whispera (`docs/07#kontekst-dla-whispera`).
    ///
    /// Przy zaznaczeniu liczy się jego **początek**: nowy tekst trafi w to
    /// miejsce, więc model ma widzieć to, co je poprzedza, a nie to, co
    /// zaraz zniknie.
    pub fn context_before(&self, chars: usize) -> String {
        let start = self.buffer.start_iter();
        let cut = match self.buffer.selection_bounds() {
            Some((from, _)) => from,
            None => {
                let mark = self.buffer.get_insert();
                self.buffer.iter_at_mark(&mark)
            }
        };
        let before = self.buffer.text(&start, &cut, false).to_string();
        diktate_core::prompt::tail(&before, chars).to_string()
    }
```

- [ ] **Step 4: Dopisz wysyłkę w `client.rs`**

```rust
    /// Przysyła daemonowi gotowy prompt dla whispera. To **podpowiedź**:
    /// nieudana wysyłka nie jest awarią sesji, a pusty tekst czyści slot.
    pub fn send_context(&self, text: &str) -> Result<()> {
        self.send_request(&Request::context(self.take_id(), text))
    }
```

- [ ] **Step 5: Testy i lint**

Run: `cargo test -p diktate-gtk && cargo clippy -p diktate-gtk --all-targets`
Expected: PASS, zero ostrzeżeń.

- [ ] **Step 6: Commit**

```bash
git add crates/diktate-gtk/src/view.rs crates/diktate-gtk/src/client.rs
git commit -m "feat(gtk): ogon tekstu przed kursorem i wysyłka kontekstu

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

### Task 8: Okno składa prompt i wysyła go z odbiciem

**Files:**
- Create: `crates/diktate-gtk/src/context.rs`
- Modify: `crates/diktate-gtk/src/main.rs` (deklaracja modułu przy pozostałych `mod`, podpięcie w `build_ui` przy `buffer.connect_changed`)
- Test: `crates/diktate-gtk/src/context.rs`

**Interfaces:**
- Consumes: `DocumentView::context_before` i `Client::send_context` (zadanie 7), `prompt::render` (zadanie 1), `GtkWhisperConfig` (zadanie 5).
- Produces:
  - `pub struct ContextSender` z `new(config: GtkWhisperConfig) -> Rc<Self>`, `set_config(&self, config: GtkWhisperConfig)`, `set_client(&self, client: Rc<Client>)`, `schedule(self: &Rc<Self>, document: &Rc<DocumentView>)`, `send_now(&self, document: &Rc<DocumentView>)`

  Klienta trzyma **sam sender**, bo w `build_ui` klienta jeszcze nie ma — połączenie staje w tle i dociera dopiero do `Ui::attach` (`main.rs:553`). Wysyłka bez klienta jest cichym brakiem działania, nie błędem.
  - `pub fn compose(config: &GtkWhisperConfig, before: &str) -> String` — czysta funkcja, testowana bez GTK

- [ ] **Step 1: Napisz testy składania promptu**

Utwórz `crates/diktate-gtk/src/context.rs` z nagłówkiem i testami:

```rust
//! Kontekst dla whispera: co klient wysyła daemonowi i jak często.
//!
//! Prompt powstaje z szablonu użytkownika i ogona tekstu sprzed kursora
//! (`docs/07#kontekst-dla-whispera`). Wysyłka jest odbita w czasie i
//! pomijana, gdy prompt się nie zmienił — pisanie z klawiatury zmienia
//! bufor przy każdym znaku, a gniazdo nie ma tego oglądać.

#[cfg(test)]
mod tests {
    use super::*;

    fn config(prompt: &str, chars: usize) -> GtkWhisperConfig {
        GtkWhisperConfig {
            prompt: prompt.to_string(),
            context_chars: chars,
        }
    }

    #[test]
    fn szablon_dostaje_ogon_dokumentu() {
        let out = compose(&config("Terminy: ggml.\n{kontekst}", 200), "Ostatnie zdanie.");
        assert_eq!(out, "Terminy: ggml.\nOstatnie zdanie.");
    }

    #[test]
    fn pusty_szablon_nie_wysyla_niczego() {
        assert_eq!(compose(&config("", 200), "Ostatnie zdanie."), "");
    }

    #[test]
    fn szablon_bez_symbolu_nie_niesie_dokumentu() {
        let out = compose(&config("Terminy: ggml.", 200), "Ostatnie zdanie.");
        assert_eq!(out, "Terminy: ggml.");
    }

    #[test]
    fn zerowa_dlugosc_kontekstu_daje_sam_szablon() {
        let out = compose(&config("Terminy: ggml.\n{kontekst}", 0), "Ostatnie zdanie.");
        assert_eq!(out, "Terminy: ggml.");
    }
}
```

- [ ] **Step 2: Uruchom testy**

Run: `cargo test -p diktate-gtk context 2>&1 | head -20`
Expected: FAIL — `cannot find function \`compose\``.

- [ ] **Step 3: Implementacja**

```rust
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use diktate_config::GtkWhisperConfig;
use diktate_core::prompt;

use crate::view::DocumentView;

/// Odstęp między ostatnią zmianą bufora a wysyłką kontekstu.
const DEBOUNCE: Duration = Duration::from_millis(400);

/// Skleja prompt z szablonu i ogona dokumentu. Ogon jest już obcięty do
/// `context_chars` przez wywołującego; tutaj zostaje samo podstawienie.
pub fn compose(config: &GtkWhisperConfig, before: &str) -> String {
    if config.context_chars == 0 {
        return prompt::render(&config.prompt, "");
    }
    prompt::render(&config.prompt, before)
}

/// Wysyłka kontekstu: pamięta ostatnio wysłany prompt i odbija zmiany
/// w czasie.
pub struct ContextSender {
    config: RefCell<GtkWhisperConfig>,
    last: RefCell<String>,
    pending: std::cell::Cell<bool>,
    /// Klient dociera później niż okno: połączenie staje w osobnym wątku
    /// i trafia do `Ui::attach`. Do tego czasu kontekst nie ma dokąd iść.
    client: RefCell<Option<Rc<crate::client::Client>>>,
}

impl ContextSender {
    pub fn new(config: GtkWhisperConfig) -> Rc<Self> {
        Rc::new(Self {
            config: RefCell::new(config),
            last: RefCell::new(String::new()),
            pending: std::cell::Cell::new(false),
            client: RefCell::new(None),
        })
    }

    /// Nowe ustawienia z okna ustawień. Kasuje pamięć ostatnio wysłanego
    /// promptu — zmiana szablonu ma dojść do daemona, choćby dokument
    /// się nie ruszył.
    pub fn set_config(&self, config: GtkWhisperConfig) {
        *self.config.borrow_mut() = config;
        self.last.borrow_mut().clear();
    }

    /// Podłączony klient. Wołane raz, z `Ui::attach`.
    pub fn set_client(&self, client: Rc<crate::client::Client>) {
        *self.client.borrow_mut() = Some(client);
    }

    /// Wysyłka po odbiciu: kolejne zmiany w trakcie oczekiwania nie dokładają
    /// kolejnych timerów, tylko korzystają z tego, który już biegnie.
    pub fn schedule(self: &Rc<Self>, document: &Rc<DocumentView>) {
        if self.pending.get() {
            return;
        }
        self.pending.set(true);
        let sender = Rc::clone(self);
        let document = Rc::clone(document);
        gtk4::glib::timeout_add_local_once(DEBOUNCE, move || {
            sender.pending.set(false);
            sender.send_now(&document);
        });
    }

    /// Wysyłka natychmiastowa — po zapisaniu ustawień i przy starcie sesji.
    pub fn send_now(&self, document: &Rc<DocumentView>) {
        let Some(client) = self.client.borrow().clone() else {
            return;
        };
        let config = self.config.borrow().clone();
        if config.prompt.trim().is_empty() {
            return;
        }
        let before = document.context_before(config.context_chars);
        let composed = compose(&config, &before);
        if composed == *self.last.borrow() {
            return;
        }
        // Kontekst jest podpowiedzią, nie czynnością użytkownika: nieudana
        // wysyłka trafia do logu i tyle. Okno nie ma o niej krzyczeć.
        match client.send_context(&composed) {
            Ok(()) => *self.last.borrow_mut() = composed,
            Err(err) => eprintln!("diktate-gtk: nie udało się wysłać kontekstu: {err:#}"),
        }
    }
}
```

- [ ] **Step 4: Podepnij w `build_ui`**

Dopisz `mod context;` przy pozostałych deklaracjach modułów w `main.rs` (`main.rs:32`). Dalej, w `build_ui`:

1. Po utworzeniu `document`: `let context_sender = context::ContextSender::new(config.whisper.clone());`
2. W istniejącym `buffer.connect_changed` (`main.rs:876-895`), na końcu domknięcia:

```rust
        // Kontekst dla whispera idzie po każdej zmianie notatki — z daemona
        // i z klawiatury tak samo. Model ma widzieć wersję zatwierdzoną,
        // a nie własną pomyłkę sprzed minuty (docs/07).
        context_sender_for_buffer.schedule(&document_for_buffer);
```

   wraz z klonami `let context_sender_for_buffer = Rc::clone(&context_sender);` i `let document_for_buffer = Rc::clone(&document);` przed podpięciem sygnału.

3. Dołóż pole `context: Rc<context::ContextSender>` do struktury `Ui` (`main.rs:473-486`) i wypełnij je przy jej budowaniu.
4. W `Ui::attach` (`main.rs:553`), w rozpakowaniu struktury dopisz `context`, a zaraz po `subscribe_levels`:

```rust
        // Klient właśnie stanął — od tej chwili kontekst ma dokąd iść.
        context.set_client(Rc::clone(&client));
        context.send_now(&document);
```

5. W obsłudze przycisku nagrywania, po udanym `client.start()` (`main.rs:678`), wołaj `context.send_now(&document)` — pierwszy segment ma już mieć kontekst.

- [ ] **Step 5: Testy i uruchomienie**

Run: `cargo test -p diktate-gtk && cargo build --workspace`
Expected: PASS.

Ręcznie: uruchom `./target/debug/diktate-gtk`, wpisz w `~/.config/diktate/gtk.toml` szablon `prompt = "Terminy: ggml.\n{kontekst}"`, podyktuj dwa zdania i sprawdź w logu daemona wiersze `diktate-daemon: kontekst — N znaków`.

- [ ] **Step 6: Commit**

```bash
cargo clippy -p diktate-gtk --all-targets
git add crates/diktate-gtk/src/context.rs crates/diktate-gtk/src/main.rs
git commit -m "feat(gtk): składa prompt whispera i wysyła go po zmianie notatki

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

### Task 9: Jeden pisarz konfiguracji w `diktate-config`

**Files:**
- Create: `crates/diktate-config/src/render.rs` (przeniesione z `crates/diktate-cli/src/setup/write.rs:1-331`)
- Modify: `crates/diktate-config/src/lib.rs` (deklaracja modułu, metody `save`)
- Modify: `crates/diktate-cli/src/setup/write.rs` (zostaje cienką warstwą z własnymi testami)
- Test: testy przenoszą się razem z kodem; w `write.rs` zostają te, które sprawdzają kreator

**Interfaces:**
- Consumes: `GtkWhisperConfig`, `WhisperContextConfig` (zadanie 5).
- Produces:
  - `diktate_config::render::{daemon_toml, gtk_toml, save}`
  - `DaemonConfig::save(&self) -> Result<()>` i `GtkConfig::save(&self) -> Result<()>` — renderują i zapisują pod `Self::path()`

- [ ] **Step 1: Przenieś moduł**

```bash
git mv crates/diktate-cli/src/setup/write.rs crates/diktate-config/src/render.rs
```

W `crates/diktate-config/src/lib.rs` dopisz `pub mod render;`, a w `render.rs` zamień `use diktate_config::{DaemonConfig, GtkConfig};` na `use crate::{DaemonConfig, GtkConfig};`. Testy w `render.rs`, które sięgają po `crate::setup::diff`, przenieś z powrotem do `write.rs` (zadanie krok 3) — reszta zostaje.

- [ ] **Step 2: Dopisz renderowanie nowych sekcji**

W `daemon_toml`, za blokiem `[whisper]`:

```rust
    out.push_str("\n[whisper.context]\n");
    out.push_str(&number(
        "max_prompt_chars",
        cfg.whisper.context.max_prompt_chars as u32,
        "twardy limit długości promptu; nadmiar obcinany od początku",
    ));
    out.push_str(&flag(
        "repetition_guard",
        cfg.whisper.context.repetition_guard,
        "segment będący echem promptu licz ponownie bez kontekstu",
    ));
    out.push_str(&format!(
        "repetition_threshold = {}  # podobieństwo, od którego segment uchodzi za echo\n",
        cfg.whisper.context.repetition_threshold
    ));
```

W `gtk_toml`, przed sekcją `[correction]`:

```rust
    out.push_str("\n[whisper]\n");
    out.push_str(&text_block(
        "prompt",
        &cfg.whisper.prompt,
        "szablon promptu; {kontekst} to miejsce na tekst sprzed kursora",
    ));
    out.push_str(&number(
        "context_chars",
        cfg.whisper.context_chars as u32,
        "ile znaków przed kursorem wchodzi w {kontekst}",
    ));
```

Uwaga na kolejność: `[whisper]` jako tabela musi stać po kluczach prostych sekcji poprzedniej — wstaw ją jako osobną sekcję, nie w środku innej.

- [ ] **Step 3: Dodaj metody `save` i cienką warstwę w CLI**

W `lib.rs`, w `impl DaemonConfig` i `impl GtkConfig`:

```rust
    /// Zapisuje konfigurację pod `Self::path()`: render z komentarzami,
    /// kopia zapasowa, `rename`. Jedyna droga zapisu w całym projekcie —
    /// kreator i okno ustawień idą tą samą (`docs/04`).
    pub fn save(&self) -> Result<()> {
        render::save(&Self::path(), &render::gtk_toml(self))
    }
```

(dla `DaemonConfig` analogicznie z `render::daemon_toml`).

Nowy `crates/diktate-cli/src/setup/write.rs`:

```rust
//! Zapis konfiguracji z kreatora.
//!
//! Renderowanie i atomowy zapis mieszkają w `diktate-config`
//! (`render.rs`) — kreator i okno ustawień piszą tę samą treść i nie mogą
//! się rozjechać. Tutaj zostaje to, co jest sprawą kreatora.

pub use diktate_config::render::{daemon_toml, gtk_toml, save};
```

Przenieś tu z powrotem testy kreatora, które używają `crate::setup::diff`.

- [ ] **Step 4: Testy całego workspace'u**

Run: `cargo test --workspace`
Expected: PASS — w tym testy `zapisany_daemon_toml_da_się_wczytać_z_powrotem` i odpowiednik dla gtk, które teraz obejmują nowe sekcje.

- [ ] **Step 5: Sprawdź obieg w praktyce**

```bash
cargo run -p diktate-cli -- setup --help
```

Expected: kreator startuje, nic się nie zmieniło w jego zachowaniu.

- [ ] **Step 6: Commit**

```bash
cargo clippy --workspace --all-targets
git add crates/diktate-config crates/diktate-cli/src/setup/write.rs
git commit -m "refactor(config): jeden pisarz konfiguracji dla kreatora i GUI

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

### Task 10: Okno ustawień

**Files:**
- Create: `crates/diktate-gtk/src/settings.rs`
- Modify: `crates/diktate-gtk/src/main.rs` (deklaracja modułu)
- Test: `crates/diktate-gtk/src/settings.rs`

**Interfaces:**
- Consumes: `GtkConfig`, `GtkWhisperConfig`, `CorrectionConfig`, `GtkConfig::save` (zadania 5 i 9).
- Produces:
  - `pub struct Fields { pub prompt: String, pub context_chars: usize, pub correction_enabled: bool, pub endpoint: String, pub model: String, pub system_prompt: String, pub redaction_system_prompt: String, pub max_change_ratio_short: f32, pub max_change_ratio_long: f32, pub min_changed_words: usize }`
  - `pub fn fields_from(config: &GtkConfig) -> Fields`
  - `pub fn apply(config: &mut GtkConfig, fields: &Fields)`
  - `pub fn present(parent: &adw::ApplicationWindow, config: &GtkConfig, on_saved: impl Fn(GtkConfig) + 'static)` — otwiera `adw::PreferencesDialog`, a po zapisie woła `on_saved` z nową konfiguracją

- [ ] **Step 1: Napisz testy mapowania (bez widżetów)**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pola_biora_wartosci_z_konfiguracji() {
        let mut config = GtkConfig::default();
        config.whisper.prompt = "Terminy: ggml.\n{kontekst}".to_string();
        config.whisper.context_chars = 300;
        config.correction.model = "bielik".to_string();
        let fields = fields_from(&config);
        assert_eq!(fields.prompt, "Terminy: ggml.\n{kontekst}");
        assert_eq!(fields.context_chars, 300);
        assert_eq!(fields.model, "bielik");
    }

    #[test]
    fn zapis_pol_wraca_do_konfiguracji() {
        let mut config = GtkConfig::default();
        let fields = Fields {
            prompt: "nowy szablon {kontekst}".to_string(),
            context_chars: 120,
            correction_enabled: false,
            endpoint: "http://localhost:8080/v1/chat/completions".to_string(),
            model: "bielik-11b-v3".to_string(),
            system_prompt: "Jesteś korektorem.".to_string(),
            redaction_system_prompt: "Jesteś redaktorem.".to_string(),
            max_change_ratio_short: 0.4,
            max_change_ratio_long: 0.6,
            min_changed_words: 3,
        };
        apply(&mut config, &fields);
        assert_eq!(config.whisper.prompt, "nowy szablon {kontekst}");
        assert_eq!(config.whisper.context_chars, 120);
        assert!(!config.correction.enabled);
        assert_eq!(config.correction.model, "bielik-11b-v3");
        assert_eq!(config.correction.min_changed_words, 3);
    }

    #[test]
    fn mapowanie_w_obie_strony_nic_nie_gubi() {
        let mut config = GtkConfig::default();
        config.whisper.prompt = "szablon".to_string();
        let mut kopia = GtkConfig::default();
        apply(&mut kopia, &fields_from(&config));
        assert_eq!(kopia.whisper.prompt, config.whisper.prompt);
        assert_eq!(kopia.correction.system_prompt, config.correction.system_prompt);
    }

    // Pola, których okno nie pokazuje, mają przetrwać zapis — inaczej
    // ustawienia zjadłyby klucz API i limity czasu przy pierwszym otwarciu.
    #[test]
    fn pola_spoza_okna_zostaja_nietkniete() {
        let mut config = GtkConfig::default();
        config.correction.api_key = "tajne".to_string();
        config.correction.timeout_ms = 12_345;
        let fields = fields_from(&config);
        apply(&mut config, &fields);
        assert_eq!(config.correction.api_key, "tajne");
        assert_eq!(config.correction.timeout_ms, 12_345);
    }
}
```

- [ ] **Step 2: Uruchom testy**

Run: `cargo test -p diktate-gtk settings 2>&1 | head -20`
Expected: FAIL — brak modułu `settings`.

- [ ] **Step 3: Napisz moduł**

Nagłówek i mapowanie:

```rust
//! Okno ustawień.
//!
//! Pokrywa to, czego nie da się sensownie zrobić edytorem tekstu: szablon
//! promptu whispera, z którego użytkownik musi **widzieć**, gdzie wchodzi
//! kontekst, oraz ustawienia korekty LLM-em. Reszta konfiguracji zostaje
//! w plikach TOML (`docs/06#okno-ustawień`).
//!
//! Widok jest `adw::PreferencesDialog` — jedyny wyjątek od `ApplicationWindow`
//! + `ToolbarView` z konwencji widoków: ten typ niesie tamten układ w sobie
//! i jest wzorcem GNOME dla ustawień.
```

`Fields`, `fields_from`, `apply` przepisują wymienione pola jeden do jednego; `apply` dotyka **wyłącznie** pól z `Fields`, więc `api_key`, `temperature`, `max_tokens`, `timeout_ms` i szablony zostają nietknięte.

`present` buduje dialog:

- `adw::PreferencesDialog::new()`, `set_title("Ustawienia")`,
- strona „Transkrypcja”: `adw::PreferencesPage` z `icon_name("audio-input-microphone-symbolic")`,
  - `PreferencesGroup` „Prompt whispera”, `description("Szablon podpowiedzi dla modelu. {kontekst} to miejsce, w które wchodzi tekst sprzed kursora — bez tego symbolu prompt jest stały.")`, w środku `gtk4::TextView` z `WrapMode::WordChar` w `gtk4::Frame` z klasą CSS `card`, wysokość minimalna 8 wierszy (`set_size_request(-1, 160)`),
  - `PreferencesGroup` „Kontekst” z `adw::SpinRow::with_range(0.0, 1000.0, 50.0)`, tytuł „Znaków przed kursorem”,
  - `PreferencesGroup` „Pliki” z dwoma `adw::ActionRow` („Otwórz gtk.toml”, „Otwórz daemon.toml”), każdy z ikoną `document-open-symbolic` i `gtk4::gio::AppInfo::launch_default_for_uri` na `file://` ścieżki z `GtkConfig::path()` / `DaemonConfig::path()`,
- strona „Korekta”: `icon_name("text-editor-symbolic")`,
  - grupa „Serwer”: `adw::SwitchRow` „Korekta włączona”, `adw::EntryRow` „Adres”, `adw::EntryRow` „Model”,
  - grupa „Instrukcje”: dwa `TextView` w kartach — prompt korektora i prompt redaktora,
  - grupa „Progi”: `SpinRow` `max_change_ratio_short` i `max_change_ratio_long` (0.0–1.0, krok 0.05, `digits(2)`) oraz `min_changed_words` (0–20, krok 1),
- zapis przy zamknięciu: `dialog.connect_closed`, które zbiera `Fields` z widżetów, woła `apply` na kopii konfiguracji, `config.save()` i dopiero potem `on_saved(config)`. Nieudany zapis → `on_saved` **nie** jest wołane, a błąd idzie do logu i do `Toast` przez wywołującego (zadanie 11 przekazuje domknięcie, które go pokaże).

- [ ] **Step 4: Testy**

Run: `cargo test -p diktate-gtk`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
cargo clippy -p diktate-gtk --all-targets
git add crates/diktate-gtk/src/settings.rs crates/diktate-gtk/src/main.rs
git commit -m "feat(gtk): okno ustawień z szablonem promptu i korektą

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

### Task 11: Przycisk, skrót i odświeżenie po zapisie

**Files:**
- Modify: `crates/diktate-gtk/src/main.rs:755-802` (przyciski i `HeaderBar`), `:1042-1088` (skróty)

**Interfaces:**
- Consumes: `settings::present` (zadanie 10), `ContextSender::{set_config, send_now}` (zadanie 8).
- Produces: przycisk `settings_button` w pasku nagłówka i skrót `Ctrl+,`.

- [ ] **Step 1: Dodaj przycisk po prawej stronie mikrofonu**

```rust
    // Ustawienia stoją po lewej, zaraz za mikrofonem: prawa strona paska
    // jest zajęta przez sześć czynności i siódma już się tam nie mieści.
    let settings_button = gtk4::Button::builder()
        .icon_name("emblem-system-symbolic")
        .tooltip_text("Ustawienia (Ctrl+,)")
        .build();
```

```rust
    header.pack_start(&record_button);
    header.pack_start(&settings_button);
```

- [ ] **Step 2: Podepnij otwieranie okna**

```rust
    {
        let window_for_settings = window.clone();
        let config_for_settings = Rc::new(RefCell::new(config.clone()));
        let sender_for_settings = Rc::clone(&context_sender);
        let document_for_settings = Rc::clone(&document);
        let toast_for_settings = toast_overlay.clone();
        settings_button.connect_clicked(move |_| {
            let current = config_for_settings.borrow().clone();
            let config_slot = Rc::clone(&config_for_settings);
            let sender = Rc::clone(&sender_for_settings);
            let document = Rc::clone(&document_for_settings);
            let toast = toast_for_settings.clone();
            settings::present(&window_for_settings, &current, move |saved| {
                // Nowy szablon ma obowiązywać od następnej wypowiedzi, więc
                // kontekst leci do daemona od razu, bez czekania na zmianę
                // notatki i bez restartu czegokolwiek. Gdy klienta jeszcze
                // nie ma, `send_now` nic nie robi — i dobrze.
                sender.set_config(saved.whisper.clone());
                *config_slot.borrow_mut() = saved;
                sender.send_now(&document);
                toast.add_toast(Toast::new("Zapisano ustawienia"));
            });
        });
    }
```

`context_sender` jest w tym miejscu dostępne, bo powstaje w `build_ui` wcześniej (zadanie 8, krok 4). Klienta trzyma sam sender, więc przycisk ustawień nie musi o nim wiedzieć.

- [ ] **Step 3: Dodaj skrót `Ctrl+,`**

W `shortcuts.connect_key_pressed`, w `match key`, przed gałęzią `Escape`:

```rust
            // `Ctrl+,` to skrót ustawień w całym GNOME — bierzemy go
            // dokładnie w tej roli.
            gtk4::gdk::Key::comma if ctrl => {
                settings_button_for_keys.emit_clicked();
                gtk4::glib::Propagation::Stop
            }
```

wraz z `let settings_button_for_keys = settings_button.clone();` przed domknięciem.

- [ ] **Step 4: Sprawdź w działającej aplikacji**

```bash
cargo build --workspace
./target/debug/diktate-daemon --restart &
sleep 5 && ./target/debug/diktate-gtk
```

Sprawdź kolejno: przycisk stoi zaraz za mikrofonem; `Ctrl+,` otwiera okno; wpisanie szablonu z `{kontekst}` i zamknięcie okna daje `Toast` i wiersz `diktate-daemon: kontekst — N znaków` w logu daemona; `~/.config/diktate/gtk.toml` ma nową sekcję `[whisper]`, a `gtk.toml.bak` — poprzednią treść.

- [ ] **Step 5: Commit**

```bash
cargo clippy --workspace --all-targets
git add crates/diktate-gtk/src/main.rs
git commit -m "feat(gtk): przycisk ustawień obok mikrofonu i skrót Ctrl+,

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

### Task 12: Dokumentacja i domknięcie

**Files:**
- Modify: `docs/01-architektura.md:172-190`, `docs/02-protokol-ipc.md:65-90`, `docs/04-konfiguracja.md:61-82` i `:139-...` (`gtk.toml`), `docs/06-gui.md:118-128` (tabela skrótów), `:171-210` (konwencje), `:309-322` (okno ustawień), `docs/07-zachowanie-dyktowania.md:199-221`, `README.md:230-240`

**Interfaces:**
- Consumes: wszystko powyżej.
- Produces: dokumenty zgodne z kodem.

- [ ] **Step 1: `docs/01` — kontekst nadpisuje prompt**

W sekcji „Kontekst między segmentami” zamień zdanie o doklejaniu do `initial_prompt` na opis stanu faktycznego: prompt składa **klient** z szablonu i ogona dokumentu i przysyła go metodą `session.context`; `whisper.initial_prompt` obowiązuje tylko wtedy, gdy kontekstu nie ma. Wyjaśnij powód (to, co widać w oknie ustawień, jest tym, co dostaje model). Opis strażnika pętli zostaje, dochodzi wzmianka o drugim przebiegu z pustym promptem.

- [ ] **Step 2: `docs/02` — metoda przestaje być planem**

Uzupełnij sekcję „Kontekst transkrypcji”: metoda jest zaimplementowana, obcinanie do `chars` robi klient, daemon ma awaryjny `max_prompt_chars`. Dopisz akapit o `RequestParams` i o tym, dlaczego oba warianty mają `deny_unknown_fields`.

- [ ] **Step 3: `docs/04` — obie sekcje konfiguracji**

Dopisz `[whisper.context]` do przykładu `daemon.toml` (z trzema polami i komentarzami) oraz `[whisper]` do przykładu `gtk.toml`. Skreśl `[whisper.context]` z listy sekcji „z dalszych etapów”, których nie wolno wpisywać.

- [ ] **Step 4: `docs/06` — okno ustawień i skrót**

- W „Konwencjach widoków” dopisz zdanie: okno ustawień jest `adw::PreferencesDialog`, bo niesie układ `ToolbarView` + `HeaderBar` w sobie i jest wzorcem GNOME; reszta konwencji obowiązuje bez zmian.
- Sekcję „Okno ustawień” przepisz na stan faktyczny: dwie strony (transkrypcja, korekta), lista pól, zapis do `gtk.toml` z kopią `.bak`, przycisk po prawej stronie mikrofonu i powód tego miejsca. Zostaw jako niezrobione: urządzenie audio, modele, akcje głosowe.
- Dopisz `Ctrl+,` do tabeli skrótów.

- [ ] **Step 5: `docs/07` — kontekst jako stan faktyczny**

W „Kontekście dla whispera” zamień czas przyszły na teraźniejszy, dopisz, że ogon liczy się od początku zaznaczenia przy uzbrojonym zastąpieniu, że wysyłka jest odbita o 400 ms i pomijana przy niezmienionym prompcie, oraz że pusty szablon znaczy brak kontekstu.

- [ ] **Step 6: `README` — wzmianka o oknie ustawień**

W opisie `diktate-gtk` dopisz, że okno ma ustawienia promptu whispera i korekty (`Ctrl+,`).

- [ ] **Step 7: Pełne sprawdzenie**

```bash
cargo test --workspace
cargo clippy --workspace --all-targets
cargo build --workspace --release
```

Expected: wszystkie testy przechodzą, zero ostrzeżeń clippy, release się buduje.

Ręcznie, na koniec: podyktuj ten sam akapit z terminologią dwa razy — raz z pustym szablonem, raz z wypełnionym — i porównaj pisownię nazw własnych.

- [ ] **Step 8: Commit**

```bash
git add docs README.md
git commit -m "docs: kontekst whispera i okno ustawień jako stan faktyczny

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```
