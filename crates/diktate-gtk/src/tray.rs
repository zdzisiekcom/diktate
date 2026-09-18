//! Ikona w obszarze powiadomień (`StatusNotifierItem` przez `ksni`).
//!
//! GNOME nie ma natywnego obszaru powiadomień od wersji 3.26, więc ikona
//! pojawia się tam dopiero z rozszerzeniem AppIndicator. Wybór padł mimo to
//! na `StatusNotifierItem`, bo to samo rozwiązanie działa poza GNOME — w KDE,
//! XFCE i większości paneli tiling WM — czyli dokładnie tam, gdzie ta
//! aplikacja ma działać (`docs/06-gui.md`). Brak tray-a nie jest błędem
//! krytycznym: aplikacja działa skrótem globalnym i oknem, a my tylko
//! ostrzegamy.
//!
//! Tray żyje na własnym wątku i nie wolno mu dotykać widgetów GTK. Dlatego
//! rozmawia z oknem **kanałem**: kliknięcie w menu wysyła [`TrayCommand`],
//! a pętla GTK odbiera je u siebie i wykonuje tą samą drogą, co kliknięcie
//! w oknie.

use async_channel::Sender;
use ksni::blocking::{Handle, TrayMethods};
use ksni::menu::StandardItem;
use ksni::{Icon, MenuItem, ToolTip};

/// Czego okno ma dokonać na życzenie tray-a.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayCommand {
    /// Przywołaj okno i przełącz nagrywanie — to samo, co skrót globalny.
    Toggle,
    /// Tylko przywołaj okno.
    Present,
    /// Zakończ aplikację.
    Quit,
}

const ICON_IDLE: &str = "audio-input-microphone-symbolic";
const ICON_RECORDING: &str = "media-record-symbolic";

/// Uchwyt do ikony w tray-u — tym rozmawia z nią okno.
pub type TrayHandle = Handle<DiktateTray>;

pub struct DiktateTray {
    recording: bool,
    backlog: u32,
    show_backlog: bool,
    commands: Sender<TrayCommand>,
}

impl DiktateTray {
    /// Uruchamia ikonę w tle. Błąd nie jest krytyczny — zwracamy `None`
    /// i mówimy o tym na stderr, bo aplikacja bez tray-a wciąż działa.
    pub fn start(show_backlog: bool, commands: Sender<TrayCommand>) -> Option<Handle<Self>> {
        let tray = Self {
            recording: false,
            backlog: 0,
            show_backlog,
            commands,
        };
        match tray.spawn() {
            Ok(handle) => Some(handle),
            Err(err) => {
                eprintln!(
                    "diktate-gtk: ikona w tray-u niedostępna ({err}) — \
                     w GNOME wymaga rozszerzenia AppIndicator. \
                     Aplikacja działa dalej przez okno i skrót globalny."
                );
                None
            }
        }
    }

    /// Stan pokazywany na ikonie i w podpowiedzi.
    pub fn set_state(&mut self, recording: bool, backlog: u32) {
        self.recording = recording;
        self.backlog = backlog;
    }

    fn send(&self, command: TrayCommand) {
        // Kanał nieograniczony — wysyłka nie blokuje wątku tray-a, a menu
        // nie ma prawa zamarznąć na czekaniu na pętlę GTK.
        let _ = self.commands.send_blocking(command);
    }

    fn status_text(&self) -> String {
        match (self.recording, self.backlog) {
            (true, 0) => "Nagrywam".to_string(),
            (true, n) if self.show_backlog => format!("Nagrywam — {n} w kolejce"),
            (true, _) => "Nagrywam".to_string(),
            (false, n) if n > 0 && self.show_backlog => format!("Przetwarzam — {n} w kolejce"),
            (false, _) => "Gotowy do dyktowania".to_string(),
        }
    }
}

impl ksni::Tray for DiktateTray {
    fn id(&self) -> String {
        env!("CARGO_PKG_NAME").into()
    }

    fn title(&self) -> String {
        "Diktate".into()
    }

    fn icon_name(&self) -> String {
        if self.recording {
            ICON_RECORDING.into()
        } else {
            ICON_IDLE.into()
        }
    }

    fn icon_pixmap(&self) -> Vec<Icon> {
        Vec::new()
    }

    fn tool_tip(&self) -> ToolTip {
        ToolTip {
            title: "Diktate".into(),
            description: self.status_text(),
            ..Default::default()
        }
    }

    /// Kliknięcie w ikonę robi to, po co ktoś w nią klika: przywołuje okno.
    fn activate(&mut self, _x: i32, _y: i32) {
        self.send(TrayCommand::Present);
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let toggle_label = if self.recording {
            "Zakończ dyktowanie"
        } else {
            "Rozpocznij dyktowanie"
        };
        vec![
            StandardItem {
                label: toggle_label.into(),
                icon_name: self.icon_name(),
                activate: Box::new(|tray: &mut Self| tray.send(TrayCommand::Toggle)),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: "Pokaż okno".into(),
                activate: Box::new(|tray: &mut Self| tray.send(TrayCommand::Present)),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: "Zakończ".into(),
                icon_name: "application-exit-symbolic".into(),
                activate: Box::new(|tray: &mut Self| tray.send(TrayCommand::Quit)),
                ..Default::default()
            }
            .into(),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tray(recording: bool, backlog: u32, show_backlog: bool) -> DiktateTray {
        let (tx, _rx) = async_channel::unbounded();
        DiktateTray {
            recording,
            backlog,
            show_backlog,
            commands: tx,
        }
    }

    #[test]
    fn ikona_zmienia_sie_w_trakcie_nagrywania() {
        assert_eq!(ksni::Tray::icon_name(&tray(false, 0, true)), ICON_IDLE);
        assert_eq!(ksni::Tray::icon_name(&tray(true, 0, true)), ICON_RECORDING);
    }

    #[test]
    fn podpowiedz_mowi_o_zaleglosciach_gdy_wlaczone() {
        assert_eq!(tray(true, 3, true).status_text(), "Nagrywam — 3 w kolejce");
        assert_eq!(tray(true, 3, false).status_text(), "Nagrywam");
    }

    #[test]
    fn po_zatrzymaniu_z_kolejka_podpowiedz_mowi_ze_liczy_dalej() {
        // Sesja zatrzymana, ale wypowiedzi wciąż się transkrybują — bez tego
        // ikona mówiłaby „gotowy", gdy tekst jeszcze nie dotarł na ekran.
        assert_eq!(
            tray(false, 2, true).status_text(),
            "Przetwarzam — 2 w kolejce"
        );
        assert_eq!(tray(false, 0, true).status_text(), "Gotowy do dyktowania");
    }

    #[test]
    fn pozycja_menu_nazywa_czynnosc_ktora_wykona() {
        let menu = ksni::Tray::menu(&tray(false, 0, true));
        assert!(
            matches!(&menu[0], MenuItem::Standard(item) if item.label == "Rozpocznij dyktowanie")
        );
        let menu = ksni::Tray::menu(&tray(true, 0, true));
        assert!(matches!(&menu[0], MenuItem::Standard(item) if item.label == "Zakończ dyktowanie"));
    }
}
