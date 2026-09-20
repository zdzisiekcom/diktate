//! Konfiguracja: `daemon.toml` i `gtk.toml`.
//!
//! Zasada z `docs/04-konfiguracja.md`: **żadnej magicznej stałej w kodzie**.
//! Każdy próg, opóźnienie i limit ma swoje pole i udokumentowaną wartość
//! domyślną, a błąd walidacji jest odmową startu z czytelnym komunikatem —
//! nigdy cichym powrotem do domyślnych. Literówka w nazwie pola zatrzymuje
//! start (`deny_unknown_fields`), bo cicho pominięte pole to najgorszy
//! rodzaj błędu konfiguracji: wygląda jak działające.
//!
//! Każdy proces czyta **wyłącznie swój** plik. Agent na maszynie bez serwera
//! wyświetlania nie powinien nawet widzieć `gtk.toml`, a klient GUI nie ma
//! powodu ruszać progów VAD.

use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

/// Katalog konfiguracji: `$XDG_CONFIG_HOME/diktate`, z odwrotem na
/// `~/.config/diktate`.
pub fn config_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("XDG_CONFIG_HOME") {
        return PathBuf::from(dir).join("diktate");
    }
    let home = std::env::var_os("HOME").map(PathBuf::from);
    home.unwrap_or_else(|| PathBuf::from("."))
        .join(".config/diktate")
}

/// Wczytuje plik konfiguracyjny; brak pliku to wartości domyślne, a nie błąd.
fn read_or_default(path: &std::path::Path) -> Result<String> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(text),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(err) => Err(err).with_context(|| format!("nie udało się odczytać {}", path.display())),
    }
}

// --- daemon.toml ---

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct DaemonConfig {
    pub audio: AudioConfig,
    pub vad: VadConfig,
    pub transcription: TranscriptionConfig,
    pub whisper: WhisperConfig,
    pub parakeet: ParakeetConfig,
    pub queue: QueueConfig,
}

/// Który silnik zamienia dźwięk na tekst (`docs/12-backendy-transkrypcji.md`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct TranscriptionConfig {
    pub backend: Backend,
}

/// Silniki transkrypcji, między którymi wybiera `transcription.backend`.
///
/// Nazwa spoza listy zatrzymuje start — serde wypisze wtedy wartość winną
/// i wszystkie dopuszczalne, co jest lepszą diagnostyką niż cichy powrót do
/// whispera.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Backend {
    /// whisper.cpp w procesie daemona; tekst końcowy, wierny (WER ~5%).
    #[default]
    Whisper,
    /// Model Parakeet TDT, też w procesie daemona; szybki podgląd, ale myli
    /// treść (WER ~15%) i sam wykrywa język.
    Parakeet,
}

/// Model Parakeeta: skąd go wziąć i iloma wątkami liczyć. Minimum 3 s audio
/// nie jest tu wpisane, bo należy do modelu, a nie do konfiguracji — pyta
/// o nie warstwa wiązań (`dk_parakeet_min_audio_s`).
/// Obie wartości domyślne są „puste" i to jest ich treść: pusta ścieżka
/// znaczy „weź domyślną", a zero wątków — „wybierz sam". Stąd wyprowadzony
/// `Default`, a nie wypisany ręcznie.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct ParakeetConfig {
    /// Ścieżka modelu ggml; puste = `$DIKTATE_PARAKEET_MODEL` albo
    /// `~/.diktate/models/ggml-parakeet-tdt-0.6b-v3-q8_0.bin`.
    pub model: String,
    /// Liczba wątków; 0 znaczy „tyle, ile widzi system". Optimum wypada na
    /// liczbie rdzeni fizycznych — SMT szkodzi (docs/12).
    pub threads: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct AudioConfig {
    /// Nazwa urządzenia wejściowego; `"default"` to urządzenie systemowe.
    pub device: String,
    /// Ile ciszy sprzed wykrycia mowy dołączyć do segmentu — bez tego
    /// pierwsza sylaba bywa ucięta.
    pub pre_roll_ms: u32,
    /// Odstęp między odczytami poziomu wysyłanymi do wskaźnika.
    pub level_interval_ms: u32,
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            device: "default".to_string(),
            pre_roll_ms: 300,
            level_interval_ms: 50,
        }
    }
}

/// Progi segmentacji. Nazwy odpowiadają `SegmenterConfig` w `diktate-core` —
/// to jedne i te same pokrętła, opisane w `docs/04`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct VadConfig {
    /// Próg decyzyjny Silero (0.0–1.0).
    pub threshold: f32,
    /// Cisza domykająca wypowiedź krótszą niż `long_segment_ms`.
    pub silence_ms_to_end: u32,
    /// Cisza domykająca wypowiedź dłuższą niż `long_segment_ms` — krótka,
    /// żeby tekst płynął na ekran w trakcie mówienia.
    pub silence_ms_to_end_long: u32,
    /// Po jakiej długości wypowiedzi przełączyć się na krótszy próg ciszy.
    pub long_segment_ms: u32,
    /// Krótsze wypowiedzi odrzucamy jako stuknięcie albo szum.
    pub min_speech_ms: u32,
    /// Twardy limit długości segmentu.
    pub max_segment_ms: u32,
}

impl Default for VadConfig {
    fn default() -> Self {
        Self {
            threshold: 0.5,
            silence_ms_to_end: 1_500,
            silence_ms_to_end_long: 200,
            long_segment_ms: 2_500,
            min_speech_ms: 300,
            max_segment_ms: 15_000,
        }
    }
}

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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct WhisperConfig {
    /// Ścieżka modelu ggml; puste = `$DIKTATE_MODEL` albo ścieżka domyślna.
    pub model: String,
    /// Kod ISO języka albo `"auto"`.
    pub language: String,
    /// Podpowiedź kontekstowa doklejana do promptu (terminologia, nazwy).
    pub initial_prompt: String,
    /// Szerokość wiązki; 1 oznacza dekodowanie zachłanne.
    pub beam_size: u32,
    /// Liczba wątków; 0 = tyle, ile rdzeni.
    pub threads: u32,
    pub context: WhisperContextConfig,
}

impl Default for WhisperConfig {
    fn default() -> Self {
        Self {
            model: String::new(),
            language: "pl".to_string(),
            initial_prompt: String::new(),
            beam_size: 1,
            threads: 0,
            context: WhisperContextConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct QueueConfig {
    /// Ile segmentów może czekać na transkrypcję. Limit jest celowy: mowa
    /// bywa szybsza niż GPU, a kolejka bez dna zjadłaby pamięć i odsuwała
    /// tekst coraz dalej od chwili, w której padł.
    pub max_pending_segments: usize,
}

impl Default for QueueConfig {
    fn default() -> Self {
        Self {
            max_pending_segments: 16,
        }
    }
}

impl DaemonConfig {
    pub fn path() -> PathBuf {
        config_dir().join("daemon.toml")
    }

    /// Wczytuje `daemon.toml`. Brak pliku to wartości domyślne; plik
    /// niepoprawny to błąd, nie ciche domyślne.
    pub fn load() -> Result<Self> {
        let path = Self::path();
        let text = read_or_default(&path)?;
        Self::from_toml(&text).with_context(|| format!("w pliku {}", path.display()))
    }

    pub fn from_toml(text: &str) -> Result<Self> {
        let config: Self = toml::from_str(text).context("niepoprawny daemon.toml")?;
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<()> {
        if !(0.0..=1.0).contains(&self.vad.threshold) {
            bail!(
                "vad.threshold musi mieścić się w 0.0–1.0, a jest {}",
                self.vad.threshold
            );
        }
        if self.vad.min_speech_ms >= self.vad.max_segment_ms {
            bail!(
                "vad.min_speech_ms ({}) musi być mniejsze od vad.max_segment_ms ({})",
                self.vad.min_speech_ms,
                self.vad.max_segment_ms
            );
        }
        if self.vad.max_segment_ms > WHISPER_WINDOW_MS {
            bail!(
                "vad.max_segment_ms ({}) przekracza okno whispera ({WHISPER_WINDOW_MS} ms) — \
                 dłuższy segment i tak zostałby obcięty, tyle że po cichu",
                self.vad.max_segment_ms
            );
        }
        if self.vad.silence_ms_to_end_long > self.vad.silence_ms_to_end {
            bail!(
                "vad.silence_ms_to_end_long ({}) musi być krótsze niż vad.silence_ms_to_end ({}) \
                 — adaptacyjny próg ma rytm skracać, nie wydłużać",
                self.vad.silence_ms_to_end_long,
                self.vad.silence_ms_to_end
            );
        }
        if self.vad.long_segment_ms >= self.vad.max_segment_ms {
            bail!(
                "vad.long_segment_ms ({}) musi być mniejsze od vad.max_segment_ms ({}) — \
                 inaczej krótszy próg ciszy nigdy by się nie włączył",
                self.vad.long_segment_ms,
                self.vad.max_segment_ms
            );
        }
        if self.audio.level_interval_ms == 0 {
            bail!("audio.level_interval_ms musi być większe od zera");
        }
        if self.queue.max_pending_segments == 0 {
            bail!("queue.max_pending_segments musi być większe od zera");
        }
        // Sekcja `[parakeet]` nie ma nic do sprawdzenia: pusty `model`
        // znaczy „ścieżka domyślna", a zerowe `threads` — „tyle, ile widzi
        // system". Istnienie pliku modelu rozstrzyga się przy starcie
        // daemona, tak samo jak przy whisperze, bo to stan dysku, a nie
        // poprawność konfiguracji.
        Ok(())
    }
}

/// Whisper przetwarza zawsze okno 30 sekund — to liczba, o którą opiera się
/// cała segmentacja (`CLAUDE.md`, `docs/07`).
const WHISPER_WINDOW_MS: u32 = 30_000;

// --- gtk.toml ---

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

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct GtkConfig {
    pub window: WindowConfig,
    pub daemon: GtkDaemonConfig,
    pub dictation: DictationConfig,
    pub shortcuts: ShortcutsConfig,
    pub tray: TrayConfig,
    pub correction: CorrectionConfig,
    pub whisper: GtkWhisperConfig,
}

/// Co klient GUI robi, gdy daemon nie działa.
///
/// Domyślnie uruchamia go sam: z punktu widzenia użytkownika Diktate jest
/// jedną aplikacją, a nie usługą, którą trzeba pamiętać, żeby włączyć.
/// Wyłączenie `autostart` zostawia decyzję systemowi — sensowne tam, gdzie
/// daemon chodzi z `systemd --user`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct GtkDaemonConfig {
    pub autostart: bool,
    /// Ile czekać na gniazdo po uruchomieniu daemona. Czas liczy się od
    /// startu procesu i musi pomieścić wczytanie modelu whispera — to
    /// sekundy, nie milisekundy.
    pub start_timeout_ms: u64,
}

impl Default for GtkDaemonConfig {
    fn default() -> Self {
        Self {
            autostart: true,
            start_timeout_ms: 30_000,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct WindowConfig {
    pub width: i32,
    pub height: i32,
    /// Czy pokazywać wskaźnik poziomu sygnału. Jego funkcja jest
    /// diagnostyczna: brak wychyleń od razu mówi, że mikrofon jest nie ten.
    pub show_level_meter: bool,
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self {
            width: 620,
            height: 420,
            show_level_meter: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct DictationConfig {
    /// Segment wchodzi w miejscu karetki, nie na końcu notatki.
    pub insert_at_cursor: bool,
    /// Spacja rozdzielająca na stykach wypowiedzi.
    pub auto_space: bool,
    /// Usuwanie wielokropka kończącego wypowiedź przerwaną pauzą na oddech.
    pub strip_trailing_ellipsis: bool,
    /// Przewijanie do punktu wstawiania — tylko gdy ten był widoczny.
    pub follow_caret: bool,
}

impl Default for DictationConfig {
    fn default() -> Self {
        Self {
            insert_at_cursor: true,
            auto_space: true,
            strip_trailing_ellipsis: true,
            follow_caret: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct ShortcutsConfig {
    pub copy: String,
    pub undo: String,
    pub redo: String,
    /// Przełączenie nagrywania **wewnątrz okna**. Skrót globalny rejestruje
    /// środowisko graficzne i nie ma go w tym pliku (`docs/06`).
    pub toggle_recording: String,
}

impl Default for ShortcutsConfig {
    fn default() -> Self {
        Self {
            copy: "<Control>c".to_string(),
            undo: "<Control>z".to_string(),
            redo: "<Control><Shift>z".to_string(),
            toggle_recording: "<Control>space".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct TrayConfig {
    pub enabled: bool,
    /// Licznik zaległości na ikonie.
    pub show_backlog: bool,
}

impl Default for TrayConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            show_backlog: true,
        }
    }
}

/// Korekta tekstu lokalnym modelem językowym (`docs/11-korekta-llm.md`).
///
/// Wszystko, co dotyczy modelu, jest tutaj — łącznie z promptem. Prompt jest
/// polem konfiguracji, a nie stałą w kodzie, bo to **on** decyduje o całym
/// zachowaniu korekty: ten sam model z promptem ostrożnym nie naprawia nawet
/// oczywistych błędów, a z promptem pokazującym przykłady napraw działa
/// (`docs/11#prompt-decyduje-o-wszystkim`). Dobieranie promptu jest więc
/// zwykłym strojeniem, nie zmianą programu.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct CorrectionConfig {
    /// Czy przycisk korekty w ogóle jest w oknie. Wyłączony ukrywa go —
    /// nie zostawiamy guzika, który zawsze kończy się błędem sieci.
    pub enabled: bool,
    /// Pełny adres punktu końcowego zgodnego z API OpenAI.
    pub endpoint: String,
    /// Nazwa modelu przekazywana w żądaniu. Serwer bywa przestawiany na inny
    /// model, więc to pole trzeba trzymać w zgodzie z `/v1/models`.
    pub model: String,
    /// Klucz API; puste = nagłówek `Authorization` nie idzie wcale.
    pub api_key: String,
    pub temperature: f32,
    pub max_tokens: u32,
    /// Limit czasu całego żądania. Akapit potrafi zająć kilka sekund,
    /// a model po podmianie na większy — wielokrotnie więcej.
    pub timeout_ms: u64,
    /// Wycisza rozumowanie u modeli, które je mają (`enable_thinking: false`
    /// oraz `reasoning_effort: "none"`). Bielik je ignoruje, ale model
    /// z domyślnie włączonym thinkingiem odpowiada bez tego 5–45× wolniej.
    pub disable_thinking: bool,
    /// Prompt systemowy — instrukcja korektora.
    pub system_prompt: String,
    /// Szablon wiadomości użytkownika; `{text}` zastępuje tekst notatki.
    pub user_template: String,
    /// Prompt systemowy redakcji poleceniem głosowym — instrukcja redaktora.
    ///
    /// Jest przeciwieństwem [`Self::system_prompt`]: korektor ma **zakaz**
    /// zmiany stylu, a redaktor robi dokładnie to, o co poprosił użytkownik.
    /// Zostają za to zakazy, które chronią treść: nie dopisuj od siebie, nie
    /// ruszaj liczb i nazw własnych, oddaj sam tekst bez komentarza.
    pub redaction_system_prompt: String,
    /// Szablon wiadomości użytkownika przy redakcji. `{instruction}` zastępuje
    /// polecenie podyktowane przez użytkownika, `{text}` — tekst celu
    /// (zaznaczenie albo całą notatkę). Oba pola są obowiązkowe.
    pub redaction_user_template: String,
    /// Do tylu słów notatka jest „krótka" i obowiązuje ostrzejszy próg zmian.
    pub short_text_words: usize,
    /// Próg odsetka zmienionych słów dla krótkiej notatki — **luźniejszy**
    /// niż dla długiej. W zdaniu z ośmiu słów dwie naprawy to już 25%, choć
    /// jest to dokładnie ta robota, po którą sięga się po korektę; do tego
    /// krótką notatkę użytkownik ogarnia wzrokiem i sam widzi, co się
    /// zmieniło. Przekroczenie nie odrzuca korekty po cichu — pyta (`docs/11`).
    pub max_change_ratio_short: f32,
    /// Próg odsetka zmienionych słów dla dłuższej notatki — tu leży prawdziwe
    /// ryzyko. Model puszczony na akapit zmienia 33–37% słów i oddaje tekst,
    /// który czyta się lepiej, ale nie jest już wypowiedzią użytkownika,
    /// a sprawdzić tego wzrokiem już się nie da. Próg jest mimo to luźny
    /// (0,5), bo korektor ma dziś usuwać wypełniacze, jąkanie i fałszywe
    /// starty — a to znaczy, że uczciwa korekta dyktowanego akapitu **musi**
    /// zmienić dużo słów. Alarm przy 0,35 odpalałby się przy każdej dobrze
    /// wykonanej robocie i przestałby cokolwiek znaczyć.
    pub max_change_ratio_long: f32,
    /// Ile zmienionych słów przechodzi zawsze, bez pytania — niezależnie od
    /// odsetka. W notatce z sześciu słów jedna uczciwa naprawa to już 17%
    /// i sam odsetek alarmowałby przy każdej poprawnie wykonanej korekcie.
    pub min_changed_words: usize,
    /// Znaki dopuszczone w odpowiedzi ponad alfabet polski, cyfry
    /// i interpunkcję. Walidacja znakowa wyłapuje model, który wstawia
    /// w środek słów znaki spoza alfabetu — robił tak testowany `ornith`.
    pub allowed_extra_chars: String,
}

/// Domyślny prompt korektora. Pochodzi wprost z pomiaru w `docs/11`: wersja
/// bez przykładów napraw dawała model bierny, który nie ruszał nawet
/// `ten wis per`. Drugie źródło jego kształtu to natura wejścia — to zapis
/// **mowy**, a nie pisma, więc poza błędami rozpoznania trzeba z niego zdjąć
/// wypełniacze, jąkanie i fałszywe starty. Dlatego prompt wylicza je z
/// przykładami, a zakaz skracania mówi wprost, że ich usunięcie skracaniem
/// nie jest — inaczej model traktuje je jak treść i zostawia nietknięte.
const DEFAULT_CORRECTION_PROMPT: &str = "\
Poprawiasz transkrypcję MOWY DYKTOWANEJ. Ktoś mówił to na głos, więc tekst ma \
wady mowy, nie pisma: powtórzenia, jąkanie, fałszywe starty, poprawianie się \
w pół zdania, wypełniacze. Oddaj to, co mówca chciał napisać, a nie zapis \
tego, jak mówił.
System rozpoznawania mowy dodatkowo ROZBIJA jedno słowo na kilka albo myli \
słowa podobnie brzmiące — odtwórz je:
  'ten wis per' → 'ten Whisper'
  'w kontener że' → 'w kontenerze'
  'morze tam być' → 'może tam być'
  'na gita bie' → 'na GitHubie'
Usuwaj stanowczo:
  wtrącenia i wypełniacze (eee, yyy, mmm, yhm, no, jakby, tak jakby, w sumie, \
generalnie, wiesz, prawda) oraz znaczniki typu [muzyka], [śmiech];
  jąkanie i powtórzone słowa: 'to to to jest' → 'to jest';
  fałszywe starty i autokorekty mówcy — zostaje wersja poprawiona: \
'zrobimy to jutro, znaczy pojutrze' → 'zrobimy to pojutrze', \
'ten plik, ten no, ten konfiguracyjny' → 'ten plik konfiguracyjny';
  tę samą myśl powiedzianą dwa razy pod rząd — zostaje jedno sformułowanie.
Wstawiaj interpunkcję i wielkie litery, dziel na zdania tam, gdzie mówca \
sklejał je w jedno długie.
ZAKAZY: nie dodawaj treści, której nie ma, nie streszczaj i nie gub żadnej \
myśli (usuwanie wypełniaczy i powtórek skracaniem nie jest), nie zmieniaj \
liczb, nazw własnych ani technicznych, nie podmieniaj słów mówcy na \
ładniejsze synonimy, nie przestawiaj kolejności zdań. Fragment już poprawny \
zostaw nietknięty.
Odpowiadasz wyłącznie poprawionym tekstem.";

/// Domyślny prompt redaktora. Redakcja odwraca umowę korekty: instrukcją jest
/// polecenie użytkownika, więc prompt nie może go przykrywać własnymi
/// zaleceniami co do stylu. Zostają zakazy chroniące treść — model, który
/// „przy okazji" dopisze zdanie albo poprawi liczbę, robi dokładnie tę szkodę,
/// przed którą redakcja ma bronić (`docs/11`).
const DEFAULT_REDACTION_PROMPT: &str = "\
Redagujesz tekst według polecenia użytkownika. Polecenie jest jedyną \
instrukcją, co zmienić — wykonaj je wiernie, także gdy każe zmienić styl, \
skrócić albo przepisać wypowiedź.
ZAKAZY: nie dodawaj treści, o którą nie proszono, nie zmieniaj liczb, nazw \
własnych ani nazw technicznych, nie komentuj tego, co zrobiłeś, nie pytaj \
o nic. Fragmentów, których polecenie nie dotyczy, nie ruszaj.
Odpowiadasz wyłącznie zredagowanym tekstem.";

impl Default for CorrectionConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            endpoint: "http://home-host.tarpan-flops.ts.net:8080/v1/chat/completions".to_string(),
            model: "bielik-11b-v3".to_string(),
            api_key: String::new(),
            temperature: 0.0,
            max_tokens: 2_000,
            timeout_ms: 120_000,
            disable_thinking: true,
            system_prompt: DEFAULT_CORRECTION_PROMPT.to_string(),
            user_template: "Popraw:\n\n{text}".to_string(),
            redaction_system_prompt: DEFAULT_REDACTION_PROMPT.to_string(),
            redaction_user_template: "Polecenie: {instruction}\n\nTekst:\n\n{text}".to_string(),
            short_text_words: 40,
            max_change_ratio_short: 0.65,
            max_change_ratio_long: 0.5,
            min_changed_words: 2,
            allowed_extra_chars: String::new(),
        }
    }
}

impl GtkConfig {
    pub fn path() -> PathBuf {
        config_dir().join("gtk.toml")
    }

    pub fn load() -> Result<Self> {
        let path = Self::path();
        let text = read_or_default(&path)?;
        Self::from_toml(&text).with_context(|| format!("w pliku {}", path.display()))
    }

    pub fn from_toml(text: &str) -> Result<Self> {
        let config: Self = toml::from_str(text).context("niepoprawny gtk.toml")?;
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<()> {
        if self.window.width <= 0 {
            bail!(
                "window.width musi być dodatnie, a jest {}",
                self.window.width
            );
        }
        if self.window.height <= 0 {
            bail!(
                "window.height musi być dodatnie, a jest {}",
                self.window.height
            );
        }
        for (nazwa, skrót) in [
            ("shortcuts.copy", &self.shortcuts.copy),
            ("shortcuts.undo", &self.shortcuts.undo),
            ("shortcuts.redo", &self.shortcuts.redo),
            (
                "shortcuts.toggle_recording",
                &self.shortcuts.toggle_recording,
            ),
        ] {
            if skrót.trim().is_empty() {
                bail!("{nazwa} jest pusty — nie ma czego związać z akcją");
            }
        }
        self.correction.validate()?;
        Ok(())
    }
}

impl CorrectionConfig {
    /// Próg zmian obowiązujący dla notatki o tylu słowach.
    pub fn max_change_ratio(&self, words: usize) -> f32 {
        if words <= self.short_text_words {
            self.max_change_ratio_short
        } else {
            self.max_change_ratio_long
        }
    }

    fn validate(&self) -> Result<()> {
        // Pola sprawdzamy także przy `enabled = false`: literówka w liczbie
        // ma się odezwać wtedy, gdy ktoś ją wpisuje, a nie pół roku później
        // przy włączaniu korekty.
        if !(0.0..=2.0).contains(&self.temperature) {
            bail!(
                "correction.temperature musi mieścić się w 0.0–2.0, a jest {}",
                self.temperature
            );
        }
        if self.max_tokens == 0 {
            bail!("correction.max_tokens musi być większe od zera");
        }
        if self.timeout_ms == 0 {
            bail!("correction.timeout_ms musi być większe od zera");
        }
        for (nazwa, próg) in [
            (
                "correction.max_change_ratio_short",
                self.max_change_ratio_short,
            ),
            (
                "correction.max_change_ratio_long",
                self.max_change_ratio_long,
            ),
        ] {
            if !(0.0..=1.0).contains(&próg) {
                bail!("{nazwa} to odsetek — musi mieścić się w 0.0–1.0, a jest {próg}");
            }
        }
        if !self.user_template.contains("{text}") {
            bail!(
                "correction.user_template musi zawierać {{text}} — inaczej model \
                 nie dostanie tekstu do poprawienia"
            );
        }
        for pole in ["{instruction}", "{text}"] {
            if !self.redaction_user_template.contains(pole) {
                bail!(
                    "correction.redaction_user_template musi zawierać {pole} — \
                     inaczej model nie dostanie {}",
                    if pole == "{instruction}" {
                        "polecenia użytkownika"
                    } else {
                        "tekstu do zredagowania"
                    }
                );
            }
        }
        if !self.enabled {
            return Ok(());
        }
        if self.endpoint.trim().is_empty() {
            bail!("correction.endpoint jest pusty — nie ma dokąd wysłać tekstu");
        }
        if self.model.trim().is_empty() {
            bail!("correction.model jest pusty — serwer nie wie, czym poprawiać");
        }
        if self.system_prompt.trim().is_empty() {
            bail!(
                "correction.system_prompt jest pusty — bez instrukcji korektora \
                 model redaguje zamiast poprawiać"
            );
        }
        if self.redaction_system_prompt.trim().is_empty() {
            bail!(
                "correction.redaction_system_prompt jest pusty — bez instrukcji \
                 redaktora model komentuje polecenie zamiast je wykonać"
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
