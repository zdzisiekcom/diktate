# 09 — Roadmapa

Kolejność wynika z jednej zasady: **każdy etap kończy się czymś, czego da się
realnie używać**. Nie budujemy infrastruktury pod funkcje, których jeszcze nie
potrzebujemy.

## Etap 1 — Dyktowanie do schowka

Cel: naciskam skrót, mówię, tekst pojawia się w oknie, kopiuję go.

- `diktate-core`: capture (cpal), ring buffer, pre-roll, VAD (Silero), kolejka
  segmentów, whisper-rs z backendem Vulkan, filtry halucynacji
- `diktate-proto` + serwer JSON-RPC na gnieździe uniksowym
- `diktate-daemon` z journalem
- `diktate-cli`: `toggle`, `start`, `stop`, `status`, `devices`,
  `model pull/list`, `config check`
- `diktate-gtk`: okno dyktowania, wskaźnik poziomu, kopiowanie, tray (`ksni`)
- `diktate-document`: spany, append, `Ctrl+Z`
- Konfiguracja: `daemon.toml`, `gtk.toml`
- Skrót globalny przez `gsettings`

Po tym etapie aplikacja jest użyteczna na co dzień. Pozwala to strojić progi
VAD i prompt na prawdziwym materiale, zanim zbudujemy cokolwiek na wierzchu.

## Etap 2 — Redakcja głosem

Cel: zaznaczam fragment, dyktuję go od nowa, zostaje zastąpiony.

- Pełny model dokumentu: kotwice, `Splice`, log operacji, redo
- Tryb zastępowania w oknie GTK (`Ctrl+R`, rozstrzyganie po `audio_start_ms`)
- Edycja klawiaturą jako operacje w tym samym logu
- Zapis historii: `raw.ndjson`, `ops.ndjson`, `note.md`
- Przeglądarka historii w tray-u

Etap 2 jest osobny od etapu 1 celowo: model kotwic ma sens dopiero wtedy, gdy
istnieje strumień segmentów, na którym widać wyścig między appendem
a zaznaczeniem.

## Etap 3 — Komendy głosowe i aktywacja głosem

Cel: „zdzisław, zgaś światło" uruchamia skrypt, „dyktuj" otwiera okno.

- `diktate-agent`: subskrypcja, dopasowywanie (contains/regex/fuzzy/exact)
- Typy akcji: `exec`, `shell`, `ipc`, `notify`, `http`
- Bezpieczeństwo: `confirm`, `--dry-run`, log audytu, prawa pliku
- Zakładka „Akcje głosowe" w ustawieniach GTK z zapisem `agent.toml`
- Jednostka systemd dla agenta
- Tryb czuwania w daemonie: VAD + model `tiny`, dopasowanie fraz
  aktywujących, potwierdzanie modelem głównym przy niskiej pewności
- Wskaźnik czuwania w tray-u i wyłącznik jednym kliknięciem

Wake word i agent są **jednym etapem celowo**: dzielą mechanizm rozmytego
dopasowywania fraz i konsumują ten sam strumień segmentów. Budowanie ich
osobno oznaczałoby napisanie tej samej logiki dwa razy. Szczegóły w
[10-wake-word.md](10-wake-word.md).

## Etap 4 — Wpisywanie do aktywnego okna

Cel: tekst trafia bezpośrednio tam, gdzie jest kursor.

- Wpisywanie przez `ydotool` (wymaga `/dev/uinput` i uprawnień)
- Tryb bezpośredni: bez okna, tekst leci do aktywnej aplikacji na bieżąco
- Rozstrzygnięcie zachowania przy poprawkach — w trybie bezpośrednim nie ma
  zaznaczenia po stronie aplikacji, więc `Ctrl+R` wymaga innej semantyki niż
  w etapie 2

To etap z największą liczbą niewiadomych dotyczących środowiska. Dlatego jest
późno — do tego czasu reszta jest już sprawdzona i ewentualne problemy
z `uinput` nie blokują niczego innego.

## Etap 5 — Redakcja LLM-em

Cel: surowa transkrypcja zamieniana w tekst do przeczytania.

- Klient HTTP do lokalnego LLM-a (API zgodne z OpenAI: Ollama,
  `llama-server`), streaming SSE
- Redakcja jako operacje `Splice` w logu — odwracalna `Ctrl+Z`
- Podgląd zmian przed zastosowaniem
- Szablony promptów redakcyjnych per profil
- Rozstrzygnięcie punktów z [07-zachowanie-dyktowania.md](07-zachowanie-dyktowania.md)

## Poza roadmapą

Rozważane i odrzucone na tym etapie, z powodami:

| Funkcja | Powód odrzucenia |
|---------|------------------|
| Flatpak | Sandbox koliduje z NPU, `ydotool` i agentem |
| Diaryzacja mówców | Inny przypadek użycia (spotkania), inny stos, osobny projekt |
| Transkrypcja plików wsadowo | Istnieją lepsze narzędzia; nie jest to dyktowanie |
| Synchronizacja notatek | Historia to zwykłe pliki — wystarczy Syncthing albo git |
| Klient webowy | Daemon jest lokalny; most WebSocket dałoby się dołożyć, gdyby zaszła potrzeba |
| Windows / macOS | `cpal` i GTK by zadziałały, ale tray, skróty i integracja to osobny projekt |
