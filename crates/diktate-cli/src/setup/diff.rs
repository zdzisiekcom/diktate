//! Lista różnic między konfiguracją bieżącą a tą po dialogu.
//!
//! Porównujemy nie pola struktur, tylko ich postać w TOML-u — dzięki temu
//! dodanie pola w `diktate-config` nie wymaga tknięcia tego pliku, a kreator
//! nie ma własnej, rozjeżdżającej się listy pól.

use anyhow::Result;
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    /// Nazwa z kropką, tak jak w pliku: `vad.long_segment_ms`.
    pub field: String,
    pub before: String,
    pub after: String,
}

pub fn changes<T: Serialize>(before: &T, after: &T) -> Result<Vec<Change>> {
    let before = flatten(&toml::Value::try_from(before)?);
    let after = flatten(&toml::Value::try_from(after)?);
    let mut out = Vec::new();
    for (field, after_value) in &after {
        let before_value = before.get(field).cloned().unwrap_or_default();
        if &before_value != after_value {
            out.push(Change {
                field: field.clone(),
                before: before_value,
                after: after_value.clone(),
            });
        }
    }
    out.sort_by(|a, b| a.field.cmp(&b.field));
    Ok(out)
}

/// Spłaszcza tabele TOML-a do par `sekcja.pole` → wartość.
fn flatten(value: &toml::Value) -> std::collections::BTreeMap<String, String> {
    let mut out = std::collections::BTreeMap::new();
    walk(value, String::new(), &mut out);
    out
}

fn walk(
    value: &toml::Value,
    prefix: String,
    out: &mut std::collections::BTreeMap<String, String>,
) {
    match value {
        toml::Value::Table(table) => {
            for (key, inner) in table {
                let path = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                walk(inner, path, out);
            }
        }
        toml::Value::String(s) => {
            out.insert(prefix, s.clone());
        }
        inne => {
            out.insert(prefix, inne.to_string());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use diktate_config::DaemonConfig;

    #[test]
    fn brak_zmian_to_pusta_lista() {
        let cfg = DaemonConfig::default();
        assert!(changes(&cfg, &cfg).unwrap().is_empty());
    }

    #[test]
    fn jedna_zmiana_z_obiema_wartościami() {
        let przed = DaemonConfig::default();
        let mut po = przed.clone();
        po.vad.long_segment_ms = 4_000;

        let zmiany = changes(&przed, &po).unwrap();

        assert_eq!(zmiany.len(), 1);
        assert_eq!(zmiany[0].field, "vad.long_segment_ms");
        assert_eq!(zmiany[0].before, "2500");
        assert_eq!(zmiany[0].after, "4000");
    }

    #[test]
    fn zmiana_w_dwóch_sekcjach() {
        let przed = DaemonConfig::default();
        let mut po = przed.clone();
        po.whisper.language = "en".to_string();
        po.audio.device = "hw:1".to_string();

        let zmiany = changes(&przed, &po).unwrap();
        let pola: Vec<&str> = zmiany.iter().map(|z| z.field.as_str()).collect();

        assert_eq!(pola, vec!["audio.device", "whisper.language"]);
    }
}
