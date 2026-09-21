# 06 — Interfejs graficzny

GTK4 + libadwaita. Trzy powierzchnie: okno dyktowania, ikona w tray-u, okno
ustawień.

## Okno dyktowania

Pojawia się w momencie rozpoczęcia nagrywania i zbiera transkrybowany tekst.

```
┌────────────────────────────────────────────────────┐
│ ● Nagrywam        large-v3-turbo · pl · Vulkan   ⚙ │
│ ▁▃▅█▅▃▁▁▂▄▆█▆▄▂▁                                   │
├────────────────────────────────────────────────────┤
│ To jest pierwszy segment, który już się            │
│ przetranskrybował i jest gotowy.                   │
│                                                    │
│ ███████ zaznaczony fragment ███████  zastępuję…    │
│                                                    │
│ ░░░ przetwarzam… ░░░                               │
├────────────────────────────────────────────────────┤
│ [Kopiuj] [Podyktuj ponownie] [Zapisz] [Odrzuć]     │
└────────────────────────────────────────────────────┘
```

`AdwWindow`, domyślnie zawsze na wierzchu (`window.always_on_top`).

**Wskaźnik poziomu dźwięku** rysowany z `audio.level` (dławione do ~20 Hz).
Pełni funkcję diagnostyczną: brak wychyleń natychmiast pokazuje, że wybrane
urządzenie wejściowe jest nie to, co trzeba — bez tego użytkownik dyktuje
w ciszę i dowiaduje się o tym po minucie.

Skala jest **decybelowa**, nie liniowa, i to nie jest szczegół kosmetyczny:
mowa przy normalnym nagrywaniu ma wartość skuteczną rzędu 0,02–0,2, więc
liniowy pasek stałby cały czas przy lewej krawędzi i nie odróżniałby cichej
mowy od wyłączonego mikrofonu — czyli nie robiłby tego jednego, do czego
służy. Dno skali to −60 dBFS. Wskaźnik pokazuje się tylko w trakcie
nagrywania.

**Segment w przetwarzaniu** pojawia się jako wyszarzony placeholder zaraz po
`segment.pending`, zanim przyjdzie `segment.added`. Bez tego przy zaległościach
w kolejce okno wygląda jak zawieszone.

**Segmenty o niskiej pewności** (`avg_logprob` poniżej progu) są wyróżniane
kolorem — sygnał, że warto przeczytać uważniej.

**Karetka jest punktem wstawiania.** Tekst z kolejnej wypowiedzi wchodzi tam,
gdzie stoi kursor, a po wstawieniu kursor stoi za nim — dyktowanie płynie dalej
od tego miejsca. Kliknięcie w środek notatki przenosi dyktowanie w środek
notatki. Okno nie przewija się do punktu wstawiania, jeśli użytkownik odjechał
widokiem w inne miejsce (`dictation.follow_caret`).

**Zaznaczenie znaczy „zastąp to"** — bez wchodzenia w żaden tryb. Zaznaczony
fragment zmienia kolor na `replace.highlight_color`, pojawia się etykieta
„zastępuję…", a następna wypowiedź wchodzi w jego miejsce. `Escape` znosi
zaznaczenie. Pełna semantyka: [07](07-zachowanie-dyktowania.md#zaznaczenie-i-zastępowanie).

**Pasek stanu** pokazuje model, język, backend i — gdy `backlog` przekroczy
`warn_backlog_at` — licznik zaległości.

### Przycisk korekty tekstu

Ikona `tools-check-spelling-symbolic` w pasku nagłówka (skrót `Super+D`)
wysyła **całą** notatkę
do lokalnego modelu językowego i wstawia poprawioną wersję. Nieczynna przy
pustej notatce i w trakcie trwającej korekty, niewidoczna przy
`correction.enabled = false` — guzik, który zawsze kończy się błędem sieci,
jest gorszy niż jego brak.

Stan idzie do podtytułu („Poprawiam tekst…"), tak jak każdy inny stan okna;
wynik — do `Toast`-a z odsetkiem zmienionych słów. Korekta wchodzi jako
**jedna operacja** modelu dokumentu, więc `Ctrl+Z` cofa ją w całości.

Zabezpieczenia i wszystkie pokrętła opisuje
[11-korekta-llm.md](11-korekta-llm.md#wdrożenie); najważniejsze z nich to
pytanie zadawane wtedy, gdy model zmienił więcej słów, niż przewiduje próg —
wtedy przestaje poprawiać błędy rozpoznania, a zaczyna redagować wypowiedź
własnymi słowami.

### Przycisk redakcji poleceniem głosowym

Ikona `document-edit-symbolic` obok korekty (skrót `Super+E`) otwiera okno
polecenia: użytkownik **mówi**, co zmienić — „zmień na bardziej formalny",
„usuń powtórzenia", „przepisz w czasie przyszłym" — a model wykonuje to na
zaznaczonym fragmencie albo, gdy nic nie jest zaznaczone, na całej notatce.

Okno polecenia trzyma się konwencji widoków: `adw::Window` z `ToolbarView`
i `HeaderBar`, tytuł „Polecenie" ze stanem w podtytule, pole jako `EntryRow`
w `PreferencesGroup`, której nagłówek mówi o celu („Zaznaczony fragment"
albo „Cała notatka"), a opis pokazuje jego początek. „Anuluj" po lewej,
„Wykonaj" (`suggested-action`) po prawej, `Escape` zamyka.

Wypowiedzi z transkrypcji trafiają przez ten czas **do pola polecenia, nie do
notatki** — to jedyne rozgałęzienie tekstu w oknie i mieszka po stronie
klienta; daemon nie wie o redakcji nic. Pole jest edytowalne: przekręcone
polecenie poprawia się szybciej palcem niż drugim podejściem.

Rozstrzyga o tym **oś czasu audio**, nie moment odebrania wiadomości: do
polecenia idą wyłącznie wypowiedzi, które padły po otwarciu okna. Zdanie
powiedziane wcześniej, a wychodzące z transkrypcji dopiero teraz, jest
treścią notatki — to ta sama reguła, co przy zastępowaniu zaznaczenia
([07-zachowanie-dyktowania.md](07-zachowanie-dyktowania.md)).

Dzięki temu redakcja działa **także w trakcie dyktowania**: przycisk jest
nieczynny tylko przy pustej notatce oraz w trakcie trwającej korekty albo
redakcji. Ukrywa go to samo `correction.enabled = false`, co przycisk
korekty. Okno polecenia włącza mikrofon, jeśli nie był włączony — i gasi go
przy zamknięciu tylko wtedy, gdy samo go włączyło; trwającego dyktowania nie
przerywa.

Reszta jest jak przy korekcie: stan w podtytule („Redaguję tekst…"), wynik
jako `Toast` z odsetkiem zmienionych słów, podmiana jako **jedna operacja**
modelu dokumentu, więc `Ctrl+Z` cofa ją w całości. Szczegóły i zabezpieczenia:
[11-korekta-llm.md](11-korekta-llm.md#redakcja-poleceniem-głosowym).

### Skróty w oknie

| Skrót | Akcja |
|-------|-------|
| `Ctrl+C` | Kopiuj całość do schowka |
| `Super+C` | Kopiuj i zamknij (przycisk „OK”) |
| `Ctrl+R` | Podyktuj ponownie zaznaczony fragment (jawne uzbrojenie; przy `implicit_replace` wystarczy zaznaczenie) |
| `Super+E` | Zredaguj poleceniem głosowym (zaznaczenie albo całą notatkę) |
| `Super+D` | Popraw tekst modelem językowym |
| `Ctrl+Z` / `Ctrl+Shift+Z` | Cofnij / ponów |
| `Ctrl+S` | Zapisz do historii |
| `Ctrl+Space` | Pauza / wznowienie nagrywania |
| `Ctrl+,` | Otwórz okno ustawień |
| `Escape` | Anuluj tryb zastępowania; ponownie — odrzuć i zamknij |

Wszystkie konfigurowalne w `gtk.toml`, sekcja `[shortcuts]`.

### „OK” — jedno domknięcie dyktowania

Zwykły przebieg to trzy ruchy: skrót globalny podnosi okno i włącza mikrofon,
użytkownik dyktuje, skrót `Super+C` domyka. Domknięcie robi wszystko naraz —
zatrzymuje nagrywanie, wkłada cały tekst do schowka i zamyka okno — więc
wklejenie w docelowej aplikacji jest następnym ruchem, a nie czwartym.

`Super+C` jest skrótem **lokalnym**: działa, gdy okno ma ognisko, i nie zabiera
kombinacji reszcie systemu. Globalny jest tylko ten jeden od nagrywania.

**Domknięcie czeka na ostatnie zdanie.** Skrót pada zwykle ułamek sekundy po
tym, jak użytkownik skończył mówić — a ostatnia wypowiedź jest wtedy jeszcze
w transkrypcji. Kopiowanie w tym momencie oddałoby schowkowi tekst bez
ostatniego zdania, czyli dokładnie to, czego nikt nie zauważy przed wklejeniem.
Żądanie jest więc **odkładane** tak samo jak żądanie nagrywania z pustego
biurka i wykonywane dopiero z powiadomienia `state.changed`, w którym sesja
jest bezczynna, a `backlog` zszedł do zera. Podtytuł przez ten czas mówi
„Przetwarzam N wypowiedzi…”, więc widać, na co okno czeka.

Powrót do mikrofonu (przycisk albo skrót globalny) **odwołuje** odłożone
domknięcie — inaczej okno zamykałoby się w środku następnego dyktowania.
Rozłączenie daemona domyka natychmiast tym, co zdążyło dojść: nie ma już kto
dosłać reszty, a wiszące okno byłoby gorsze od niepełnego schowka.

Daemon **zostaje** po zamknięciu okna. Kolejne naciśnięcie skrótu globalnego
podnosi okno na nowo bez czekania na wczytanie modelu.

Przycisk „Podyktuj ponownie" jest aktywny tylko przy niepustym zaznaczeniu
i przy `implicit_replace = true` pełni funkcję informacyjną — pokazuje, że
zaznaczenie jest uzbrojone. Ukrywanie go byłoby oszczędnością na czytelności:
mechanizm bez widocznego śladu w oknie jest mechanizmem, o którym nikt się nie
dowie.

Tekst jest **edytowalny również klawiaturą** — `GtkTextView` nie jest tylko do
odczytu. Edycje z klawiatury stają się operacjami `SetText` w tym samym logu,
co poprawki głosowe, więc `Ctrl+Z` działa na jedne i drugie jednolicie.

## Konwencje widoków

Obowiązują **każdy** widok w `diktate-gtk` — okno dyktowania, ustawienia,
przeglądarkę historii i cokolwiek dojdzie później. Nowy widok, który je łamie,
jest błędem, nawet jeśli działa.

**Okno bez osobnego paska tytułowego.** `adw::ApplicationWindow` z
`adw::ToolbarView` i `adw::HeaderBar` jako górnym paskiem. Nigdy
`gtk::ApplicationWindow` z `set_titlebar` — to daje odcięty pasek systemowy
zamiast paska wtopionego w okno, co od razu wygląda obco na GNOME. Wyjątek:
okno ustawień to `adw::PreferencesDialog` — ten typ niesie układ
`ToolbarView`/`HeaderBar` w sobie i jest natywnym wzorcem GNOME dla ustawień,
więc osobne składanie go od zera byłoby robieniem gorzej tego, co widżet już
robi dobrze. Reszta konwencji na tej liście obowiązuje go bez zmian.

**Motyw idzie za systemem.** `adw::StyleManager` z `ColorScheme::Default`.
Nie wymuszamy jasnego ani ciemnego, nie doklejamy własnych kolorów tła,
nie nadpisujemy palety Adwaity. Kolory bierzemy z klas nazwanych
(`suggested-action`, `destructive-action`, `error`, `warning`, `dim-label`),
nie z literałów.

**Tytuł dwupoziomowy zamiast paska stanu.** `adw::WindowTitle`: tytuł to nazwa
widoku, podtytuł to stan („Gotowe", „Nagrywam…", „Brak daemona"). Osobna
etykieta statusu pod treścią jest zabroniona — zjada miejsce i nie jest tam,
gdzie użytkownik GNOME jej szuka.

**Akcje jako ikony symboliczne w pasku**, z podpowiedziami (`tooltip`).
Podpisy tekstowe tylko tam, gdzie ikona byłaby zagadką. Nazwy ikon zawsze
z sufiksem `-symbolic`.

**Pusty stan to `adw::StatusPage`**, nie puste pole. Ikona, tytuł i jedno
zdanie mówiące, co zrobić. Przełączanie pusty/wypełniony przez `gtk::Stack`,
w obie strony — skasowanie całej treści wraca do stanu pustego.

**Potwierdzenia przez `adw::Toast`** w `adw::ToastOverlay`. Żadnych okien
modalnych na „skopiowano", żadnych niemych akcji bez śladu.

**Tekst w `adw::Clamp`.** Przy szerokim oknie kolumna tekstu nie rozciąga się
na całą szerokość — tak samo jak w Ustawieniach i w Tekstowym.

**Listy ustawień przez `adw::PreferencesGroup` i `adw::ActionRow`/`SwitchRow`**,
nie ręcznie składane `Box`-y z etykietami. Grupy dostają nagłówki.

**Teksty interfejsu po polsku**, z pełną ortografią. Nazwy widżetów, zmiennych
i klas CSS po angielsku — jak w reszcie kodu.

## Ikona w tray-u

Wymaganie: aplikacja ma rezydować w tray-u jako ikona.

GNOME nie ma natywnego obszaru powiadomień od wersji 3.26. Dostępne drogi to
`StatusNotifierItem` (protokół D-Bus obsługiwany przez rozszerzenie
AppIndicator, obecne w większości instalacji GNOME oraz natywnie w KDE, XFCE
i większości paneli w tiling WM) albo własne rozszerzenie GNOME Shell.

Wybrane: **`StatusNotifierItem` przez crate `ksni`**. Uzasadnienie — to samo
rozwiązanie działa poza GNOME, czyli dokładnie tam, gdzie aplikacja ma
działać. Własne rozszerzenie Shella działałoby wyłącznie w GNOME, wymagałoby
utrzymania kodu w JavaScripcie i przepisywania go przy każdej większej wersji
Shella.

Instalator wykrywa brak rozszerzenia AppIndicator i wypisuje instrukcję
instalacji. Aplikacja działa bez tray-a (sterowanie hotkeyem i `diktate-cli`),
ale z ostrzeżeniem przy starcie.

### Menu

```
◉ Rozpocznij dyktowanie          Ctrl+Alt+D
─────────────────────────────────
  Profil          ▸  pl · en · szybki
  Model           ▸  large-v3-turbo · medium · base
─────────────────────────────────
  Historia notatek
  Ustawienia
─────────────────────────────────
  Zakończ
```

Dzisiejsze menu ma z tego trzy pozycje: przełączenie dyktowania, „Pokaż
okno" i „Zakończ". Profile, wybór modelu, historia i ustawienia dochodzą
razem z etapami, które je wprowadzają — pozycja menu prowadząca donikąd jest
gorsza od jej braku. Tray żyje na własnym wątku i nie dotyka widgetów: menu
wysyła polecenie kanałem, a pętla GTK wykonuje je tą samą drogą co
kliknięcie w oknie.

Ikona zmienia się w trakcie nagrywania (`icon_recording`), a przy zaległościach
pokazuje licznik, jeśli `tray.show_backlog` jest włączone.

## Skrót globalny

Na Wayland żadna aplikacja nie przechwyci globalnego skrótu samodzielnie —
nie ma odpowiednika `XGrabKey`. Dostępne są dwie drogi.

**Portal `org.freedesktop.portal.GlobalShortcuts`** — rozwiązanie docelowe
i czyste, ale wsparcie w GNOME jest świeże, a zachowanie różni się między
kompozytorami.

**Skrót środowiska wywołujący polecenie** — użytkownik (albo instalator przez
`gsettings`) rejestruje własny skrót GNOME uruchamiający `diktate-cli toggle`.

Wybrane na start: **skrót środowiska uruchamiający `diktate-gtk --toggle`**.
Działa dzisiaj, w każdym środowisku, i pozwala związać cokolwiek — `sxhkd`,
Hyprland, KDE, i3. Portal zostaje dołożony później jako wygoda instalacji, nie
jako warunek działania.

Polecenie woła **okno**, a nie `diktate-cli`, bo jeden skrót ma zrobić obie
rzeczy naraz: przywołać okno na pierwszy plan i włączyć mikrofon. Naciśnięte
przy pustym biurku otwiera notatkę i od razu nagrywa; naciśnięte ponownie
kończy dyktowanie. Stoi na tym, że aplikacja jest pojedynczą instancją
(`HANDLES_COMMAND_LINE`): drugie uruchomienie nie tworzy drugiego okna, tylko
przekazuje argumenty do już działającego. `diktate-cli toggle` dochodzi
później dla zastosowań bez okna — jedno nie wyklucza drugiego, bo obie drogi
kończą się tą samą metodą IPC.

Rejestracja skrótu `Ctrl+Alt+D` w GNOME:

```bash
diktate-gtk --install-shortcut "<Control><Alt>d"   # bez argumentu: Ctrl+Alt+D
diktate-gtk --remove-shortcut
```

Polecenie modyfikuje `org.gnome.settings-daemon.plugins.media-keys
custom-keybindings`. Powtórne wywołanie nadpisuje istniejący wpis, zamiast
dokładać drugi.

### Skrót z pustego biurka uruchamia całą drogę

Skrót potrafi wystartować aplikację od zera: akcja `toggle-recording` odpala
się wtedy, zanim daemon odpowie, a bywa że przed jego uruchomieniem.
Przełączenie wtedy samego przycisku dawało stan pozorny — przycisk wyglądał
na wciśnięty, `session.start` nigdy nie szedł, a nagrywanie wymagało dwóch
naciśnięć.

Żądanie, które przyszło przed połączeniem, jest więc **odkładane** i wykonywane
po połączeniu — tą samą drogą co kliknięcie przycisku, czyli razem z ikoną
nagrywania, wskaźnikiem poziomu i podtytułem ze stanu sesji. Drugie naciśnięcie
w trakcie czekania odwołuje pierwsze, tak jak drugie kliknięcie. Podtytuł przez
ten czas mówi „Uruchamiam daemona — zaraz zacznę nagrywać…", a spinner obok
tytułu się kręci, więc widać, że żądanie nie przepadło.

Gdy połączenie się nie powiodło albo padło, odłożone żądanie **przepada** —
okno nie obiecuje nagrania, którego nie będzie.

## Okno ustawień

Na tym etapie **celowo minimalne**. Konfiguracja odbywa się w plikach TOML
([04-konfiguracja.md](04-konfiguracja.md)), a okno ustawień
(`crates/diktate-gtk/src/settings.rs`, `adw::PreferencesDialog`) pokrywa tylko
to, czego nie da się sensownie zrobić edytorem tekstu: szablon promptu
whispera, z którego trzeba **widzieć**, gdzie wchodzi kontekst, i próg jego
długości, plus ustawienia korekty LLM-em, które inaczej żyłyby tylko
w literałach TOML.

Dwie strony:

- **Transkrypcja** (`audio-input-microphone-symbolic`) — grupa „Prompt
  whispera" z wielolinijkową kartą tekstową i opisem tłumaczącym `{kontekst}`;
  grupa „Kontekst" ze `SpinRow` „Znaków przed kursorem" (0–1000, krok 50);
  grupa „Pliki" z wierszami otwierającymi `gtk.toml` i `daemon.toml`
  w domyślnym edytorze — reszta obu plików zostaje poza oknem.
- **Korekta** (`text-editor-symbolic`) — grupa „Serwer" (włącznik, adres,
  model), osobne grupy „Prompt korektora" i „Prompt redaktora" z kartami
  tekstowymi i opisem różnicy między nimi, grupa „Progi" z dwoma `SpinRow`
  odsetków zmian i jednym liczby słów. `api_key`, `temperature`, `max_tokens`,
  `timeout_ms` i szablony `user_template`/`redaction_user_template` zostają
  w pliku — to strojenie, nie ustawienia, i okno ma ich nie ruszać, żeby
  otwarcie go nigdy nie skasowało klucza API.

Zapis idzie do `gtk.toml` przez `GtkConfig::save()` — ten sam pisarz, którego
używa kreator `diktate setup` ([04](04-konfiguracja.md#kreator-diktate-setup)),
atomowo i z kopią `.bak`. Udany zapis zamyka okno, pokazuje `Toast`
„Zapisano ustawienia" w oknie głównym i od razu wysyła nowy kontekst do
daemona — zmiana obowiązuje od następnej wypowiedzi, bez restartu czegokolwiek.
Nieudany zapis zostawia okno ustawień otwarte, z nietkniętą treścią pól, i
pokazuje błąd `Toast`-em w **samym oknie ustawień**, nie w oknie głównym pod
spodem — tam akurat patrzy użytkownik w tej chwili. Ten toast ma przycisk
„Zamknij bez zapisywania": bez niego trwale nieudany zapis (katalog bez prawa
zapisu, dysk pełny) zostawiałby okno, którego nie da się zamknąć zwyczajnie,
bo jedyna zwykła droga zamknięcia to udany zapis.

Przycisk otwierający okno: `emblem-system-symbolic`, `pack_start` zaraz za
przyciskiem mikrofonu — prawa strona paska jest zajęta przez sześć akcji
i siódma się tam nie mieści. Skrót `Ctrl+,`, jak w całym GNOME.

Które opcje przenieść do GUI na stałe, zdecydujemy po okresie używania
aplikacji. Poza oknem zostają na razie: wybór urządzenia audio z listy,
pobieranie i przełączanie modeli (wymagają `model.list`/`model.pull` i
pisania po `daemon.toml` — osobna robota) i zakładka „Akcje głosowe" — edytor
reguł agenta ([05-agent.md](05-agent.md)). Wcześniejsze budowanie pełnego
panelu ustawień oznaczałoby utrzymywanie dwóch ścieżek konfiguracji dla
opcji, które być może nigdy nie zostaną dotknięte po pierwszym strojeniu.

## Uruchamianie daemona przez klienta

Wymaganie: użytkownik nie ma pamiętać o daemonie. Diktate jest dla niego
jedną aplikacją, a nie usługą plus okno.

Klient, który nie zastał gniazda, **uruchamia daemona sam** i ponawia próbę
połączenia z narastającym odstępem, aż gniazdo się pojawi (domyślnie do
30 sekund — tyle, ile potrafi trwać wczytanie modelu whispera, bo daemon
otwiera gniazdo dopiero po nim). Podtytuł mówi kolejno „Łączę z daemonem…",
„Uruchamiam daemona…" i wreszcie „Gotowe"; przycisk nagrywania jest
nieaktywny, dopóki nie ma z kim rozmawiać.

Łączenie idzie **osobnym wątkiem**: okno jest widoczne i odpowiada od
pierwszej chwili, a nie zastyga na czas wczytywania modelu.

Uruchomiony daemon dostaje własną grupę procesów i **przeżywa zamknięcie
okna** — kolejne uruchomienie klienta zastaje gotowe gniazdo i wczytany
model, czyli dyktowanie od razu, bez kilkunastosekundowej przerwy.

Plik wykonywalny wybierany jest w kolejności: `$DIKTATE_DAEMON`, katalog
obok klienta (`target/debug/diktate-gtk` uruchamia `target/debug/diktate-daemon`,
nie wersję z systemu), na końcu `PATH`.

Wyłączenie tego zachowania to `autostart = false` w `[daemon]` w `gtk.toml`
([04-konfiguracja.md](04-konfiguracja.md)) — sensowne tam, gdzie daemon
chodzi pod `systemd --user`.

## Zachowanie przy rozłączeniu daemona

Klient GTK, który stracił połączenie, pokazuje stan „rozłączony", zachowuje
dokument i próbuje ponownie z narastającym opóźnieniem. Po ponownym połączeniu
wykonuje `session.subscribe { since_seq }` i nadrabia zaległe segmenty
z journalu. Użytkownik nie traci materiału przy restarcie daemona.
