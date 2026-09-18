# Diktate — reguły pracy w tym repozytorium

Lokalna transkrypcja mowy na tekst dla Linuksa. Rust, cargo workspace, daemon
plus klienci. Specyfikacja projektowa leży w `docs/` i jest **nadrzędna wobec
kodu**: jeśli kod robi coś inaczej, to kod jest do poprawy albo dokument do
świadomej zmiany — nie milczącego rozjazdu.

## Zanim zaczniesz zmieniać cokolwiek

| Zmieniasz | Przeczytaj najpierw |
|-----------|---------------------|
| Widok GTK | `docs/06-gui.md`, sekcja **Konwencje widoków** — obowiązuje każdy widok |
| Semantykę dyktowania | `docs/07-zachowanie-dyktowania.md` — serce aplikacji |
| Protokół IPC | `docs/02-protokol-ipc.md` — `diktate-proto` jest jedynym źródłem kształtu wiadomości |
| Model dokumentu | `docs/03-model-dokumentu.md` |
| Kolejność prac | `docs/09-roadmapa.md`, `docs/00-etap-0.md` |

## Interfejs graficzny — zasada nadrzędna

Każdy widok w `diktate-gtk` ma wyglądać jak natywna aplikacja GNOME.
Pełna lista konwencji: `docs/06-gui.md#konwencje-widoków`. W skrócie:
`adw::ApplicationWindow` + `ToolbarView` + `HeaderBar` (nigdy osobny pasek
tytułowy przez `set_titlebar`), `ColorScheme::Default` (motyw za systemem,
zero własnych kolorów), stan w podtytule `WindowTitle`, pusty stan jako
`StatusPage`, potwierdzenia jako `Toast`, tekst w `Clamp`, ikony symboliczne.

Nowy widok składany „po swojemu" z gołych `Box`-ów i `Label`-i jest błędem,
nawet jeśli działa.

## Język

- Komentarze, komunikaty błędów, logi i teksty interfejsu: **po polsku**,
  z pełną ortografią i znakami diakrytycznymi.
- Nazwy typów, funkcji, zmiennych i klas CSS: po angielsku.
- Komentarz `//!` na górze modułu wyjaśnia **zasadę działania**, a nie
  przepisuje kod linijka po linijce.

## Budowanie i uruchamianie

```bash
cargo build --workspace              # domyślnie backend Vulkan
cargo build --workspace --no-default-features   # wariant CPU
cargo test --workspace
cargo clippy --workspace --all-targets
```

Model whisper: `~/models/whisper/ggml-small.bin`, ścieżkę nadpisuje
`DIKTATE_MODEL`. Diagnostyka segmentacji bez mikrofonu:

```bash
cargo run -p diktate-core --example segment-wav -- testdata/jfk.wav
./target/debug/diktate-cli record --wav testdata/jfk.wav --language en
python3 scripts/smoke-ipc.py         # test dymny protokołu, wymaga daemona
```

## Pułapki tego środowiska

Trzy obejścia, które wyglądają na dziwactwa, a są konieczne — nie usuwaj ich
bez sprawdzenia, że problem zniknął:

- **`WHISPER_DONT_GENERATE_BINDINGS=1`** w `.cargo/config.toml` — bindgen
  0.71 z clang 22 i glibc 2.43 źle wylicza rozmiary struktur (`_IO_FILE`),
  co daje `E0080` w wygenerowanych bindingach.
- **`[patch.crates-io]` na `vendor/whisper-rs-0.14.4`** — oryginalny
  `vulkan.rs` woła symbole `ggml_backend_vk_*`, których whisper.cpp już nie
  eksportuje po przejściu na ogólny rejestr backendów. Bez tej łatki Vulkan
  się nie kompiluje, a bez Vulkana transkrypcja jest ~6× wolniejsza
  (3,1 s wobec 18,1 s na `testdata/jfk.wav`, model `small`).
- **`openssl-sys` z cechą `vendored` w `[build-dependencies]`** rdzenia —
  `ort` (przez `voice_activity_detector`) ciągnie `native-tls` w swoim
  `build.rs`, a system nie ma nagłówków OpenSSL. Unifikacja cech w resolverze
  v2 jest osobna dla build-dependencies, więc wpis musi być właśnie tam.

## Wydajność transkrypcji — liczba, o której łatwo zapomnieć

Whisper przetwarza zawsze okno **30 sekund**, więc koszt jednego segmentu jest
stały niezależnie od jego długości: dwusekundowy urywek kosztuje tyle samo, co
dziesięciosekundowa wypowiedź. Stąd segmenter zbiera materiał (`target_speech_ms`)
zamiast ciąć na każdej pauzie. Każdy pomysł „tnijmy drobniej, będzie płynniej"
trzeba konfrontować z tą liczbą.

## Testy

Logika bez wejścia/wyjścia (segmenter, filtry, składanie ramek, resampling) ma
mieć testy jednostkowe działające **bez mikrofonu, bez modelu i bez GTK**.
Przy filtrach halucynacji ważniejsze od testów odrzucania są testy tego, co
odrzucone być **nie może** — filtr zjadający prawdziwy tekst jest gorszy od
przepuszczonego `[muzyka]`.
