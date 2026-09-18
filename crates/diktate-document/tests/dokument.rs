//! Testy modelu dokumentu — dokładnie te przypadki, które `docs/03-model-dokumentu.md`
//! wymienia jako obowiązkowe, plus reguły sklejania z `docs/07`.
//!
//! Wszystkie działają bez audio, bez daemona i bez GTK.

use diktate_document::{Anchor, Document, Normalization};

/// Dokument z jedną wypowiedzią — punkt wyjścia większości testów.
fn dokument_z(text: &str) -> (Document, Anchor) {
    let mut doc = Document::new();
    let anchor = doc
        .insert_segment(0, text, Anchor::Start, &bez_normalizacji())
        .anchor;
    (doc, anchor)
}

/// Normalizacja wyłączona — testy struktury spanów nie mają walczyć ze
/// spacjami dokładanymi na stykach.
fn bez_normalizacji() -> Normalization {
    Normalization {
        auto_space: false,
        strip_trailing_ellipsis: false,
    }
}

#[test]
fn wstawienie_na_koncu_nie_przesuwa_istniejacych_kotwic() {
    // Scenariusz docelowy: użytkownik zaznacza fragment, a w tle dochodzi
    // kolejny segment. Zaznaczenie nie może stać się nieważne.
    let (mut doc, _) = dokument_z("Pierwsza wypowiedź.");
    let zaznaczenie_od = doc.anchor_at_char(3).unwrap();
    let zaznaczenie_do = doc.anchor_at_char(8).unwrap();
    let tekst_zaznaczenia = doc.text_between(zaznaczenie_od, zaznaczenie_do);

    doc.insert_segment(1, "Druga.", doc.end_anchor(), &bez_normalizacji());

    assert_eq!(
        doc.text_between(zaznaczenie_od, zaznaczenie_do),
        tekst_zaznaczenia
    );
    assert_eq!(doc.text(), "Pierwsza wypowiedź.Druga.");
}

#[test]
fn wstawienie_w_srodek_spanu_zachowuje_id_lewej_polowki() {
    let (mut doc, _) = dokument_z("abcdef");
    let lewa_id = doc.anchor_at_char(0).unwrap().span_id();
    let za_cieciem = doc.anchor_at_char(5).unwrap();

    doc.insert_segment(1, "XY", doc.anchor_at_char(3).unwrap(), &bez_normalizacji());

    assert_eq!(doc.text(), "abcXYdef");
    // Lewa połówka zachowuje SpanId — kotwice przed punktem cięcia są
    // dosłownie takie same.
    assert_eq!(doc.anchor_at_char(0).unwrap().span_id(), lewa_id);
    // Kotwica zza punktu cięcia wskazuje ten sam znak co przed operacją.
    assert_eq!(doc.char_at(za_cieciem), Some('f'));
}

#[test]
fn wstawienie_na_granicy_spanow_nie_tnie_niczego() {
    let (mut doc, _) = dokument_z("abc");
    doc.insert_segment(1, "def", doc.end_anchor(), &bez_normalizacji());
    let liczba_spanow = doc.span_count();

    doc.insert_segment(2, "X", doc.anchor_at_char(3).unwrap(), &bez_normalizacji());

    assert_eq!(doc.text(), "abcXdef");
    // Doszedł dokładnie jeden span — żaden istniejący nie został przecięty.
    assert_eq!(doc.span_count(), liczba_spanow + 1);
}

#[test]
fn undo_cofa_wstawienie_znak_w_znak() {
    let (mut doc, _) = dokument_z("Pierwsza.");
    let przed = doc.text();
    doc.insert_segment(1, "Druga.", doc.end_anchor(), &bez_normalizacji());

    assert!(doc.undo());

    assert_eq!(doc.text(), przed);
}

#[test]
fn redo_przywraca_cofniete_wstawienie() {
    let (mut doc, _) = dokument_z("Pierwsza.");
    doc.insert_segment(1, "Druga.", doc.end_anchor(), &bez_normalizacji());
    let po = doc.text();

    doc.undo();
    assert!(doc.redo());

    assert_eq!(doc.text(), po);
}

#[test]
fn nowa_operacja_kasuje_stos_ponowien() {
    let (mut doc, _) = dokument_z("Pierwsza.");
    doc.insert_segment(1, "Druga.", doc.end_anchor(), &bez_normalizacji());
    doc.undo();

    doc.insert_segment(2, "Trzecia.", doc.end_anchor(), &bez_normalizacji());

    assert!(
        !doc.redo(),
        "ponowienie po nowej operacji nie ma czego przywrócić"
    );
    assert_eq!(doc.text(), "Pierwsza.Trzecia.");
}

#[test]
fn undo_na_pustym_dokumencie_nie_panikuje() {
    let mut doc = Document::new();
    assert!(!doc.undo());
    assert_eq!(doc.text(), "");
}

// --- normalizacja: sklejanie z otoczeniem (docs/07) ---

fn normalizacja() -> Normalization {
    Normalization {
        auto_space: true,
        strip_trailing_ellipsis: true,
    }
}

#[test]
fn spacja_rozdzielajaca_pojawia_sie_na_styku_dwoch_wypowiedzi() {
    let mut doc = Document::new();
    doc.insert_segment(0, "Pierwsza.", Anchor::Start, &normalizacja());
    doc.insert_segment(1, "Druga.", doc.end_anchor(), &normalizacja());

    assert_eq!(doc.text(), "Pierwsza. Druga.");
}

#[test]
fn brak_spacji_przed_interpunkcja() {
    let mut doc = Document::new();
    doc.insert_segment(0, "Zdanie", Anchor::Start, &normalizacja());
    doc.insert_segment(1, ", dalej.", doc.end_anchor(), &normalizacja());

    assert_eq!(doc.text(), "Zdanie, dalej.");
}

#[test]
fn brak_spacji_po_otwierajacym_nawiasie() {
    let mut doc = Document::new();
    doc.insert_segment(0, "Tekst (", Anchor::Start, &normalizacja());
    doc.insert_segment(1, "wtrącenie)", doc.end_anchor(), &normalizacja());

    assert_eq!(doc.text(), "Tekst (wtrącenie)");
}

#[test]
fn spacja_takze_za_tekstem_wstawionym_w_srodek_zdania() {
    let mut doc = Document::new();
    doc.insert_segment(0, "Ala ma kota.", Anchor::Start, &normalizacja());
    // Kursor tuż za „Ala" — wstawiamy w środek zdania.
    doc.insert_segment(1, "zawsze", doc.anchor_at_char(3).unwrap(), &normalizacja());

    assert_eq!(doc.text(), "Ala zawsze ma kota.");
}

#[test]
fn koncowy_wielokropek_znika_przed_doklejeniem_dalszego_ciagu() {
    // Whisper kończy wielokropkiem każdą wypowiedź przerwaną pauzą na
    // oddech — bez tej reguły notatka jest nimi usiana.
    let mut doc = Document::new();
    doc.insert_segment(0, "Zacząłem mówić…", Anchor::Start, &normalizacja());
    doc.insert_segment(1, "i skończyłem.", doc.end_anchor(), &normalizacja());

    assert_eq!(doc.text(), "Zacząłem mówić i skończyłem.");
}

#[test]
fn jedno_cofniecie_zdejmuje_wstawienie_razem_ze_spacja_i_wielokropkiem() {
    // Zasada nadrzędna z docs/07: normalizacja jest częścią operacji
    // wstawienia, a nie osobnym krokiem po niej.
    let mut doc = Document::new();
    doc.insert_segment(0, "Zacząłem mówić…", Anchor::Start, &normalizacja());
    let przed = doc.text();
    doc.insert_segment(1, "i skończyłem.", doc.end_anchor(), &normalizacja());

    assert!(doc.undo());

    assert_eq!(
        doc.text(),
        przed,
        "jedno cofnięcie musi wrócić do stanu sprzed wstawienia"
    );
}

// --- edycja klawiaturą w tym samym logu ---

#[test]
fn edycja_klawiatura_jest_operacja_w_tym_samym_logu() {
    let (mut doc, _) = dokument_z("Ala ma kota.");
    let od = doc.anchor_at_char(7).unwrap();
    let do_ = doc.anchor_at_char(11).unwrap();

    doc.set_text(od, do_, "psa");

    assert_eq!(doc.text(), "Ala ma psa.");
    assert!(doc.undo());
    assert_eq!(doc.text(), "Ala ma kota.");
}

#[test]
fn skasowanie_calego_spanu_usuwa_go_z_dokumentu() {
    let mut doc = Document::new();
    doc.insert_segment(0, "abc", Anchor::Start, &bez_normalizacji());
    doc.insert_segment(1, "def", doc.end_anchor(), &bez_normalizacji());

    let od = doc.anchor_at_char(3).unwrap();
    let do_ = doc.anchor_at_char(6).unwrap();
    doc.set_text(od, do_, "");

    assert_eq!(doc.text(), "abc");
}

// --- własność: fold jest deterministyczny ---

#[test]
fn fold_daje_ten_sam_wynik_przy_kazdym_przeliczeniu() {
    let mut doc = Document::new();
    // Sekwencja mieszanych operacji: wstawienia na końcu, w środek i
    // edycje klawiaturą, z cofnięciami po drodze.
    for (seq, i) in (0..20u64).enumerate() {
        let seq = seq as u64;
        let anchor = if i % 3 == 0 {
            doc.end_anchor()
        } else {
            let len = doc.char_len();
            doc.anchor_at_char(len / 2).unwrap_or(Anchor::Start)
        };
        doc.insert_segment(seq, &format!("w{i} "), anchor, &normalizacja());
        if i % 5 == 4 {
            doc.undo();
        }
    }
    let tekst = doc.text();
    let spany: Vec<_> = doc.spans().to_vec();

    doc.refold();

    assert_eq!(doc.text(), tekst);
    assert_eq!(doc.spans(), spany.as_slice());
}

// --- opis zmiany dla warstwy widoku ---

#[test]
fn wstawienie_raportuje_dokladnie_to_co_widok_ma_zrobic_z_buforem() {
    // Widok nie może przerysowywać całego bufora przy każdej wypowiedzi —
    // zgubiłby zaznaczenie i pozycję karetki. Dostaje więc opis zmiany:
    // gdzie, ile znaków usunąć i co wstawić.
    let mut doc = Document::new();
    doc.insert_segment(0, "Zacząłem mówić…", Anchor::Start, &normalizacja());

    let zmiana = doc.insert_segment(1, "i skończyłem.", doc.end_anchor(), &normalizacja());

    // Wielokropek (1 znak) znika, w jego miejsce wchodzi spacja i tekst.
    assert_eq!(zmiana.at, "Zacząłem mówić".chars().count());
    assert_eq!(zmiana.removed, 1);
    assert_eq!(zmiana.inserted, " i skończyłem.");
    assert_eq!(doc.text(), "Zacząłem mówić i skończyłem.");
}

#[test]
fn kotwica_po_wstawieniu_stoi_tuz_za_wstawionym_tekstem() {
    let mut doc = Document::new();
    let zmiana = doc.insert_segment(0, "Ala ma kota.", Anchor::Start, &normalizacja());

    assert_eq!(
        doc.char_position(zmiana.anchor),
        "Ala ma kota.".chars().count()
    );
}

#[test]
fn kotwica_po_wstawieniu_w_srodek_stoi_przed_doklejona_spacja() {
    // Dalszy ciąg dyktowania należy tam, gdzie skończyła się mowa — a nie
    // za spacją, którą normalizacja dołożyła z tyłu.
    let mut doc = Document::new();
    doc.insert_segment(0, "Ala ma kota.", Anchor::Start, &normalizacja());
    let zmiana = doc.insert_segment(1, "zawsze", doc.anchor_at_char(3).unwrap(), &normalizacja());

    assert_eq!(doc.text(), "Ala zawsze ma kota.");
    assert_eq!(
        doc.char_position(zmiana.anchor),
        "Ala zawsze".chars().count()
    );
}
