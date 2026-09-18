//! Rejestracja skrótu globalnego w GNOME przez `gsettings`.
//!
//! Na Wayland żadna aplikacja nie przechwyci globalnego skrótu sama — nie ma
//! odpowiednika `XGrabKey`. Zamiast czekać na portal `GlobalShortcuts`
//! (świeży i różny między kompozytorami) rejestrujemy **własny skrót
//! środowiska**, który uruchamia `diktate-gtk --toggle`. Działa dzisiaj,
//! w każdym środowisku, i pozwala związać cokolwiek — także w sxhkd,
//! Hyprlandzie czy i3, gdzie wystarczy wpisać to samo polecenie ręcznie.
//!
//! `gsettings` trzyma własne skróty jako listę ścieżek relokowalnego
//! schematu. Składanie i rozbieranie tej listy jest tu wydzielone
//! z wywołań procesu, żeby dało się je przetestować bez GNOME.

use std::process::Command;

use anyhow::{bail, Context, Result};

const MEDIA_KEYS_SCHEMA: &str = "org.gnome.settings-daemon.plugins.media-keys";
const KEYBINDING_SCHEMA: &str = "org.gnome.settings-daemon.plugins.media-keys.custom-keybinding";
const KEYBINDING_PATH_PREFIX: &str =
    "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/";
/// Nazwa, po której poznajemy własny wpis przy usuwaniu.
const BINDING_NAME: &str = "Diktate — dyktowanie";

/// Rozbiera listę w formacie `gsettings`: `['a', 'b']` albo `@as []`.
fn parse_list(raw: &str) -> Vec<String> {
    let raw = raw.trim();
    let Some(start) = raw.find('[') else {
        return Vec::new();
    };
    let Some(end) = raw.rfind(']') else {
        return Vec::new();
    };
    raw[start + 1..end]
        .split(',')
        .map(|item| item.trim().trim_matches('\'').trim_matches('"').to_string())
        .filter(|item| !item.is_empty())
        .collect()
}

/// Składa listę w formacie, który `gsettings set` przyjmie z powrotem.
fn format_list(items: &[String]) -> String {
    if items.is_empty() {
        return "@as []".to_string();
    }
    let inner: Vec<String> = items.iter().map(|item| format!("'{item}'")).collect();
    format!("[{}]", inner.join(", "))
}

/// Pierwsza wolna ścieżka `customN` — numerujemy od zera i bierzemy pierwszą
/// nieużywaną, zamiast dopisywać na końcu. Dziury po usuniętych skrótach
/// istnieją i nie ma powodu ich omijać.
fn next_free_slot(existing: &[String]) -> String {
    for n in 0.. {
        let candidate = format!("{KEYBINDING_PATH_PREFIX}custom{n}/");
        if !existing.iter().any(|item| item == &candidate) {
            return candidate;
        }
    }
    unreachable!("zakres u32 nie wyczerpie się przy skrótach klawiszowych")
}

fn gsettings(args: &[&str]) -> Result<String> {
    let output = Command::new("gsettings")
        .args(args)
        .output()
        .context("nie udało się uruchomić `gsettings` — czy GNOME jest zainstalowane?")?;
    if !output.status.success() {
        bail!(
            "gsettings {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn custom_keybindings() -> Result<Vec<String>> {
    Ok(parse_list(&gsettings(&[
        "get",
        MEDIA_KEYS_SCHEMA,
        "custom-keybindings",
    ])?))
}

fn set_custom_keybindings(items: &[String]) -> Result<()> {
    gsettings(&[
        "set",
        MEDIA_KEYS_SCHEMA,
        "custom-keybindings",
        &format_list(items),
    ])?;
    Ok(())
}

/// Ścieżka skrótu wskazującego na nasze polecenie, jeśli już istnieje.
fn existing_slot(items: &[String]) -> Result<Option<String>> {
    for item in items {
        let schema = format!("{KEYBINDING_SCHEMA}:{item}");
        let name = gsettings(&["get", &schema, "name"])?;
        if name.trim().trim_matches('\'') == BINDING_NAME {
            return Ok(Some(item.clone()));
        }
    }
    Ok(None)
}

/// Rejestruje skrót `keys` uruchamiający tę binarkę z `--toggle`.
/// Powtórne wywołanie nadpisuje istniejący wpis, a nie dokłada drugiego.
pub fn install(keys: &str) -> Result<()> {
    let command = format!(
        "{} --toggle",
        std::env::current_exe()
            .context("nie udało się ustalić ścieżki własnej binarki")?
            .display()
    );

    let mut items = custom_keybindings()?;
    let slot = match existing_slot(&items)? {
        Some(slot) => slot,
        None => {
            let slot = next_free_slot(&items);
            items.push(slot.clone());
            set_custom_keybindings(&items)?;
            slot
        }
    };

    let schema = format!("{KEYBINDING_SCHEMA}:{slot}");
    gsettings(&["set", &schema, "name", BINDING_NAME])?;
    gsettings(&["set", &schema, "command", &command])?;
    gsettings(&["set", &schema, "binding", keys])?;
    println!("Skrót {keys} uruchamia teraz: {command}");
    Ok(())
}

/// Usuwa wcześniej zarejestrowany skrót. Brak wpisu nie jest błędem.
pub fn remove() -> Result<()> {
    let items = custom_keybindings()?;
    let Some(slot) = existing_slot(&items)? else {
        println!("Nie było czego usuwać — skrót nie jest zarejestrowany.");
        return Ok(());
    };
    let remaining: Vec<String> = items.into_iter().filter(|item| item != &slot).collect();
    set_custom_keybindings(&remaining)?;
    println!("Skrót usunięty.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pusta_lista_gsettings_jest_rozbierana_na_nic() {
        assert!(parse_list("@as []").is_empty());
        assert!(parse_list("[]").is_empty());
        assert!(parse_list("").is_empty());
    }

    #[test]
    fn lista_gsettings_jest_rozbierana_na_sciezki() {
        let raw = "['/org/gnome/a/custom0/', '/org/gnome/a/custom1/']\n";
        assert_eq!(
            parse_list(raw),
            vec![
                "/org/gnome/a/custom0/".to_string(),
                "/org/gnome/a/custom1/".to_string()
            ]
        );
    }

    #[test]
    fn zlozona_lista_wraca_do_postaci_przyjmowanej_przez_gsettings() {
        let items = vec!["/a/".to_string(), "/b/".to_string()];
        assert_eq!(format_list(&items), "['/a/', '/b/']");
        // Pusta lista wymaga jawnego typu — `[]` samo w sobie gsettings
        // odrzuca jako niejednoznaczne.
        assert_eq!(format_list(&[]), "@as []");
    }

    #[test]
    fn skladanie_i_rozbieranie_listy_jest_odwracalne() {
        let items = vec![
            format!("{KEYBINDING_PATH_PREFIX}custom0/"),
            format!("{KEYBINDING_PATH_PREFIX}custom3/"),
        ];
        assert_eq!(parse_list(&format_list(&items)), items);
    }

    #[test]
    fn pierwsza_wolna_szczelina_to_zero_gdy_nie_ma_zadnej() {
        assert_eq!(
            next_free_slot(&[]),
            format!("{KEYBINDING_PATH_PREFIX}custom0/")
        );
    }

    #[test]
    fn wolna_szczelina_wypelnia_dziure_po_usunietym_skrocie() {
        let existing = vec![
            format!("{KEYBINDING_PATH_PREFIX}custom0/"),
            format!("{KEYBINDING_PATH_PREFIX}custom2/"),
        ];
        assert_eq!(
            next_free_slot(&existing),
            format!("{KEYBINDING_PATH_PREFIX}custom1/")
        );
    }
}
