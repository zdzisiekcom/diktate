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

**Pasek stanu** pokazuje model, język, backend i — gdy `backlog` przekroczy
`warn_backlog_at` — licznik zaległości.

### Skróty w oknie

| Skrót | Akcja |
|-------|-------|
| `Ctrl+C` | Kopiuj całość do schowka |
| `Ctrl+Enter` | Kopiuj i zamknij |
| `Ctrl+R` | Podyktuj ponownie zaznaczony fragment |
| `Ctrl+Z` / `Ctrl+Shift+Z` | Cofnij / ponów |
| `Ctrl+S` | Zapisz do historii |
| `Ctrl+Space` | Pauza / wznowienie nagrywania |
| `Escape` | Anuluj tryb zastępowania; ponownie — odrzuć i zamknij |

Wszystkie konfigurowalne w `gtk.toml`, sekcja `[shortcuts]`.

Tekst jest **edytowalny również klawiaturą** — `GtkTextView` nie jest tylko do
odczytu. Edycje z klawiatury stają się operacjami `SetText` w tym samym logu,
co poprawki głosowe, więc `Ctrl+Z` działa na jedne i drugie jednolicie.

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

Wybrane na start: **`diktate-cli toggle` + skrót w ustawieniach środowiska**.
Działa dzisiaj, w każdym środowisku, i pozwala związać cokolwiek — `sxhkd`,
Hyprland, KDE, i3. Portal zostaje dołożony później jako wygoda instalacji, nie
jako warunek działania.

Instalator oferuje automatyczną rejestrację skrótu `Ctrl+Alt+D` w GNOME:

```bash
diktate-cli install-shortcut --keys "<Control><Alt>d"
```

Polecenie modyfikuje `org.gnome.settings-daemon.plugins.media-keys
custom-keybindings` i jest odwracalne przez `--remove`.

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
