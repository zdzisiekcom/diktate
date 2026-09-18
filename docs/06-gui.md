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

### Skróty w oknie

| Skrót | Akcja |
|-------|-------|
| `Ctrl+C` | Kopiuj całość do schowka |
| `Ctrl+Enter` | Kopiuj i zamknij |
| `Ctrl+R` | Podyktuj ponownie zaznaczony fragment (jawne uzbrojenie; przy `implicit_replace` wystarczy zaznaczenie) |
| `Ctrl+Z` / `Ctrl+Shift+Z` | Cofnij / ponów |
| `Ctrl+S` | Zapisz do historii |
| `Ctrl+Space` | Pauza / wznowienie nagrywania |
| `Escape` | Anuluj tryb zastępowania; ponownie — odrzuć i zamknij |

Wszystkie konfigurowalne w `gtk.toml`, sekcja `[shortcuts]`.

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
zamiast paska wtopionego w okno, co od razu wygląda obco na GNOME.

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

## Okno ustawień

Na tym etapie **celowo minimalne**. Konfiguracja odbywa się w plikach TOML
([04-konfiguracja.md](04-konfiguracja.md)), a okno ustawień pokrywa tylko to,
czego nie da się sensownie zrobić edytorem tekstu:

- wybór urządzenia audio z listy (z podglądem poziomu na żywo)
- pobieranie i przełączanie modeli (pasek postępu)
- przycisk „Otwórz plik konfiguracyjny" dla reszty
- zakładka „Akcje głosowe" — edytor reguł agenta ([05-agent.md](05-agent.md))

Które opcje przenieść do GUI na stałe, zdecydujemy po okresie używania
aplikacji. Wcześniejsze budowanie pełnego panelu ustawień oznaczałoby
utrzymywanie dwóch ścieżek konfiguracji dla opcji, które być może nigdy nie
zostaną dotknięte po pierwszym strojeniu.

## Zachowanie przy rozłączeniu daemona

Klient GTK, który stracił połączenie, pokazuje stan „rozłączony", zachowuje
dokument i próbuje ponownie z narastającym opóźnieniem. Po ponownym połączeniu
wykonuje `session.subscribe { since_seq }` i nadrabia zaległe segmenty
z journalu. Użytkownik nie traci materiału przy restarcie daemona.
