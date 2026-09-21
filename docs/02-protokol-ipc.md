# 02 — Protokół IPC

## Transport

Gniazdo uniksowe w `$XDG_RUNTIME_DIR/diktate.sock`, prawa `0600`. Ramkowanie:
**NDJSON** — jedna wiadomość JSON na linię. Protokół: **JSON-RPC 2.0**
z powiadomieniami w obie strony.

### Dlaczego nie D-Bus i nie HTTP

D-Bus byłby najbardziej „gnomowy" (aktywacja usługi, sygnały za darmo), ale
wymaga sesyjnego busa, co na headless serwerze albo w gołym tmuksie jest
uciążliwe — a właśnie tam ma działać `diktate-agent` i `diktate-cli`.

HTTP + WebSocket działałby zdalnie od ręki, ale nadkłada cały stos HTTP,
wymaga wymyślenia autoryzacji i otwiera port TCP widoczny dla każdego procesu
użytkownika.

Gniazdo uniksowe daje prawa dostępu z systemu plików za darmo, jest
debugowalne przez `socat`, działa na każdym Linuksie i BSD, a dołożenie
nasłuchu TCP albo mostu WebSocket później to kilkanaście linii — bez
przepisywania rdzenia.

## Wersjonowanie

Pierwsza wiadomość od klienta to `hello`. Daemon odrzuca połączenie
z niekompatybilną wersją głównego numeru protokołu.

```json
→ {"jsonrpc":"2.0","id":1,"method":"hello","params":{"proto":"1.0","client":"diktate-gtk/0.1.0"}}
← {"jsonrpc":"2.0","id":1,"result":{"proto":"1.0","daemon":"0.1.0","state":"idle"}}
```

## Metody (klient → daemon)

### Sterowanie sesją

| Metoda | Parametry | Wynik |
|--------|-----------|-------|
| `session.start` | `{ profile?: string }` | `{ session_id, started_at }` |
| `session.stop` | `{ session_id? }` | `{ session_id, segment_count }` |
| `session.toggle` | — | `{ session_id, state }` |
| `session.status` | — | `{ state, session_id?, backlog, model, language }` |
| `wake.set` | `{ enabled: bool }` | `{ state }` — włącza/wyłącza tryb czuwania |

`session.toggle` istnieje po to, żeby skrót klawiaturowy GNOME mógł być
związany z jednym poleceniem: `diktate-cli toggle`.

`state` przyjmuje wartości `idle`, `listening` (czuwanie na frazę aktywującą),
`recording` i `degraded`. `wake.set` pozwala wyłączyć nasłuch jednym
kliknięciem z menu tray-a — patrz [10-wake-word.md](10-wake-word.md#prywatność).

### Subskrypcja

| Metoda | Parametry | Wynik |
|--------|-----------|-------|
| `session.subscribe` | `{ since_seq?: u64 }` | `{ session_id?, segments: [...], next_seq }` |
| `session.unsubscribe` | — | `{}` |

`since_seq` pozwala dołączyć w trakcie: daemon odsyła w wyniku wszystkie
segmenty o numerze większym niż podany (z journalu), a od tego momentu
przysyła powiadomienia. Bez `since_seq` klient dostaje pełny snapshot
bieżącej sesji.

### Kontekst transkrypcji

| Metoda | Parametry | Wynik |
|--------|-----------|-------|
| `session.context` | `{ text: string }` | `{ chars }` |

Klient przysyła ogon tekstu **poprzedzającego punkt wstawiania** (a przy
uzbrojonym zastąpieniu — poprzedzającego zaznaczenie), już sklejony z szablonu
promptu i obcięty do `[whisper] context_chars` z `gtk.toml`
([04](04-konfiguracja.md#gtktoml)) — to klient zna dokument i wie, gdzie
w nim kończy się szablon, a zaczyna kontekst, więc obcinanie do długości robi
on, nie daemon. Daemon trzyma otrzymany tekst jako `prompt` w slocie
połączenia i podaje go silnikowi przy każdym kolejnym segmencie; jedyne, co
sam sobie dolicza, to awaryjne obcięcie promptu do `whisper.context.max_prompt_chars`
z `daemon.toml`, na wypadek gdyby ktoś przysłał więcej, niż rozsądnie
zmieści się w oknie whispera.

Wywołanie jest **podpowiedzią, nie zobowiązaniem**: daemon transkrybuje
normalnie także wtedy, gdy nikt kontekstu nie przysłał, a pusty `text`
czyści slot. Kierunek jest jednostronny — daemon nie zna dokumentu i nie
próbuje go poznać; dostaje tylko ciąg znaków do promptu. Branie ogona
z własnego logu transkrypcji zamiast od klienta (`whisper.context.source =
"transcript"`) zostaje pomysłem na przyszłość — dziś nie ma journalu,
z którego dałoby się taki ogon wziąć.

Powód, dla którego kontekst pochodzi od klienta, a nie z logu transkrypcji:
w logu jest to, co whisper *usłyszał*, a w dokumencie to, co użytkownik
*zatwierdził*. Po poprawieniu błędnie rozpoznanego nazwiska model ma widzieć
wersję poprawioną. Szczegóły w
[07](07-zachowanie-dyktowania.md#kontekst-dla-whispera).

`Request::params` niesie to razem z `session.subscribe` w jednym typie,
`RequestParams` (`#[serde(untagged)]`) — JSON-RPC nie ma znacznika wariantu,
więc rozstrzyga kształt obiektu. Oba warianty, `Subscription` i
`ContextParams`, mają `deny_unknown_fields`: bez tego `Subscription` (same
pola z wartością domyślną) dopasowałaby się do dowolnego obiektu, w tym do
`{"text": "…"}`, i kontekst zniknąłby po cichu jako pusta subskrypcja.

### Modele i konfiguracja

| Metoda | Parametry | Wynik |
|--------|-----------|-------|
| `model.list` | — | `{ installed: [...], available: [...] }` |
| `model.pull` | `{ name }` | `{ job_id }` — postęp przez `model.progress` |
| `model.load` | `{ name }` | `{ loaded, backend }` |
| `config.get` | `{ section? }` | aktualna konfiguracja daemona |
| `config.reload` | — | `{ ok }` — wymusza ponowne wczytanie `daemon.toml` |
| `profile.set` | `{ name }` | `{ language, prompt }` |
| `devices.list` | — | `{ devices: [...] }` |

`config.get` zwraca konfigurację tylko do odczytu. Zapis konfiguracji odbywa
się **przez plik**, nie przez IPC — daemon przeładowuje ją przez inotify.
Trzymanie jednego kierunku zapisu eliminuje klasę błędów, w których GUI
i plik rozjeżdżają się ze sobą.

## Powiadomienia (daemon → klient)

### `segment.added`

Najważniejsze zdarzenie w całym systemie.

```json
{
  "jsonrpc": "2.0",
  "method": "segment.added",
  "params": {
    "session_id": "01JF7X…",
    "seq": 42,
    "text": "To jest treść tego, co powiedziałem.",
    "audio_start_ms": 18340,
    "audio_end_ms": 21120,
    "received_at": "2026-09-18T10:14:02.481Z",
    "language": "pl",
    "avg_logprob": -0.21,
    "no_speech_prob": 0.01
  }
}
```

`audio_start_ms` jest liczony od początku sesji i jest **jedyną prawdziwą osią
czasu**. Kolejność przybycia do klienta może się różnić od kolejności
wypowiedzi przy zaległościach w kolejce, więc klient porządkuje po
`audio_start_ms`, a rozstrzyga o trybie edycji (wstawienie vs. zastąpienie) także
po nim. Szczegóły w [03](03-model-dokumentu.md).

`seq` jest monotoniczny i służy wyłącznie do nadrabiania zaległości
(`since_seq`).

`avg_logprob` i `no_speech_prob` pochodzą z whispera i pozwalają klientowi
oznaczyć segmenty o niskiej pewności (np. innym kolorem).

`language` przy `[whisper].language = "auto"` bywa zmyślony — whisper na szumie
potrafi „wykryć" język, którego nie było. Segmenty z językiem spoza
`filters.allowed_languages` daemon odrzuca jako `segment.dropped`
z `reason = "language"`, zamiast podawać je klientowi.

### Pozostałe

| Powiadomienie | Znaczenie |
|---------------|-----------|
| `segment.pending` | Segment zamknięty przez VAD, czeka lub jest liczony. Pozwala pokazać placeholder |
| `segment.dropped` | Segment odrzucony przez filtr (`reason`: `pattern`, `no_speech`, `logprob`, `too_short`, `language`) |
| `state.changed` | `{ state, backlog, backend, model, warning? }` |
| `audio.level` | `{ rms, peak }` — do wizualizacji, dławione do ~20 Hz |
| `model.progress` | `{ job_id, downloaded, total }` |
| `wake.detected` | `{ phrase, text, score, kind }` — `kind` to `"dictation"` lub `"agent"` |
| `error` | `{ code, message, fatal }` |

`audio.level` jest wysyłany **wyłącznie** do klientów, które o to poprosiły
(`session.subscribe { levels: true }`) — `diktate-agent` i `diktate-cli` nie
mają powodu odbierać dwudziestu wiadomości na sekundę.

## Stany daemona

```
idle ──session.start──► recording ──VAD──► recording (segmenty lecą)
  ▲                         │
  └──────session.stop───────┘

listening ──fraza dyktowania──► recording      (wakeword.enabled = true)
    ▲                               │
    └───────session.stop────────────┘

recording + kolejka pełna  → state = "recording", backlog = N
błąd backendu przy starcie → state = "degraded", warning = "…"
```

## Obsługa błędów

Kody błędów JSON-RPC w zakresie aplikacyjnym (`-32000` w dół):

| Kod | Znaczenie |
|-----|-----------|
| `-32000` | Brak wczytanego modelu |
| `-32001` | Urządzenie audio niedostępne |
| `-32002` | Sesja już trwa / nie trwa |
| `-32003` | Niekompatybilna wersja protokołu |
| `-32004` | Model nieznany lub suma kontrolna się nie zgadza |

Zerwanie połączenia przez klienta nie przerywa sesji — daemon nagrywa dalej,
a journal pozwala klientowi nadrobić po ponownym połączeniu. To celowe:
zamknięcie okna GTK w trakcie dyktowania nie może kasować materiału.

## Debugowanie

```bash
socat - UNIX-CONNECT:$XDG_RUNTIME_DIR/diktate.sock
{"jsonrpc":"2.0","id":1,"method":"hello","params":{"proto":"1.0","client":"socat"}}
{"jsonrpc":"2.0","id":2,"method":"session.subscribe","params":{}}
```
