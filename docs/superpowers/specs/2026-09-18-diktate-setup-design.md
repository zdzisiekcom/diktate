# `diktate setup` — kreator konfiguracji i pobieranie modeli

Data: 2026-09-18. Stan: design zatwierdzony do spisania planu.

## Problem

Pierwsze uruchomienie Diktate wymaga dziś trzech rzeczy zrobionych ręcznie:
pobrania modelu ggml z Hugging Face, położenia go w miejscu, którego szuka
`default_model_path`, i napisania `daemon.toml` z progami, których sensowne
wartości znane są tylko z `CLAUDE.md`. Każda z nich jest pułapką osobno,
a razem tworzą próg wejścia większy niż sama aplikacja.

`diktate-config` tego nie rozwiązuje i nie ma rozwiązywać: to biblioteka
opisująca **kształt** konfiguracji — struktury, wartości domyślne, walidację,
wczytywanie. Niczego nie pyta i niczego nie zapisuje. Kreator jest jej
klientem, a nie jej zamiennikiem: czyta przez nią stan bieżący, pyta
użytkownika, serializuje z powrotem. Dzięki temu nie istnieje druga lista pól
do utrzymania.

## Zakres

Dwie nowe podkomendy w `diktate-cli`:

- `diktate setup [--advanced]` — dialog w terminalu, od modelu po zapis
  obu plików konfiguracyjnych,
- `diktate model list` i `diktate model pull <nazwa>` — to samo pobieranie
  bez dialogu; komenda zapowiedziana już w `docs/01-architektura.md`
  i `docs/08-dystrybucja-i-testy.md`.

Poza zakresem: kreator graficzny w GTK (okno ustawień z `docs/06` zostaje
minimalne), przełączanie modelu w locie (`model.load`), test mikrofonu
z transkrypcją.

## Zmiany w specyfikacji

Katalog modeli to **`~/.diktate/models/`**. `docs/01-architektura.md:204`
mówi dziś `$XDG_DATA_HOME/diktate/models/` — to zdanie zmieniamy świadomie,
razem z wdrożeniem. Powód jest praktyczny: katalog ma być przewidywalny i
łatwy do wskazania w rozmowie, a modele i tak nie są danymi, które ktokolwiek
chciałby synchronizować przez XDG.

Kolejność szukania modelu pozostaje: `$DIKTATE_MODEL`, potem pole
`whisper.model` z `daemon.toml`, potem domyślna ścieżka. Zmienia się tylko ta
ostatnia — z `~/models/whisper/ggml-small.bin` na
`~/.diktate/models/ggml-small-q5_1.bin`. Setup wykrywa modele leżące
w starym `~/models/whisper` i proponuje użycie ich stamtąd zamiast pobierania
tego samego pliku drugi raz; nie przenosi ich sam, bo cudzy katalog to nie
jego własność.

## Budowa

```
crates/diktate-cli/src/
  main.rs            # clap: record | setup | model
  model/
    catalog.rs       # tabela znanych modeli
    store.rs         # ~/.diktate/models, skan dysku, ścieżki
    download.rs      # pobieranie z weryfikacją SHA-256
  setup/
    dialog.rs        # zadawanie pytań, wartości domyślne, Enter
    profile.rs       # profil rytmu → progi VAD
    diff.rs          # lista różnic wobec stanu bieżącego
    write.rs         # serializacja z komentarzami, kopia .bak
```

Podział jest po odpowiedzialnościach, nie po wygodzie: `catalog` to dane,
`download` to wejście/wyjście, `profile` i `diff` to czysta logika (a więc
testowalna bez sieci i bez terminala), `dialog` to jedyne miejsce, które
czyta ze standardowego wejścia.

### Katalog modeli

Tabela stałych w `catalog.rs`, jeden wpis na model:

| pole | znaczenie |
|------|-----------|
| `name` | `small-q5_1` — nazwa w komendach i w dialogu |
| `file` | `ggml-small-q5_1.bin` — nazwa pliku na dysku i w repo HF |
| `bytes` | rozmiar, do paska postępu i do pokazania przed pobraniem |
| `sha256` | suma kontrolna |
| `run_ms` | zmierzony czas przebiegu na tym sprzęcie (`CLAUDE.md`) |
| `polish` | ocena polszczyzny: słaba / dobra / najlepsza |
| `recommended` | znacznik na jednej pozycji (`small-q5_1`) |

Źródło plików: `https://huggingface.co/ggerganov/whisper.cpp/resolve/main/<file>`.
Dodanie modelu to jedna linijka. Lista jest krótka i kuratorowana celowo —
pełny spis z API HF zawiera warianty, które na tym sprzęcie nie mają sensu,
a pytanie do sieci przy samym wyświetleniu listy psuje kreator bez internetu.

Przy wyświetlaniu `store` skanuje `~/.diktate/models` (i stary katalog) i
oznacza pozycje już obecne — wybór takiej nie pobiera niczego.

### Pobieranie

`ureq` z rustls. Wybór podyktowany `CLAUDE.md`: OpenSSL w tym środowisku nie
ma nagłówków, a rdzeń już raz musiał to obchodzić wpisem `vendored`
w `[build-dependencies]`. Rustls nie dotyka tego problemu wcale.

Przebieg: pobranie do `<file>.part`, wznawianie przerwanego pobrania
nagłówkiem `Range` gdy `.part` istnieje, pasek postępu w terminalu,
po zakończeniu suma SHA-256, dopiero potem zmiana nazwy na docelową.
Zerwane łącze zostawia `.part`, nigdy pliku, który wygląda na kompletny
model — półpobrany `ggml-*.bin` jest gorszy niż jego brak, bo whisper
przewraca się na nim komunikatem, który niczego nie tłumaczy.

Warstwa HTTP jest za traitem (`trait Source { fn get_range(...) }`), żeby
testy pobierania działały na lokalnym pliku, bez sieci.

### Dialog — krótka ścieżka

Sześć pytań, każde z domyślną odpowiedzią pod Enterem:

1. **Model** — lista z rozmiarem, czasem i oceną polszczyzny; domyślnie
   rekomendowany albo ten już ustawiony w `daemon.toml`.
2. **Język** — domyślnie `pl`.
3. **Urządzenie wejściowe** — lista z `cpal`, domyślnie `default`.
   Setup wypisuje urządzenia, ale niczego nie nagrywa.
4. **Rytm segmentacji** — jeden wybór z trzech profili, patrz niżej.
5. **Ikona w zasobniku** — tak/nie (`tray.enabled`).
6. **Skrót globalny** — z przypomnieniem, że rejestruje go środowisko
   graficzne, nie plik konfiguracyjny.

`--advanced` przechodzi zamiast tego przez **każde** pole `DaemonConfig`
i `GtkConfig` po kolei, z bieżącą wartością w nawiasie i jednozdaniowym
opisem wziętym z `docs/04-konfiguracja.md`.

### Profile rytmu

Profil ustawia trójkę pól naraz, bo w oderwaniu od siebie te liczby nie
znaczą nic:

| profil | `long_segment_ms` | `silence_ms_to_end_long` | `silence_ms_to_end` |
|--------|-------------------|--------------------------|---------------------|
| szybki | 2000 | 200 | 1200 |
| zrównoważony (domyślny) | 2500 | 200 | 1500 |
| dokładny | 4000 | 400 | 2000 |

Profil „szybki" schodzi poniżej progu przepustowości wyliczonego w
`CLAUDE.md` dla `small-q5_1` (segment kosztuje wtedy więcej, niż trwa, więc
kolejka rośnie). Kreator wybiera go tylko wtedy, gdy wybrany model jest
szybszy — inaczej pokazuje ten profil z ostrzeżeniem, co dokładnie się stanie.

Wybór profilu innego niż domyślny nie jest ostatecznym słowem — `--advanced`
nadal pozwala ustawić każdą z tych liczb osobno, a plik napisany ręcznie
kreator odczyta jako „profil własny" i nie podmieni go po cichu.

### Zapis

Kolejność jest istotna:

1. wczytanie stanu bieżącego przez `DaemonConfig::load` / `GtkConfig::load`
   (brak pliku to wartości domyślne, nie błąd),
2. dialog zmienia kopię tego stanu,
3. `validate()` na wyniku — kreator nie potrafi zapisać pliku, który
   zablokowałby start daemona,
4. lista różnic na ekranie: `vad.long_segment_ms: 2500 → 4000`,
5. pytanie o potwierdzenie,
6. kopia `daemon.toml.bak`, zapis nowego pliku.

Zapisywany plik zawiera polskie komentarze nad każdą sekcją i każdym polem —
ten sam tekst, który opisuje pola w `docs/04`. Plik po kreatorze ma nadawać
się do dalszej ręcznej edycji, a nie być zrzutem serializera.

Ręczne komentarze użytkownika **nie są** zachowywane: zapis nadpisuje plik
w całości, od tego jest `.bak`. Świadoma rezygnacja z `toml_edit` —
zachowanie cudzego układu kosztuje osobną ścieżkę zapisu, a plik i tak jest
w całości generowalny.

### Finał

Podsumowanie ze ścieżkami zapisanych plików i katalogu modeli, przypomnienie
o `diktate setup --advanced`. Jeśli żyje daemon, kreator mówi to wprost
i proponuje `diktate-daemon --restart` — bez tego zmiany wyglądają, jakby nie
działały (`CLAUDE.md`, „Budowanie i uruchamianie").

Wykrycie daemona wymaga pliku PID, który dziś leży w `diktate-daemon`
(binarka, nie da się z niej importować). `pidfile.rs` przenosimy do
`diktate-proto`, obok `socket_path()` — proto trzyma już lokalizacje
sesyjne, więc to jest jego miejsce, a nie nowy dom wymyślony na tę potrzebę.
Dublowanie odczytu PID w dwóch miejscach byłoby gorsze.

## Testy

Bez mikrofonu, bez modelu, bez sieci, bez GTK:

- **katalog**: nazwy unikalne, dokładnie jedna pozycja rekomendowana,
  każda suma SHA-256 ma poprawną długość,
- **profile**: każdy profil przechodzi `DaemonConfig::validate()`;
  rozpoznanie pliku spoza profili jako „własny",
- **różnice**: brak zmian daje pustą listę; zmiana jednego pola daje
  dokładnie jedną pozycję z obiema wartościami,
- **zapis**: round-trip — wygenerowany tekst wczytany przez `from_toml`
  daje tę samą strukturę; komentarze nie psują parsowania,
- **pobieranie**: na `Source` z lokalnego pliku — pełne pobranie,
  wznowienie z istniejącego `.part`, wykrycie niezgodnej sumy (plik
  docelowy nie powstaje),
- **skan dysku**: model w nowym katalogu, model w starym katalogu,
  brak obu.

Dialog sam w sobie nie jest testowany jednostkowo — jest cienki i
sprowadza się do wywołań przetestowanej logiki.

## Kolejność prac

1. `pidfile.rs` → `diktate-proto`, daemon korzysta z nowego miejsca.
2. `model/store.rs` + zmiana `default_model_path` na `~/.diktate/models`.
3. `model/catalog.rs` z tabelą.
4. `model/download.rs` za traitem `Source`.
5. `diktate model list` / `model pull`.
6. `setup/profile.rs`, `setup/diff.rs`, `setup/write.rs`.
7. `setup/dialog.rs` i podkomenda `setup`.
8. Aktualizacja `docs/01`, `docs/04`, `docs/08`, `README.md`, `CLAUDE.md`.
