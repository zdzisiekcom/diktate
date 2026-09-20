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
| Kreator setup / pobieranie modeli | `docs/04-konfiguracja.md#kreator-diktate-setup` |

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

Model whisper: `~/.diktate/models/ggml-small-q5_1.bin`, ścieżkę nadpisuje
`DIKTATE_MODEL`, a przed domyślną wchodzi pole `whisper.model` z `daemon.toml`.
Modele pobiera `diktate setup` (kreator) albo `diktate model pull <nazwa>`;
kuratorowana lista modeli żyje w `crates/diktate-cli/src/model/catalog.rs`
i to ją trzeba uzupełnić, dopisując nowy wariant. Diagnostyka bez mikrofonu:

```bash
cargo run -p diktate-core --example segment-wav -- testdata/jfk.wav   # gdzie tnie segmenter
cargo run -p diktate-core --example bench-model -- probka.wav 4       # koszt modelu, wczytany raz
./target/debug/diktate-cli record --wav testdata/jfk.wav --language en
python3 scripts/smoke-ipc.py         # test dymny protokołu, wymaga daemona
```

Daemon jest jeden na sesję (plik `$XDG_RUNTIME_DIR/diktate.pid`). Po zmianie
`daemon.toml` albo przebudowaniu binarki: `diktate-daemon --restart`. Zwykły
start przy działającym daemonie odmawia — bez tego starszy proces zostawał
w pamięci i zmiany wyglądały, jakby nie działały.

## Pułapki tego środowiska

Jedno obejście, które wygląda na dziwactwo, a jest konieczne — nie usuwaj go
bez sprawdzenia, że problem zniknął:

- **`openssl-sys` z cechą `vendored` w `[build-dependencies]`** rdzenia —
  `ort` (przez `voice_activity_detector`) ciągnie `native-tls` w swoim
  `build.rs`, a system nie ma nagłówków OpenSSL. Unifikacja cech w resolverze
  v2 jest osobna dla build-dependencies, więc wpis musi być właśnie tam.

Dwa wcześniejsze obejścia — `WHISPER_DONT_GENERATE_BINDINGS=1` w
`.cargo/config.toml` i łatka `[patch.crates-io]` na `vendor/whisper-rs-0.14.4`
— **zniknęły** razem z `whisper-rs`. Oba brały się z tego, że wiązania
rustowe zostawały w tyle za whisper.cpp: bindgen psuł się na strukturach
glibc, a `vulkan.rs` wołał symbole `ggml_backend_vk_*`, usunięte po przejściu
ggml na ogólny rejestr backendów. Dziś whisper.cpp jest submodułem, a wiązania
są własne i wąskie (patrz niżej), więc ta klasa problemów nie ma się gdzie
pojawić.

## whisper.cpp jako submoduł

Silnik żyje w `vendor/whisper.cpp` (submoduł przypięty do wydania **v1.9.4**)
i daje oba modele naraz: `libwhisper` i `libparakeet` dzielą jedną kopię ggml.
Po świeżym sklonowaniu repo trzeba raz:

```bash
git submodule update --init
```

`WHISPER_CPP_DIR=/ścieżka/do/klonu` nadpisuje ten wybór, gdy ktoś ma już
zbudowane drzewo i nie chce kompilować go po raz drugi.

Rust **nie odwzorowuje** struktur whisper.cpp. `whisper_full_params` ma 61 pól
i jest przekazywana przez wartość, więc każda zmiana pola w silniku psułaby
wiązania po cichu — bez błędu kompilacji, za to z przesuniętymi wartościami
w czasie pracy. Zamiast tego jest warstwa pośrednia w C++
(`crates/diktate-whispercpp-sys/src/shim.{h,cpp}`), a przez granicę FFI
przechodzą wyłącznie wskaźniki i liczby. Dokładając cokolwiek do API silnika,
dokłada się to tam, a nie w Ruście.

## Wydajność transkrypcji — liczby, o których łatwo zapomnieć

Whisper przetwarza zawsze okno **30 sekund**, więc koszt segmentu jest
zdominowany przez enkoder i prawie nie zależy od tego, ile w segmencie mowy.
„Prawie" — bo dekoder skaluje się z liczbą tokenów. Zmierzone dla
`small-q5_1` na Vulkanie, whisper.cpp **1.9.4**: 2 s materiału → 0,89 s,
3 s → 1,14 s, 5 s → 1,44 s. Wniosek jest mocniejszy, niż gdyby koszt był
idealnie stały: **krótkie segmenty się nie opłacają**, bo dwusekundowy
wycinek kosztuje ponad połowę tego, co pięciosekundowy, choć niesie 2,5×
mniej mowy.

Te liczby są nowe. Do wersji 1.7.4 (przez `whisper-rs`) te same segmenty
kosztowały 2,6 / 2,8 / 3,0 s — przejście na 1.9.4 dało **2,1–2,9× szybciej**,
przy tym samym modelu i tym samym GPU. Wnioski oparte na starych liczbach
trzeba przeliczyć, a nie przepisać.

Segmenter mimo to tnie płynnie, adaptacyjnym progiem ciszy (`docs/07`):
rozpędzona wypowiedź domyka się już po 200 ms pauzy, żeby tekst pojawiał się
w trakcie mówienia, a nie blokiem na końcu. O tempie decyduje przy tym
`long_segment_ms`, nie długość pauzy — po każdym cięciu segment rośnie od
zera, więc dopiero ten próg wyznacza, jak często tekst wychodzi.

### Modele na tym sprzęcie (Intel Meteor Lake, Vulkan)

Mierzone na 20 s polskiej mowy, model wczytany raz, jeszcze na whisper.cpp
1.7.4 (`cargo run -p diktate-core --example bench-model -- plik.wav 4`).
**Bezwzględne czasy są więc zawyżone ~2,5×** wobec dzisiejszego silnika;
proporcje między modelami zostają:

| model | rozmiar | wczytanie | przebieg | jakość polszczyzny |
|-------|---------|-----------|----------|--------------------|
| `base` | 141 MB | 0,5 s | ~2,1 s | słaba — myli słowa |
| `small-q5_1` | 181 MB | 0,6 s | ~4,5 s | dobra; **domyślny wybór** |
| `small` | 465 MB | — | ~4,9 s | jak wyżej, wolniej i 2,5× większy |
| `large-v3-turbo-q5_0` | 547 MB | 1,2 s | 14 → 20 s | najlepsza, ale nie do użycia |

Turbo przy ciągłej pracy **zwalnia** (14,0 → 16,4 → 19,3 → 20,0 s), dobijając
do czasu rzeczywistego — iGPU najpewniej ogranicza zegar pod obciążeniem. Im
dłużej ktoś mówi, tym bardziej transkrypcja zostaje w tyle, więc duży model
odpada tu mimo najlepszej jakości. Na mocniejszym GPU rachunek może wyjść
inaczej — wtedy trzeba go powtórzyć, a nie przyjąć te liczby na wiarę.

Kwantyzacja na Vulkanie **nie** spowalnia: `small-q5_1` jest szybsze od
`small`, a `base-q5_1` od `base`. Wersje `q5` są więc wyborem domyślnym.

### Dobieranie `long_segment_ms`

Segment musi kosztować mniej, niż trwa, inaczej kolejka rośnie i tekst
pojawia się **później**, nie wcześniej. Dla `small-q5_1` na 1.9.4 (~1,2 s na
segment) próg 2500 ms daje segmenty ~3,4 s, czyli ~35% przepustowości.
Na 1.7.4 było to ~85% i próg 2500 ms był granicą — dziś **nie jest**, więc
zejście niżej stało się możliwe bez zmiany modelu. Nie zostało zrobione:
zmiana rytmu segmentacji dotyka semantyki dyktowania (`docs/07`), a nie tylko
wydajności, i wymaga osobnej decyzji.

Każdy pomysł na zmianę rytmu segmentacji trzeba skonfrontować z obiema
liczbami naraz: kosztem przebiegu dla wybranego modelu i długością segmentu,
jaką dany próg faktycznie daje (`--example segment-wav` to pokazuje).

## Testy

Logika bez wejścia/wyjścia (segmenter, filtry, składanie ramek, resampling) ma
mieć testy jednostkowe działające **bez mikrofonu, bez modelu i bez GTK**.
Przy filtrach halucynacji ważniejsze od testów odrzucania są testy tego, co
odrzucone być **nie może** — filtr zjadający prawdziwy tekst jest gorszy od
przepuszczonego `[muzyka]`.
