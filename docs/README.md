# Diktate — dokumentacja projektowa

Lokalna transkrypcja mowy na tekst dla Linuksa: rezydentny daemon oparty
o whisper.cpp z akceleracją sprzętową, okno GTK4 do dyktowania i redakcji,
oraz agent wykonujący akcje na podstawie komend głosowych.

Status: **faza projektowa**. Nie ma jeszcze kodu.

## Spis dokumentów

| Dokument | Zawartość |
|----------|-----------|
| [01-architektura.md](01-architektura.md) | Podział na procesy, wybór technologii, przepływ danych |
| [02-protokol-ipc.md](02-protokol-ipc.md) | Protokół JSON-RPC między daemonem a klientami |
| [03-model-dokumentu.md](03-model-dokumentu.md) | Spany, operacje, zastępowanie zaznaczenia głosem |
| [04-konfiguracja.md](04-konfiguracja.md) | Pełny układ plików TOML, wszystkie parametry |
| [05-agent.md](05-agent.md) | Komendy głosowe, reguły, bezpieczeństwo |
| [06-gui.md](06-gui.md) | Okno dyktowania, tray, skróty klawiaturowe |
| [07-zachowanie-dyktowania.md](07-zachowanie-dyktowania.md) | **Serce aplikacji** — semantyka dyktowania |
| [08-dystrybucja-i-testy.md](08-dystrybucja-i-testy.md) | Budowanie, pakowanie, strategia testów |
| [09-roadmapa.md](09-roadmapa.md) | Kolejność prac, etapy, co świadomie odłożone |
| [10-wake-word.md](10-wake-word.md) | Aktywacja głosem — przegląd silników, wybrane rozwiązanie |

## Decyzje, które już zapadły

Poniższe ustalenia są wynikiem sesji projektowej z 2026-09-18 i stanowią
założenia wyjściowe całej reszty dokumentacji.

1. **Język: Rust.** Uzasadnienie w [01-architektura.md](01-architektura.md#wybór-języka).
2. **Backend akceleracji: Vulkan** jako domyślny (pokrywa Intel Arc i AMD
   jednym kodem), OpenVINO i ROCm jako opcjonalne feature flagi, CPU jako
   fallback.
3. **Podział na procesy.** Daemon jest „głupi" — transkrybuje i rozgłasza
   segmenty. Cała semantyka dokumentu i redakcji żyje po stronie klienta.
4. **Transport IPC: gniazdo uniksowe + JSON-RPC 2.0** (NDJSON).
5. **Transkrypcja hybrydowa.** VAD tnie mowę na segmenty w miejscach ciszy;
   każdy segment po transkrypcji jest gotowy i nie zmienia się sam z siebie.
6. **Dystrybucja: binarki dla Linuksa.** Bez Flatpaka — sandbox kolidowałby
   z dostępem do GPU/NPU i z `ydotool`.
7. **Konfiguracja wyłącznie w plikach TOML.** Przenoszenie wybranych opcji do
   GUI to osobna, późniejsza decyzja.
8. **Wake word przez VAD + whisper `tiny`**, nie przez dedykowany silnik.
   Jeden mechanizm obsługuje i „dyktuj", i „zdzisław, zgaś światło", bez
   nowych zależności i bez treningu modeli.
9. **Model interakcji przejęty z prototypu `gadajj`**: tekst wchodzi w miejscu
   karetki, zaznaczenie samo w sobie znaczy „zastąp to", a po zastąpieniu
   dyktowanie płynie dalej od poprawionego miejsca. Prototyp był używany na
   co dzień i to zachowanie zdało egzamin — przepisujemy je,
   dokładając rozstrzyganie po osi czasu audio i pełną odwracalność.
   Szczegóły w [07-zachowanie-dyktowania.md](07-zachowanie-dyktowania.md).

## Poza zakresem pierwszej wersji

- Wpisywanie tekstu do aktywnego okna przez `ydotool`
- Redakcja tekstu lokalnym LLM-em
- Diaryzacja (rozróżnianie mówców)
- Flatpak
