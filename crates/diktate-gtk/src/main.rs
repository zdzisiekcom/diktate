//! Okno dyktowania.
//!
//! Jedno okno: przycisk przełączający nagrywanie, `TextView` na tekst,
//! przyciski czyszczenia, korekty tekstu modelem językowym (patrz
//! [`correct`] i `docs/11-korekta-llm.md`) i redakcji poleceniem
//! podyktowanym głosem ([`redact`]) oraz dwa przyciski wyjścia —
//! „Kopiuj" i „OK". „OK" (skrót lokalny
//! `Super+C`) domyka dyktowanie jednym ruchem: zatrzymuje nagrywanie,
//! kopiuje całość do schowka i zamyka okno — ale dopiero, gdy kolejka
//! transkrypcji ucichnie, żeby nie zjeść ostatniego zdania; patrz
//! [`Accept`]. Tekst nie jest dopisywany na koniec bufora, tylko
//! wstawiany **w kotwicę wstawiania** modelu dokumentu — patrz [`view`]
//! i `docs/07-zachowanie-dyktowania.md`. Dopisanie na koniec jest tylko
//! przypadkiem szczególnym tej jednej ścieżki.
//!
//! Warstwa widoku zbudowana jest na libadwaicie, żeby okno wyglądało i
//! zachowywało się jak natywna aplikacja GNOME: pasek nagłówka zintegrowany
//! z treścią (bez osobnego paska tytułowego), podążanie za systemowym
//! schematem kolorów oraz `StatusPage` widoczna, dopóki bufor jest pusty.
//!
//! **Stan sesji w podtytule.** Podtytuł `WindowTitle` jest jedynym miejscem,
//! gdzie pokazujemy stan — zgodnie z konwencją widoków w `docs/06-gui.md` nie
//! ma osobnej etykiety statusu pod treścią. Źródłem prawdy o stanie jest
//! wyłącznie powiadomienie `state.changed` z daemona (`Incoming::State`) —
//! kliknięcie przycisku nagrywania nie zgaduje tekstu podtytułu, tylko czeka
//! na potwierdzenie. Ikona mikrofonu pulsuje w trakcie nagrywania, a `Spinner`
//! obok tytułu pokazuje się, dopóki jest cokolwiek w kolejce transkrypcji.
//! Podtytuł składa [`Subtitle`] z trzech niezależnych źródeł — stanu sesji,
//! uzbrojonego zastąpienia i trwającej korekty — bo każde z nich zmienia się
//! samo i żadne nie może zamazać pozostałych.

mod client;
mod correct;
mod daemon;
mod meter;
mod redact;
mod shortcut;
mod tray;
mod view;

use client::{Client, Incoming};
use diktate_config::GtkConfig;
use diktate_proto::SessionState;
use gtk4::{
    LevelBar, PolicyType, ScrolledWindow, Spinner, Stack, TextView, ToggleButton, WrapMode,
};
use libadwaita::prelude::*;
use libadwaita::{
    Application, ApplicationWindow, Clamp, HeaderBar, StatusPage, Toast, ToastOverlay, ToolbarView,
    WindowTitle,
};
use meter::meter_value;
use std::cell::Cell;
use std::rc::Rc;
use tray::{DiktateTray, TrayCommand, TrayHandle};
use view::DocumentView;

const APP_ID: &str = "dev.diktate.Gtk";

/// Nazwy stron w `Stack`-u treści: pusty bufor pokazuje zachętę do
/// dyktowania, niepusty — widok tekstu.
const PAGE_EMPTY: &str = "empty";
const PAGE_TEXT: &str = "text";

/// Akcja aplikacji wołana przez skrót globalny: przywołuje okno i przełącza
/// nagrywanie. Nazwa jest częścią umowy z `--toggle`.
const ACTION_TOGGLE: &str = "toggle-recording";

/// Akcja domykająca dyktowanie: kopiuje tekst do schowka, zatrzymuje
/// nagrywanie i zamyka okno. To ona stoi za przyciskiem „OK".
const ACTION_ACCEPT: &str = "accept";

/// Skrót akcji „OK" — działa wyłącznie w oknie aplikacji, nie globalnie.
const ACCEPT_ACCEL: &str = "<Super>c";

/// Klasa widoku tekstu podnoszona na czas uzbrojonego zastąpienia.
const REPLACE_ARMED_CLASS: &str = "replace-armed";

/// Jedyny własny styl okna: zaznaczenie czekające na zastąpienie ma wyglądać
/// inaczej niż zwykłe. Barwy nazwane pochodzą z Adwaity i idą za motywem.
const REPLACE_CSS: &str = "textview.replace-armed text selection {\n    background-color: @warning_bg_color;\n    color: @warning_fg_color;\n}\n";

fn main() -> gtk4::glib::ExitCode {
    // Polecenia administracyjne obsługujemy przed uruchomieniem GTK —
    // rejestracja skrótu nie potrzebuje okna ani serwera wyświetlania.
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--install-shortcut") => {
            let keys = args.get(1).map(String::as_str).unwrap_or(DEFAULT_SHORTCUT);
            return match shortcut::install(keys) {
                Ok(()) => gtk4::glib::ExitCode::SUCCESS,
                Err(err) => {
                    eprintln!("diktate-gtk: nie udało się zarejestrować skrótu: {err:#}");
                    gtk4::glib::ExitCode::FAILURE
                }
            };
        }
        Some("--remove-shortcut") => {
            return match shortcut::remove() {
                Ok(()) => gtk4::glib::ExitCode::SUCCESS,
                Err(err) => {
                    eprintln!("diktate-gtk: nie udało się usunąć skrótu: {err:#}");
                    gtk4::glib::ExitCode::FAILURE
                }
            };
        }
        Some("--help" | "-h") => {
            print_help();
            return gtk4::glib::ExitCode::SUCCESS;
        }
        _ => {}
    }

    // Aplikacja libadwaity sama inicjalizuje bibliotekę i podłącza się pod
    // `adw::StyleManager` z domyślnym `ColorScheme::Default`, czyli
    // podążaniem za systemowym jasnym/ciemnym motywem — nic tu nie trzeba
    // wymuszać.
    //
    // `HANDLES_COMMAND_LINE` sprawia, że drugie uruchomienie nie tworzy
    // drugiego okna, tylko przekazuje argumenty do już działającej
    // instancji. Na tym stoi skrót globalny: `diktate-gtk --toggle`
    // wywołane z pustego biurka podnosi istniejące okno i włącza mikrofon,
    // zamiast otwierać kolejną kopię aplikacji.
    let app = Application::builder()
        .application_id(APP_ID)
        .flags(gtk4::gio::ApplicationFlags::HANDLES_COMMAND_LINE)
        .build();

    app.connect_command_line(|app, cmdline| {
        let toggle = cmdline
            .arguments()
            .iter()
            .any(|arg| arg.to_string_lossy() == "--toggle");

        if app.windows().is_empty() {
            build_ui(app);
        }
        if let Some(window) = app.active_window() {
            window.present();
        }
        if toggle {
            if let Some(action) = app.lookup_action(ACTION_TOGGLE) {
                action.activate(None);
            }
        }
        0
    });
    app.run()
}

/// Domyślny skrót proponowany przez `--install-shortcut` bez argumentu.
const DEFAULT_SHORTCUT: &str = "<Control><Alt>d";

fn print_help() {
    println!(
        "Diktate — okno dyktowania.

Użycie:
  diktate-gtk                      otwiera okno
  diktate-gtk --toggle             przywołuje okno i przełącza nagrywanie
  diktate-gtk --install-shortcut [SKRÓT]
                                   rejestruje skrót globalny w GNOME
                                   (domyślnie {DEFAULT_SHORTCUT})
  diktate-gtk --remove-shortcut    usuwa zarejestrowany skrót
  diktate-gtk --help               ten opis

W oknie: Super+C kopiuje tekst, zatrzymuje nagrywanie i zamyka okno.

W środowiskach spoza GNOME zwiąż własnym narzędziem polecenie
`diktate-gtk --toggle`."
    );
}

/// Forma liczebnika odpowiadająca polskiej gramatyce dla rzeczowników
/// policzalnych: pojedyncza dla 1, „kilka" dla końcówek 2–4 (poza 12–14),
/// „wiele" dla reszty (dopełniacz liczby mnogiej).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PluralForm {
    One,
    Few,
    Many,
}

fn polish_plural_form(n: u32) -> PluralForm {
    if n == 1 {
        return PluralForm::One;
    }
    let last_digit = n % 10;
    let last_two_digits = n % 100;
    if (2..=4).contains(&last_digit) && !(12..=14).contains(&last_two_digits) {
        PluralForm::Few
    } else {
        PluralForm::Many
    }
}

/// Odmiana słowa „wypowiedź" dla danej liczby. Formy „kilka" i „wiele"
/// pokrywają się dla tego rzeczownika („2 wypowiedzi", „5 wypowiedzi",
/// „22 wypowiedzi") — jedyną formą różną jest liczba pojedyncza.
fn wypowiedzi_word(n: u32) -> &'static str {
    match polish_plural_form(n) {
        PluralForm::One => "wypowiedź",
        PluralForm::Few | PluralForm::Many => "wypowiedzi",
    }
}

fn backlog_phrase(n: u32) -> String {
    format!("{n} {}", wypowiedzi_word(n))
}

/// Podtytuł składany z trzech niezależnych rzeczy: stanu sesji, uzbrojonego
/// zastąpienia i czynności zajmującej okno (korekta albo redakcja). Stany chwilowe idą na początek, bo to je
/// użytkownik ma zobaczyć — stan sesji jest tłem, które i tak widzi ikoną
/// mikrofonu.
///
/// Sam stan sesji nigdy nie jest zgadywany z kliknięcia przycisku: bierze się
/// wyłącznie z powiadomienia `state.changed` (`docs/06`).
fn compose_subtitle(state: SessionState, backlog: u32, armed: bool, busy: Option<&str>) -> String {
    let mut out = subtitle_for_state(state, backlog);
    if armed {
        out = format!("Zastępuję… · {out}");
    }
    if let Some(czynność) = busy {
        out = format!("{czynność} · {out}");
    }
    out
}

/// Nazwy czynności zajmujących okno. Jedno gniazdo, bo korekta i redakcja
/// wykluczają się nawzajem — obie chodzą do tego samego modelu i obie
/// podmieniają tekst, więc druga w trakcie pierwszej nie miałaby czego
/// poprawiać.
const BUSY_CORRECTING: &str = "Poprawiam tekst…";
const BUSY_REDACTING: &str = "Redaguję tekst…";

/// Podtytuł okna wraz ze wszystkim, z czego się składa.
///
/// Trzy niezależne źródła piszą w to samo miejsce, więc żadne z nich nie może
/// znać całego tekstu — każde zmienia swoją część i prosi o przerysowanie.
/// Bez tego korekta zamazywałaby „zastępuję…”, a stan z daemona zamazywałby
/// jedno i drugie.
struct Subtitle {
    title: WindowTitle,
    session: Cell<(SessionState, u32)>,
    armed: Cell<bool>,
    busy: Cell<Option<&'static str>>,
}

impl Subtitle {
    fn new(title: WindowTitle) -> Self {
        Self {
            title,
            session: Cell::new((SessionState::Idle, 0)),
            armed: Cell::new(false),
            busy: Cell::new(None),
        }
    }

    fn session(&self) -> (SessionState, u32) {
        self.session.get()
    }

    fn set_session(&self, state: SessionState, backlog: u32) {
        self.session.set((state, backlog));
        self.refresh();
    }

    fn set_armed(&self, armed: bool) {
        self.armed.set(armed);
        self.refresh();
    }

    /// Ogłasza czynność zajmującą okno — albo jej koniec (`None`).
    fn set_busy(&self, busy: Option<&'static str>) {
        self.busy.set(busy);
        self.refresh();
    }

    /// Komunikat, który wypiera złożony podtytuł do czasu następnej zmiany
    /// stanu — brak daemona, zerwane połączenie, błąd `session.start`.
    fn say(&self, text: &str) {
        self.title.set_subtitle(text);
    }

    fn refresh(&self) {
        let (state, backlog) = self.session.get();
        self.title.set_subtitle(&compose_subtitle(
            state,
            backlog,
            self.armed.get(),
            self.busy.get(),
        ));
    }
}

fn subtitle_for_state(state: SessionState, backlog: u32) -> String {
    match (state, backlog) {
        (SessionState::Idle, 0) => "Gotowe".to_string(),
        (SessionState::Idle, n) => format!("Przetwarzam {}…", backlog_phrase(n)),
        (SessionState::Recording, 0) => "Nagrywam".to_string(),
        (SessionState::Recording, n) => format!("Nagrywam · przetwarzam {}", backlog_phrase(n)),
    }
}

/// Stan połączenia z daemonem widziany przez przycisk nagrywania.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Link {
    #[default]
    Connecting,
    Spawning,
    Ready,
    Lost,
}

/// Bramka między żądaniem nagrywania a klientem.
///
/// Skrót globalny potrafi wystartować całą aplikację: akcja `toggle-recording`
/// odpala się wtedy **przed** połączeniem z daemonem, a bywa że i przed jego
/// uruchomieniem. Przełączanie wtedy samego przycisku dawało stan, w którym
/// przycisk wygląda na wciśnięty, `session.start` nigdy nie poszedł, a
/// użytkownik musiał nacisnąć dwa razy, żeby cokolwiek nagrać. Bramka takie
/// żądanie **odkłada** i wykonuje je dopiero, gdy jest z kim rozmawiać —
/// pełną drogą, przez ten sam przycisk, więc razem z ikoną nagrywania,
/// wskaźnikiem poziomu i podtytułem.
#[derive(Default)]
struct Gate {
    link: Cell<Link>,
    pending: Cell<bool>,
}

impl Gate {
    fn link(&self) -> Link {
        self.link.get()
    }

    fn pending(&self) -> bool {
        self.pending.get()
    }

    /// Żądanie przełączenia nagrywania. `true` = wykonaj je teraz na
    /// przycisku, `false` = odłożone albo nie ma czego odkładać.
    fn request(&self) -> bool {
        match self.link.get() {
            Link::Ready => true,
            // Połączenie jest w drodze: drugie naciśnięcie odwołuje
            // pierwsze, tak jak drugie kliknięcie przycisku.
            Link::Connecting | Link::Spawning => {
                self.pending.set(!self.pending.get());
                false
            }
            // Nikt nie odbierze i nikt nie wróci — nie obiecujemy nagrania,
            // którego nie będzie.
            Link::Lost => false,
        }
    }

    fn spawning(&self) {
        self.link.set(Link::Spawning);
    }

    /// Połączenie stanęło. `true` = jest odłożone żądanie do wykonania.
    fn ready(&self) -> bool {
        self.link.set(Link::Ready);
        self.pending.take()
    }

    fn lost(&self) {
        self.link.set(Link::Lost);
        self.pending.set(false);
    }
}

/// Czy dyktowanie można domknąć już teraz.
///
/// „OK" ma oddać **cały** podyktowany tekst, a ostatnia wypowiedź bywa
/// jeszcze w transkrypcji w chwili naciśnięcia skrótu. Domykamy więc dopiero
/// wtedy, gdy mikrofon milczy i kolejka jest pusta — inaczej skrót zjadałby
/// ostatnie zdanie.
fn accept_ready(recording: bool, backlog: u32) -> bool {
    !recording && backlog == 0
}

/// Domknięcie dyktowania odłożone do ucichnięcia kolejki.
///
/// Żądanie z przycisku „OK" albo ze skrótu rzadko trafia w moment, w którym
/// nie ma już nic do przetworzenia. Dlatego jest tu tak samo odkładane jak
/// żądanie nagrywania w [`Gate`]: zapamiętujemy je, a wykonujemy z obsługi
/// `state.changed`, gdy `backlog` spadnie do zera.
struct Accept {
    pending: Cell<bool>,
    backlog: Cell<u32>,
    finish: Box<dyn Fn()>,
}

impl Accept {
    fn new(finish: Box<dyn Fn()>) -> Self {
        Self {
            pending: Cell::new(false),
            backlog: Cell::new(0),
            finish,
        }
    }

    /// Żądanie domknięcia. Zwraca `true`, jeśli trzeba jeszcze zatrzymać
    /// nagrywanie — samo zatrzymanie należy do przycisku, żeby szła
    /// jedna ścieżka (`session.stop`, podtytuł, wskaźnik poziomu).
    fn request(&self, recording: bool) -> bool {
        if accept_ready(recording, self.backlog.get()) {
            (self.finish)();
            return false;
        }
        self.pending.set(true);
        recording
    }

    /// Nowy stan sesji z daemona. Domyka, jeśli czekaliśmy właśnie na to.
    fn on_state(&self, recording: bool, backlog: u32) {
        self.backlog.set(backlog);
        if self.pending.get() && accept_ready(recording, backlog) {
            self.pending.set(false);
            (self.finish)();
        }
    }

    /// Połączenie padło — nie ma na co czekać. Oddajemy to, co jest.
    fn give_up(&self) {
        if self.pending.take() {
            (self.finish)();
        }
    }

    fn cancel(&self) {
        self.pending.set(false);
    }
}

/// Podtytuł w fazie, gdy okno nie ma jeszcze klienta. Mówi nie tylko, na co
/// czekamy, ale i to, że odłożone żądanie nagrywania nie przepadło.
fn connecting_subtitle(link: Link, pending: bool) -> &'static str {
    match (link, pending) {
        (Link::Connecting, false) => "Łączę z daemonem…",
        (Link::Connecting, true) => "Łączę z daemonem — zaraz zacznę nagrywać…",
        (Link::Spawning, false) => "Uruchamiam daemona…",
        (Link::Spawning, true) => "Uruchamiam daemona — zaraz zacznę nagrywać…",
        (Link::Ready, _) => "Gotowe",
        (Link::Lost, _) => "Daemon się rozłączył",
    }
}

/// Pokazuje fazę łączenia: podtytuł i spinner obok tytułu.
///
/// Spinner w tej fazie znaczy „czekam, żeby zacząć nagrywać" — kręci się
/// wyłącznie wtedy, gdy jest odłożone żądanie, żeby zwykły start okna nie
/// migotał bez powodu. Po połączeniu ten sam spinner przejmuje licznik
/// zaległości transkrypcji.
fn show_connecting(subtitle: &Subtitle, spinner: &Spinner, gate: &Gate) {
    subtitle.say(connecting_subtitle(gate.link(), gate.pending()));
    if gate.pending() {
        spinner.set_visible(true);
        spinner.start();
    } else {
        spinner.stop();
        spinner.set_visible(false);
    }
}

/// Widżety, które ożywają dopiero z połączeniem.
///
/// Zbieramy je w jedno miejsce, bo podłączenie klienta dzieje się już po
/// pokazaniu okna — asynchronicznie, w odpowiedzi na wiadomość z wątku
/// łączącego. Bez tej struktury każdy widżet trzeba by klonować z osobna
/// do bloku `async`.
struct Ui {
    /// Podtytuł wraz ze wszystkim, co się na niego składa.
    subtitle: Rc<Subtitle>,
    backlog_spinner: Spinner,
    level_bar: LevelBar,
    record_button: ToggleButton,
    document: Rc<DocumentView>,
    tray: Option<TrayHandle>,
    show_level_meter: bool,
    gate: Rc<Gate>,
    accept: Rc<Accept>,
    /// Rozgałęzienie wypowiedzi: notatka albo pole polecenia redakcji.
    sink: Rc<redact::Sink>,
}

/// Co wątek łączący ma do powiedzenia oknu.
enum ConnectEvent {
    /// Daemona nie było — właśnie został uruchomiony i czekamy na gniazdo.
    Spawning,
    Ready(Client),
    Failed(String),
}

/// Łączy się z daemonem w tle i podłącza okno, gdy połączenie stanie.
///
/// Czekanie na gniazdo blokuje wątek na tyle długo (wczytanie modelu
/// whispera), że pętla GTK nie może go dotknąć — stąd osobny wątek i kanał.
fn spawn_connect(ui: Ui, config: diktate_config::GtkDaemonConfig) {
    let (tx, rx) = async_channel::unbounded::<ConnectEvent>();
    let timeout = std::time::Duration::from_millis(config.start_timeout_ms);
    let autostart = config.autostart;

    let spawn_tx = tx.clone();
    std::thread::spawn(move || {
        let result = if autostart {
            Client::connect_or_spawn(timeout, || {
                let _ = spawn_tx.send_blocking(ConnectEvent::Spawning);
            })
        } else {
            Client::connect()
        };
        let event = match result {
            Ok(client) => ConnectEvent::Ready(client),
            Err(err) => ConnectEvent::Failed(format!("{err:#}")),
        };
        let _ = tx.send_blocking(event);
    });

    let subtitle = Rc::clone(&ui.subtitle);
    let backlog_spinner = ui.backlog_spinner.clone();
    let gate = Rc::clone(&ui.gate);
    // `Option` zamiast zwykłego przeniesienia: kompilator nie wie, że
    // pętla kończy się na pierwszym `Ready` albo `Failed`, więc uchwyt
    // musimy z niej wyjąć, a nie przenieść.
    let mut ui = Some(ui);
    gtk4::glib::spawn_future_local(async move {
        while let Ok(event) = rx.recv().await {
            match event {
                ConnectEvent::Spawning => {
                    gate.spawning();
                    show_connecting(&subtitle, &backlog_spinner, &gate);
                }
                ConnectEvent::Ready(client) => {
                    if let Some(ui) = ui.take() {
                        ui.attach(Rc::new(client));
                    }
                    break;
                }
                ConnectEvent::Failed(message) => {
                    gate.lost();
                    backlog_spinner.stop();
                    backlog_spinner.set_visible(false);
                    subtitle.say(&format!("Brak daemona: {message}"));
                    break;
                }
            }
        }
    });
}

impl Ui {
    /// Podłącza gotowego klienta: strumień powiadomień i przycisk nagrywania.
    fn attach(self, client: Rc<Client>) {
        let Ui {
            subtitle,
            backlog_spinner,
            level_bar,
            record_button,
            document,
            tray,
            show_level_meter,
            gate,
            accept,
            sink,
        } = self;

        // Odczyty poziomu przychodzą tylko na prośbę — to kilkanaście
        // wiadomości na sekundę i nie ma ich dostawać nikt, kto nie rysuje
        // wskaźnika.
        if show_level_meter {
            if let Err(err) = client.subscribe_levels() {
                eprintln!("diktate-gtk: nie udało się włączyć odczytów poziomu: {err:#}");
            }
        }
        subtitle.set_session(SessionState::Idle, 0);
        record_button.set_sensitive(true);
        backlog_spinner.set_visible(false);
        backlog_spinner.stop();

        // Wiadomości z daemona wpadają tu asynchronicznie: tekst trafia w
        // kotwicę wstawiania, a zmiana stanu aktualizuje podtytuł
        // i widoczność spinnera zaległości.
        let incoming = client.incoming();
        let document_for_incoming = Rc::clone(&document);
        let subtitle_for_incoming = Rc::clone(&subtitle);
        let backlog_spinner_for_incoming = backlog_spinner.clone();
        let level_bar_for_incoming = level_bar.clone();
        let record_button_for_incoming = record_button.clone();
        let gate_for_incoming = Rc::clone(&gate);
        let accept_for_incoming = Rc::clone(&accept);
        gtk4::glib::spawn_future_local(async move {
            while let Ok(message) = incoming.recv().await {
                match message {
                    Incoming::Segment(segment) => {
                        // Gdy otwarte jest okno polecenia, wypowiedź jest
                        // poleceniem redakcyjnym, a nie treścią notatki —
                        // ale tylko wtedy, gdy padła po jego otwarciu.
                        // Zaległość sprzed niego należy do notatki.
                        let polecenie = sink.deliver(&segment.text, |mark| {
                            document_for_incoming.spoken_after(mark, segment.audio_start_ms)
                        });
                        if !polecenie {
                            document_for_incoming.insert_segment(&segment);
                        }
                    }
                    Incoming::Level { rms } => {
                        level_bar_for_incoming.set_value(meter_value(rms));
                    }
                    Incoming::State { state, backlog } => {
                        let (poprzedni, _) = subtitle_for_incoming.session();
                        // Nowe nagranie zaczyna oś czasu audio od zera —
                        // model dokumentu musi wiedzieć, od kiedy ją liczyć.
                        if state == SessionState::Recording && poprzedni != SessionState::Recording
                        {
                            document_for_incoming.begin_audio_session();
                        }
                        subtitle_for_incoming.set_session(state, backlog);
                        if let Some(handle) = &tray {
                            handle.update(|tray| {
                                tray.set_state(state == SessionState::Recording, backlog)
                            });
                        }
                        let nagrywa = state == SessionState::Recording;
                        // Odłożone „OK" domyka się dokładnie tutaj: gdy
                        // mikrofon zamilkł, a kolejka transkrypcji zeszła
                        // do zera, czyli cały tekst jest już w dokumencie.
                        accept_for_incoming.on_state(nagrywa, backlog);
                        level_bar_for_incoming.set_visible(nagrywa);
                        if !nagrywa {
                            level_bar_for_incoming.set_value(0.0);
                        }

                        if backlog > 0 {
                            backlog_spinner_for_incoming.set_visible(true);
                            backlog_spinner_for_incoming.start();
                        } else {
                            backlog_spinner_for_incoming.stop();
                            backlog_spinner_for_incoming.set_visible(false);
                        }
                    }
                }
            }

            // Kanał się zamknął — daemon się rozłączył. Zatrzymujemy
            // spinner, żeby nic nie kręciło się bez powodu przy martwym
            // połączeniu, i blokujemy przycisk: nie ma dokąd wysłać
            // `session.start`.
            backlog_spinner_for_incoming.stop();
            backlog_spinner_for_incoming.set_visible(false);
            level_bar_for_incoming.set_visible(false);
            gate_for_incoming.lost();
            // Nie ma już kto dosłać ostatniej wypowiedzi — oddajemy
            // schowkowi to, co zdążyło dojść, zamiast wisieć w oczekiwaniu.
            accept_for_incoming.give_up();
            // Najpierw odciskamy przycisk (przy okazji gasi klasę
            // `destructive-action`), dopiero potem piszemy podtytuł —
            // nieudany `session.stop` na martwym gnieździe zdążyłby go
            // nadpisać komunikatem o błędzie.
            record_button_for_incoming.set_active(false);
            record_button_for_incoming.set_sensitive(false);
            subtitle_for_incoming.say("Daemon się rozłączył");
        });

        record_button.connect_toggled(move |button| {
            // Klasa CSS pokazuje stan nagrywania na pierwszy rzut oka —
            // tak jak w systemowych aplikacjach GNOME (np. Nagrywanie
            // dźwięku). Sam tekst podtytułu nie jest tu zgadywany — czeka
            // na potwierdzenie przez `state.changed`.
            if button.is_active() {
                button.add_css_class("destructive-action");
            } else {
                button.remove_css_class("destructive-action");
            }

            let result = if button.is_active() {
                client.start()
            } else {
                client.stop()
            };
            if let Err(err) = result {
                subtitle.say(&format!("Błąd: {err}"));
                button.set_active(false);
            }
        });

        // Skrót globalny naciśnięty, zanim było z kim rozmawiać: dopiero
        // teraz puszczamy go w ruch — tą samą drogą co kliknięcie, więc
        // z ikoną nagrywania, wskaźnikiem poziomu i podtytułem ze stanu.
        if gate.ready() {
            record_button.set_active(true);
        }
    }
}

fn build_ui(app: &Application) {
    // Błąd w gtk.toml zatrzymuje start z czytelnym komunikatem, a nie cofa
    // się po cichu do wartości domyślnych (docs/04).
    let config = match GtkConfig::load() {
        Ok(config) => config,
        Err(err) => {
            eprintln!("diktate-gtk: {err:#}");
            std::process::exit(1);
        }
    };

    let text_view = TextView::builder()
        .wrap_mode(WrapMode::WordChar)
        .left_margin(12)
        .right_margin(12)
        .top_margin(12)
        .bottom_margin(12)
        .build();

    let scroller = ScrolledWindow::builder()
        .hscrollbar_policy(PolicyType::Never)
        .vexpand(true)
        .child(&text_view)
        .build();

    // `Clamp` ogranicza szerokość tekstu w dużym oknie, tak jak robią to
    // aplikacje GNOME (np. Ustawienia, Teksty).
    let clamp = Clamp::builder().child(&scroller).maximum_size(720).build();

    let status_page = StatusPage::builder()
        .icon_name("audio-input-microphone-symbolic")
        .title("Gotowy do dyktowania")
        .description("Naciśnij przycisk nagrywania, aby rozpocząć dyktowanie.")
        .vexpand(true)
        .build();

    let stack = Stack::new();
    stack.add_named(&status_page, Some(PAGE_EMPTY));
    stack.add_named(&clamp, Some(PAGE_TEXT));
    stack.set_visible_child_name(PAGE_EMPTY);

    let toast_overlay = ToastOverlay::new();
    toast_overlay.set_child(Some(&stack));

    let window_title = WindowTitle::new("Diktate", "Łączę z daemonem…");

    // Spinner obok tytułu — widoczny wyłącznie, gdy w kolejce transkrypcji
    // jest cokolwiek do przetworzenia (backlog > 0).
    let backlog_spinner = Spinner::builder().visible(false).build();

    let title_box = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Horizontal)
        .spacing(6)
        .halign(gtk4::Align::Center)
        .build();
    title_box.append(&window_title);
    title_box.append(&backlog_spinner);

    let record_button = ToggleButton::builder()
        .icon_name("audio-input-microphone-symbolic")
        .tooltip_text("Nagrywaj")
        .build();
    let copy_button = gtk4::Button::builder()
        .icon_name("edit-copy-symbolic")
        .tooltip_text("Kopiuj do schowka")
        .build();
    // Czyszczenie stoi przed kopiowaniem i jest nieczynne przy pustej
    // notatce — nie ma wtedy czego usuwać.
    let clear_button = gtk4::Button::builder()
        .icon_name("edit-clear-all-symbolic")
        .tooltip_text("Wyczyść tekst")
        .sensitive(false)
        .build();
    // Przycisk domykający: kopiuje i zamyka okno jednym ruchem. Jako
    // czynność sugerowana (`suggested-action`) stoi po prawej stronie
    // paska nagłówka — tak GNOME oznacza główne wyjście z widoku.
    // Korekta LLM-em: widoczna tylko wtedy, gdy jest skonfigurowana. Guzik,
    // który zawsze kończy się błędem sieci, jest gorszy niż jego brak.
    let correct_button = gtk4::Button::builder()
        .icon_name("tools-check-spelling-symbolic")
        .tooltip_text("Popraw tekst modelem językowym (Super+D)")
        .sensitive(false)
        .visible(config.correction.enabled)
        .build();
    // Redakcja poleceniem głosowym stoi obok korekty i dzieli z nią
    // konfigurację — jeden serwer, jeden model, dwie różne instrukcje.
    let redact_button = gtk4::Button::builder()
        .icon_name("document-edit-symbolic")
        .tooltip_text("Zredaguj poleceniem głosowym (Super+E)")
        .sensitive(false)
        .visible(config.correction.enabled)
        .build();
    let accept_button = gtk4::Button::builder()
        .label("OK")
        .tooltip_text("Kopiuj, zamknij nagrywanie i okno (Super+C)")
        .build();
    accept_button.add_css_class("suggested-action");

    let header = HeaderBar::new();
    header.set_title_widget(Some(&title_box));
    header.pack_start(&record_button);
    header.pack_end(&accept_button);
    header.pack_end(&copy_button);
    header.pack_end(&clear_button);
    header.pack_end(&correct_button);
    header.pack_end(&redact_button);

    // Wskaźnik poziomu tuż pod paskiem nagłówka: widoczny wyłącznie w trakcie
    // nagrywania, bo poza nim nie ma czego pokazywać.
    let level_bar = LevelBar::builder()
        .min_value(0.0)
        .max_value(1.0)
        .visible(false)
        .margin_start(12)
        .margin_end(12)
        .margin_top(6)
        .margin_bottom(6)
        .build();

    let toolbar_view = ToolbarView::new();
    toolbar_view.add_top_bar(&header);
    if config.window.show_level_meter {
        toolbar_view.add_top_bar(&level_bar);
    }
    toolbar_view.set_content(Some(&toast_overlay));

    let window = ApplicationWindow::builder()
        .application(app)
        .title("Diktate")
        .default_width(config.window.width)
        .default_height(config.window.height)
        .content(&toolbar_view)
        .build();

    // Bufor sam decyduje, którą stronę stosu pokazać — obejmuje to zarówno
    // tekst dopisywany z daemona, jak i ręczne usunięcie treści przez
    // użytkownika (wymaganie: powrót do `StatusPage` po wyczyszczeniu).
    // Model dokumentu podpięty pod bufor. Od tej chwili bufor jest widokiem,
    // a nie źródłem prawdy.
    let document = DocumentView::attach(&text_view, config.dictation.clone());

    // Trwająca korekta albo redakcja: przyciski i podtytuł muszą o niej
    // wiedzieć niezależnie od siebie, a sygnał zmiany bufora chodzi też
    // w jej trakcie. Obie czynności dzielą tę jedną flagę — wykluczają się.
    let busy = Rc::new(Cell::new(false));
    // Rozgałęzienie wypowiedzi: notatka albo pole polecenia redakcji.
    let sink = redact::Sink::new();

    // Podtytuł trzymany osobno: składa się ze stanu sesji, uzbrojenia
    // zastąpienia i trwającej korekty, a te trzy rzeczy zmieniają się
    // niezależnie od siebie.
    let subtitle = Rc::new(Subtitle::new(window_title.clone()));
    subtitle.say("Łączę z daemonem…");

    // Uzbrojone zaznaczenie widać dwiema drogami: innym kolorem samego
    // zaznaczenia i słowem „zastępuję…" w podtytule (docs/07). Kolory biorą
    // się z nazwanych barw Adwaity, więc motyw nadal decyduje.
    if let Some(display) = gtk4::gdk::Display::default() {
        let provider = gtk4::CssProvider::new();
        provider.load_from_data(REPLACE_CSS);
        gtk4::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
    let text_view_for_armed = text_view.clone();
    let subtitle_for_armed = Rc::clone(&subtitle);
    document.connect_armed(move |armed| {
        if armed {
            text_view_for_armed.add_css_class(REPLACE_ARMED_CLASS);
        } else {
            text_view_for_armed.remove_css_class(REPLACE_ARMED_CLASS);
        }
        subtitle_for_armed.set_armed(armed);
    });

    let buffer = text_view.buffer();
    let stack_for_buffer = stack.clone();
    let clear_button_for_buffer = clear_button.clone();
    let correct_button_for_buffer = correct_button.clone();
    let busy_for_buffer = Rc::clone(&busy);
    let redact_button_for_buffer = redact_button.clone();
    buffer.connect_changed(move |buffer| {
        let empty = buffer.char_count() == 0;
        let page = if empty { PAGE_EMPTY } else { PAGE_TEXT };
        stack_for_buffer.set_visible_child_name(page);
        clear_button_for_buffer.set_sensitive(!empty);
        // W trakcie korekty przycisk zostaje nieczynny — odpowiedź modelu
        // dotyczy tekstu sprzed niej i drugie żądanie tylko by je pomieszało.
        correct_button_for_buffer.set_sensitive(!empty && !busy_for_buffer.get());
        // Redakcja rządzi się tym samym: w trakcie dyktowania jest czynna,
        // bo wypowiedzi sprzed otwarcia okna polecenia odsiewa znacznik.
        redact_button_for_buffer.set_sensitive(!empty && !busy_for_buffer.get());
    });

    // Bramka pilnuje, żeby żądanie nagrywania z pustego biurka nie przepadło
    // ani nie skończyło się przyciskiem, który tylko wygląda na wciśnięty.
    let gate = Rc::new(Gate::default());

    // Domknięcie dyktowania w jednym miejscu: schowek dostaje cały tekst
    // dokumentu (nie zaznaczenie), a okno się zamyka. Daemon zostaje —
    // kolejne wywołanie skrótu globalnego podniesie okno na nowo.
    let accept = {
        let document = Rc::clone(&document);
        let window = window.clone();
        Rc::new(Accept::new(Box::new(move || {
            window.clipboard().set_text(&document.text());
            window.close();
        })))
    };

    // Akcja dla skrótu globalnego: przełącza przycisk nagrywania, a cała
    // reszta (start/stop sesji, podtytuł) dzieje się już tą samą drogą co
    // przy kliknięciu — jedna ścieżka zamiast dwóch. Gdy klienta jeszcze
    // nie ma, żądanie czeka w bramce i ruszy tą samą drogą po połączeniu.
    let toggle_action = gtk4::gio::SimpleAction::new(ACTION_TOGGLE, None);
    let record_button_for_action = record_button.clone();
    let subtitle_for_action = Rc::clone(&subtitle);
    let backlog_spinner_for_action = backlog_spinner.clone();
    let gate_for_action = Rc::clone(&gate);
    let accept_for_toggle = Rc::clone(&accept);
    toggle_action.connect_activate(move |_, _| {
        // Powrót do mikrofonu odwołuje odłożone „OK" — inaczej okno
        // zamknęłoby się w środku następnego dyktowania.
        accept_for_toggle.cancel();
        if gate_for_action.request() {
            record_button_for_action.set_active(!record_button_for_action.is_active());
        } else {
            show_connecting(
                &subtitle_for_action,
                &backlog_spinner_for_action,
                &gate_for_action,
            );
        }
    });
    app.add_action(&toggle_action);

    // „OK" jako akcja aplikacji, nie tylko sygnał przycisku: skrót
    // `Super+C` ma trafiać w to samo, a akcja jest jedynym miejscem,
    // gdzie wiąże się jedno z drugim. Skrót jest lokalny — działa, gdy
    // okno ma ognisko, i nie zabiera kombinacji reszcie systemu.
    let accept_action = gtk4::gio::SimpleAction::new(ACTION_ACCEPT, None);
    let accept_for_action = Rc::clone(&accept);
    let record_button_for_accept = record_button.clone();
    accept_action.connect_activate(move |_, _| {
        if accept_for_action.request(record_button_for_accept.is_active()) {
            // Zatrzymanie idzie tą samą drogą co kliknięcie przycisku:
            // `session.stop`, podtytuł i wskaźnik poziomu bez duplikatu.
            record_button_for_accept.set_active(false);
        }
    });
    app.add_action(&accept_action);
    app.set_accels_for_action(&format!("app.{ACTION_ACCEPT}"), &[ACCEPT_ACCEL]);

    let accept_action_for_button = accept_action.clone();
    accept_button.connect_clicked(move |_| accept_action_for_button.activate(None));

    // Ikona w tray-u rozmawia z oknem kanałem — żyje na własnym wątku
    // i nie wolno jej dotykać widgetów GTK.
    let tray_handle = if config.tray.enabled {
        let (tray_tx, tray_rx) = async_channel::unbounded::<TrayCommand>();
        let handle = DiktateTray::start(config.tray.show_backlog, tray_tx);
        if handle.is_some() {
            let app_for_tray = app.clone();
            let window_for_tray = window.clone();
            gtk4::glib::spawn_future_local(async move {
                while let Ok(command) = tray_rx.recv().await {
                    match command {
                        TrayCommand::Present => window_for_tray.present(),
                        TrayCommand::Toggle => {
                            window_for_tray.present();
                            if let Some(action) = app_for_tray.lookup_action(ACTION_TOGGLE) {
                                action.activate(None);
                            }
                        }
                        TrayCommand::Quit => app_for_tray.quit(),
                    }
                }
            });
        }
        handle
    } else {
        None
    };

    // Połączenie z daemonem idzie osobnym wątkiem. Gdy daemona nie ma,
    // klient sam go uruchamia i czeka na gniazdo — a to trwa tyle, ile
    // wczytanie modelu whispera, czyli sekundy. Okno przez ten czas jest
    // już otwarte i odpowiada; przycisk nagrywania budzi się dopiero
    // z połączeniem.
    record_button.set_sensitive(false);
    let correction_config = std::sync::Arc::new(config.correction.clone());
    let redaction = Rc::new(redact::Redaction {
        window: window.clone(),
        toasts: toast_overlay.clone(),
        document: Rc::clone(&document),
        subtitle: Rc::clone(&subtitle),
        busy: Rc::clone(&busy),
        sink: Rc::clone(&sink),
        record_button: record_button.clone(),
        button: redact_button.clone(),
        config: std::sync::Arc::clone(&correction_config),
    });
    let ui = Ui {
        subtitle: Rc::clone(&subtitle),
        backlog_spinner: backlog_spinner.clone(),
        level_bar: level_bar.clone(),
        record_button: record_button.clone(),
        document: Rc::clone(&document),
        tray: tray_handle,
        show_level_meter: config.window.show_level_meter,
        gate,
        accept: Rc::clone(&accept),
        sink: Rc::clone(&sink),
    };
    spawn_connect(ui, config.daemon.clone());

    let document_for_copy = Rc::clone(&document);
    let toast_overlay_for_copy = toast_overlay.clone();
    copy_button.connect_clicked(move |button| {
        button.clipboard().set_text(&document_for_copy.text());
        toast_overlay_for_copy.add_toast(Toast::new("Skopiowano"));
    });

    wire_correction(
        &correct_button,
        &window,
        &toast_overlay,
        &document,
        &subtitle,
        &busy,
        std::sync::Arc::clone(&correction_config),
    );
    redaction.wire();

    let document_for_clear = Rc::clone(&document);
    let toast_overlay_for_clear = toast_overlay.clone();
    clear_button.connect_clicked(move |_| {
        if document_for_clear.clear() {
            toast_overlay_for_clear.add_toast(Toast::new("Wyczyszczono"));
        }
    });

    // Cofanie i ponawianie obejmuje dyktowanie i pisanie razem — obie drogi
    // zapisują się w tym samym logu operacji.
    let shortcuts = gtk4::EventControllerKey::new();
    let document_for_keys = Rc::clone(&document);
    let redact_button_for_keys = redact_button.clone();
    let correct_button_for_keys = correct_button.clone();
    shortcuts.connect_key_pressed(move |_, key, _, state| {
        let ctrl = state.contains(gtk4::gdk::ModifierType::CONTROL_MASK);
        let shift = state.contains(gtk4::gdk::ModifierType::SHIFT_MASK);
        let super_key = state.contains(gtk4::gdk::ModifierType::SUPER_MASK);
        match key {
            gtk4::gdk::Key::z if ctrl && !shift => {
                document_for_keys.undo();
                gtk4::glib::Propagation::Stop
            }
            // Redakcja poleceniem głosowym i korekta modelem stoją obok
            // siebie na klawiszu `Super`, tak jak domknięcie `Super+C`.
            // Kombinacje z `Ctrl` zostają dla pracy z tekstem (`Ctrl+R`,
            // `Ctrl+Z`), a `Super` zbiera czynności aplikacji. Skróty są
            // lokalne — dotyczą tekstu w tym oknie, więc nie zabierają
            // kombinacji reszcie systemu (`docs/06`).
            gtk4::gdk::Key::E | gtk4::gdk::Key::e if super_key && !shift => {
                if redact_button_for_keys.is_sensitive() {
                    redact_button_for_keys.emit_clicked();
                }
                gtk4::glib::Propagation::Stop
            }
            gtk4::gdk::Key::D | gtk4::gdk::Key::d if super_key && !shift => {
                if correct_button_for_keys.is_sensitive() {
                    correct_button_for_keys.emit_clicked();
                }
                gtk4::glib::Propagation::Stop
            }
            gtk4::gdk::Key::Z | gtk4::gdk::Key::z if ctrl && shift => {
                document_for_keys.redo();
                gtk4::glib::Propagation::Stop
            }
            // Escape znosi zaznaczenie i wraca do zwykłego wstawiania, bez
            // żadnej zmiany w dokumencie (docs/07). Gdy nic nie jest
            // uzbrojone, klawisz idzie dalej — okno może mieć z nim swoje
            // sprawy.
            gtk4::gdk::Key::Escape if document_for_keys.cancel_replace() => {
                gtk4::glib::Propagation::Stop
            }
            _ => gtk4::glib::Propagation::Proceed,
        }
    });
    window.add_controller(shortcuts);

    window.present();
}

/// Podpina przycisk korekty tekstu.
///
/// Żądanie idzie do modelu **osobnym wątkiem**: odpowiedź to sekundy, a
/// pętla GTK nie może na nią czekać. Na czas korekty przycisk jest nieczynny,
/// a podtytuł mówi „Poprawiam tekst…” — nie ma tu drugiego wskaźnika
/// postępu, bo stan okna mieszka w podtytule (`docs/06`).
///
/// Wynik nie wchodzi do dokumentu wprost: najpierw przechodzi przez
/// [`correct::review`], a przy nadmiarze zmian — przez pytanie do
/// użytkownika. Zasada z `docs/11`: narzędzie zjadające prawdziwy tekst jest
/// gorsze od przepuszczonego błędu.
fn wire_correction(
    button: &gtk4::Button,
    window: &ApplicationWindow,
    toasts: &ToastOverlay,
    document: &Rc<DocumentView>,
    subtitle: &Rc<Subtitle>,
    busy: &Rc<Cell<bool>>,
    config: std::sync::Arc<diktate_config::CorrectionConfig>,
) {
    let window = window.clone();
    let toasts = toasts.clone();
    let document = Rc::clone(document);
    let subtitle = Rc::clone(subtitle);
    let busy = Rc::clone(busy);

    button.connect_clicked(move |button| {
        let before = document.text();
        if before.trim().is_empty() || busy.get() {
            return;
        }

        busy.set(true);
        button.set_sensitive(false);
        subtitle.set_busy(Some(BUSY_CORRECTING));

        let (tx, rx) = async_channel::bounded::<anyhow::Result<String>>(1);
        let config_for_thread = std::sync::Arc::clone(&config);
        let text_for_thread = before.clone();
        std::thread::spawn(move || {
            let _ = tx.send_blocking(correct::ask(&config_for_thread, &text_for_thread));
        });

        let window = window.clone();
        let toasts = toasts.clone();
        let document = Rc::clone(&document);
        let subtitle = Rc::clone(&subtitle);
        let busy = Rc::clone(&busy);
        let button = button.clone();
        let config = std::sync::Arc::clone(&config);
        gtk4::glib::spawn_future_local(async move {
            let result = rx.recv().await;
            busy.set(false);
            subtitle.set_busy(None);
            button.set_sensitive(!document.text().is_empty());

            match result {
                Ok(Ok(after)) => {
                    handle_correction(&window, &toasts, &document, &config, &before, &after)
                }
                Ok(Err(err)) => toasts.add_toast(Toast::new(&format!("Korekta: {err:#}"))),
                // Wątek zniknął bez odpowiedzi — nie ma czego pokazywać poza
                // tym, że nic z tego nie wyszło.
                Err(_) => toasts.add_toast(Toast::new("Korekta przerwana")),
            }
        });
    });
}

/// Rozstrzyga, co zrobić z odpowiedzią modelu.
fn handle_correction(
    window: &ApplicationWindow,
    toasts: &ToastOverlay,
    document: &Rc<DocumentView>,
    config: &diktate_config::CorrectionConfig,
    before: &str,
    after: &str,
) {
    // Notatka mogła w międzyczasie urosnąć o wypowiedź, która dopiero teraz
    // wyszła z transkrypcji. Podmiana całości zjadłaby ją bez śladu, a to
    // dokładnie ten rodzaj straty, przed którym korekta ma się bronić.
    if document.text() != before {
        toasts.add_toast(Toast::new(
            "Tekst zmienił się w trakcie korekty — nic nie podmieniam",
        ));
        return;
    }

    match correct::review(config, before, after) {
        correct::Verdict::Accept { ratio } => commit_correction(toasts, document, after, ratio),
        correct::Verdict::Reject { reason } => {
            toasts.add_toast(Toast::new(&format!("Korekta odrzucona: {reason}")))
        }
        correct::Verdict::Confirm { ratio, limit } => {
            let dialog = libadwaita::AlertDialog::new(
                Some("Korekta zmieniła więcej, niż wypada"),
                Some(&format!(
                    "Model zmienił {}% słów, a próg dla tej notatki to {}%. Przy takiej \
                     skali zmian przestaje poprawiać błędy rozpoznania, a zaczyna \
                     redagować wypowiedź własnymi słowami.",
                    correct::percent(ratio),
                    correct::percent(limit),
                )),
            );
            dialog.add_responses(&[("cancel", "Odrzuć"), ("apply", "Zastosuj mimo to")]);
            dialog.set_response_appearance("apply", libadwaita::ResponseAppearance::Destructive);
            dialog.set_default_response(Some("cancel"));
            dialog.set_close_response("cancel");

            let toasts = toasts.clone();
            let document = Rc::clone(document);
            let after = after.to_string();
            dialog.choose(window, gtk4::gio::Cancellable::NONE, move |response| {
                if response == "apply" {
                    commit_correction(&toasts, &document, &after, ratio);
                }
            });
        }
    }
}

/// Wpisuje poprawiony tekst do dokumentu. Jedna operacja logu, więc `Ctrl+Z`
/// cofa korektę w całości — i o tym właśnie mówi powiadomienie.
fn commit_correction(toasts: &ToastOverlay, document: &Rc<DocumentView>, text: &str, ratio: f32) {
    document.replace_all(text);
    toasts.add_toast(Toast::new(&format!(
        "Poprawiono — zmienionych słów: {}%. Ctrl+Z cofa.",
        correct::percent(ratio)
    )));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ok_czeka_az_ucichnie_mikrofon_i_kolejka() {
        // Najważniejszy przypadek: skrót naciśnięty tuż po ostatnim zdaniu,
        // które siedzi jeszcze w transkrypcji. Domknięcie musi poczekać.
        assert!(!accept_ready(true, 0), "nagrywanie trwa");
        assert!(!accept_ready(false, 1), "ostatnia wypowiedź w kolejce");
        assert!(!accept_ready(true, 3));
        assert!(accept_ready(false, 0), "cisza i pusta kolejka");
    }

    #[test]
    fn ok_w_ciszy_domyka_od_razu() {
        let done = Rc::new(Cell::new(0u32));
        let done_for_accept = Rc::clone(&done);
        let accept = Accept::new(Box::new(move || {
            done_for_accept.set(done_for_accept.get() + 1)
        }));

        assert!(!accept.request(false), "nie ma czego zatrzymywać");
        assert_eq!(done.get(), 1);
    }

    #[test]
    fn ok_w_trakcie_nagrywania_domyka_dopiero_po_opustoszeniu_kolejki() {
        let done = Rc::new(Cell::new(0u32));
        let done_for_accept = Rc::clone(&done);
        let accept = Accept::new(Box::new(move || {
            done_for_accept.set(done_for_accept.get() + 1)
        }));

        assert!(accept.request(true), "trzeba zatrzymać nagrywanie");
        assert_eq!(done.get(), 0);

        accept.on_state(false, 2);
        assert_eq!(done.get(), 0, "kolejka jeszcze pracuje");

        accept.on_state(false, 0);
        assert_eq!(done.get(), 1);

        // Kolejne stany nie domykają po raz drugi.
        accept.on_state(false, 0);
        assert_eq!(done.get(), 1);
    }

    #[test]
    fn ponowne_nagrywanie_odwoluje_odlozone_ok() {
        let done = Rc::new(Cell::new(0u32));
        let done_for_accept = Rc::clone(&done);
        let accept = Accept::new(Box::new(move || {
            done_for_accept.set(done_for_accept.get() + 1)
        }));

        accept.request(true);
        accept.cancel();
        accept.on_state(false, 0);
        assert_eq!(done.get(), 0, "użytkownik wrócił do dyktowania");
    }

    #[test]
    fn utrata_daemona_oddaje_to_co_jest() {
        let done = Rc::new(Cell::new(0u32));
        let done_for_accept = Rc::clone(&done);
        let accept = Accept::new(Box::new(move || {
            done_for_accept.set(done_for_accept.get() + 1)
        }));

        accept.request(true);
        accept.give_up();
        assert_eq!(done.get(), 1);
        accept.give_up();
        assert_eq!(done.get(), 1, "raz, nie za każdym rozłączeniem");
    }

    #[test]
    fn zadanie_nagrywania_przed_polaczeniem_czeka_zamiast_przepasc() {
        let gate = Gate::default();
        // Skrót globalny naciśnięty, zanim daemon odpowiedział.
        assert!(!gate.request(), "przycisku nie wolno wtedy przełączać");
        assert!(gate.pending());

        // Połączenie stanęło — dopiero teraz nagrywanie ma ruszyć.
        assert!(gate.ready());
        assert_eq!(gate.link(), Link::Ready);
        assert!(!gate.pending(), "żądanie zostało zużyte");
    }

    #[test]
    fn drugie_nacisniecie_w_trakcie_laczenia_odwoluje_pierwsze() {
        let gate = Gate::default();
        assert!(!gate.request());
        assert!(!gate.request());
        assert!(!gate.pending());
        assert!(!gate.ready(), "nie ma czego uruchamiać po połączeniu");
    }

    #[test]
    fn po_polaczeniu_zadanie_idzie_wprost_do_przycisku() {
        let gate = Gate::default();
        gate.ready();
        assert!(gate.request(), "to już zwykłe kliknięcie");
        assert!(!gate.pending(), "nic się nie odkłada");
    }

    #[test]
    fn uruchamianie_daemona_nie_gubi_odlozonego_zadania() {
        let gate = Gate::default();
        assert!(!gate.request());
        gate.spawning();
        assert_eq!(gate.link(), Link::Spawning);
        assert!(gate.pending());
        assert!(gate.ready());
    }

    #[test]
    fn po_utracie_polaczenia_nic_nie_czeka_w_bramce() {
        let gate = Gate::default();
        assert!(!gate.request());
        gate.lost();
        assert!(
            !gate.pending(),
            "odłożone żądanie przepada razem z daemonem"
        );
        // Nie obiecujemy nagrania, którego nie będzie — nikt nie wraca po
        // to żądanie, bo ponowne łączenie jeszcze nie istnieje.
        assert!(!gate.request());
        assert!(!gate.pending());
    }

    #[test]
    fn podtytul_mowi_o_czekajacym_nagrywaniu() {
        assert_eq!(
            connecting_subtitle(Link::Connecting, false),
            "Łączę z daemonem…"
        );
        assert_eq!(
            connecting_subtitle(Link::Spawning, false),
            "Uruchamiam daemona…"
        );
        assert_eq!(
            connecting_subtitle(Link::Spawning, true),
            "Uruchamiam daemona — zaraz zacznę nagrywać…"
        );
        assert_eq!(
            connecting_subtitle(Link::Connecting, true),
            "Łączę z daemonem — zaraz zacznę nagrywać…"
        );
    }

    #[test]
    fn liczba_pojedyncza_dla_jednej_wypowiedzi() {
        assert_eq!(polish_plural_form(1), PluralForm::One);
        assert_eq!(wypowiedzi_word(1), "wypowiedź");
        assert_eq!(backlog_phrase(1), "1 wypowiedź");
    }

    #[test]
    fn forma_kilka_dla_koncowek_2_do_4() {
        for n in [2, 3, 4, 22, 23, 24, 32, 33, 34] {
            assert_eq!(polish_plural_form(n), PluralForm::Few, "n = {n}");
        }
    }

    #[test]
    fn forma_wiele_dla_piatki_i_wiecej_oraz_nastolatkow() {
        for n in [5, 6, 9, 10, 11, 12, 13, 14, 15, 20, 21, 25] {
            assert_eq!(polish_plural_form(n), PluralForm::Many, "n = {n}");
        }
    }

    #[test]
    fn odmiana_slowa_wypowiedz_pokrywa_sie_dla_kilku_i_wielu() {
        // Dla rzeczownika „wypowiedź" forma „kilka" i „wiele" brzmią tak samo
        // („wypowiedzi"), więc wynikowy tekst jest identyczny niezależnie od
        // tego, do której grupy trafia liczba — sprawdzamy oba przypadki z
        // treści zadania wprost.
        assert_eq!(wypowiedzi_word(2), "wypowiedzi");
        assert_eq!(wypowiedzi_word(5), "wypowiedzi");
        assert_eq!(wypowiedzi_word(12), "wypowiedzi");
        assert_eq!(wypowiedzi_word(22), "wypowiedzi");
        assert_eq!(wypowiedzi_word(23), "wypowiedzi");
        assert_eq!(wypowiedzi_word(25), "wypowiedzi");

        assert_eq!(backlog_phrase(2), "2 wypowiedzi");
        assert_eq!(backlog_phrase(5), "5 wypowiedzi");
        assert_eq!(backlog_phrase(12), "12 wypowiedzi");
        assert_eq!(backlog_phrase(22), "22 wypowiedzi");
        assert_eq!(backlog_phrase(23), "23 wypowiedzi");
        assert_eq!(backlog_phrase(25), "25 wypowiedzi");
    }

    #[test]
    fn uzbrojone_zastapienie_widac_w_podtytule() {
        assert_eq!(
            compose_subtitle(SessionState::Recording, 0, true, None),
            "Zastępuję… · Nagrywam"
        );
    }

    #[test]
    fn bez_uzbrojenia_podtytul_zostaje_taki_jak_byl() {
        assert_eq!(
            compose_subtitle(SessionState::Idle, 0, false, None),
            subtitle_for_state(SessionState::Idle, 0)
        );
    }

    #[test]
    fn trwajaca_korekta_nie_zamazuje_reszty_podtytulu() {
        // Trzy niezależne rzeczy naraz: korekta idzie na początek, ale
        // ani zastąpienie, ani stan sesji nie znikają.
        assert_eq!(
            compose_subtitle(SessionState::Recording, 2, true, Some(BUSY_CORRECTING)),
            "Poprawiam tekst… · Zastępuję… · Nagrywam · przetwarzam 2 wypowiedzi"
        );
        assert_eq!(
            compose_subtitle(SessionState::Idle, 0, false, Some(BUSY_CORRECTING)),
            "Poprawiam tekst… · Gotowe"
        );
    }

    #[test]
    fn trwajaca_redakcja_mowi_o_sobie_a_nie_o_korekcie() {
        // Korekta i redakcja zajmują to samo miejsce w podtytule, ale są
        // różnymi czynnościami — użytkownik ma widzieć tę, która trwa.
        assert_eq!(
            compose_subtitle(SessionState::Idle, 0, false, Some(BUSY_REDACTING)),
            "Redaguję tekst… · Gotowe"
        );
    }

    #[test]
    fn podtytul_gotowe_gdy_bezczynnie_i_pusto() {
        assert_eq!(subtitle_for_state(SessionState::Idle, 0), "Gotowe");
    }

    #[test]
    fn podtytul_nagrywam_gdy_pusta_kolejka() {
        assert_eq!(subtitle_for_state(SessionState::Recording, 0), "Nagrywam");
    }

    #[test]
    fn podtytul_nagrywam_z_kolejka() {
        assert_eq!(
            subtitle_for_state(SessionState::Recording, 1),
            "Nagrywam · przetwarzam 1 wypowiedź"
        );
        assert_eq!(
            subtitle_for_state(SessionState::Recording, 2),
            "Nagrywam · przetwarzam 2 wypowiedzi"
        );
        assert_eq!(
            subtitle_for_state(SessionState::Recording, 5),
            "Nagrywam · przetwarzam 5 wypowiedzi"
        );
    }

    #[test]
    fn podtytul_przetwarzam_po_zatrzymaniu_sesji() {
        assert_eq!(
            subtitle_for_state(SessionState::Idle, 3),
            "Przetwarzam 3 wypowiedzi…"
        );
    }
}
