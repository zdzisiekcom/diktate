# 10 — Wake word

Aktywacja głosem: powiedzenie „dyktuj" otwiera okno dyktowania, a „zdzisław,
zgaś światło" wykonuje akcję — bez dotykania klawiatury.

## Ustalenie wyjściowe: polski odcina większość gotowych rozwiązań

Przegląd dostępnych silników (wrzesień 2026):

| Silnik | Polski | Trening | Uwagi |
|--------|--------|---------|-------|
| openWakeWord / `oww-rs` | nie | Piper PL + Colab, ~1 h | Modele wyłącznie angielskie; autor podaje jako powód brak TTS-ów dla innych języków |
| sherpa-onnx KWS | nie | — | Open-vocabulary, ale modele tylko dla chińskiego i angielskiego |
| Vosk `small-pl-0.22` | tak | brak | 50 MB, Apache 2.0, gramatyka ograniczona do kilku słów. Wymaga `libvosk.so` |
| rustpotter | agnostyczny | 3–8 własnych nagrań | MFCC + DTW, czysty Rust. Ostatnie wydanie 2023 |
| Picovoice Porcupine | zależnie od planu | — | Komercyjny, licencja wyklucza swobodną dystrybucję |

## Wybrane rozwiązanie: VAD + whisper `tiny`

Mikrofon jest otwarty, Silero VAD czuwa przy znikomym koszcie. Gdy VAD wykryje
wypowiedź, krótki fragment jest transkrybowany modelem `tiny`, a wynik
dopasowywany rozmyto do listy fraz z konfiguracji. Brak trafienia — audio jest
odrzucane i nigdzie nie zapisywane.

```
mikrofon ──► ring buffer ──► Silero VAD
                                │ wypowiedź zamknięta ciszą
                                ▼
                         whisper tiny (model czuwania)
                                │ tekst
                                ▼
                      rozmyte dopasowanie do [wakeword].phrases
                        │                          │
                   brak trafienia            trafienie
                        │                          │
                   odrzuć audio      ┌─────────────┴─────────────┐
                                     ▼                           ▼
                            fraza dyktowania              fraza agenta
                         (otwórz okno, przejdź     (cała wypowiedź trafia do
                          na model główny)          diktate-agent — reguły
                                                    dopasowują resztę zdania)
```

### Dlaczego nie dedykowany silnik wake word

Decydujący argument nie dotyczy kosztu obliczeniowego, lecz tego, że **dwa
przypadki użycia są różne**:

- „dyktuj" — wystarczy wykryć samo słowo
- „zdzisław, zgaś światło" — potrzebna jest **cała reszta zdania**

Dedykowany silnik (rustpotter, openWakeWord) wykrywa wyłącznie słowo. Drugi
przypadek i tak wymagałby następnie przepuszczenia całej wypowiedzi przez
whispera, żeby ustalić, o jaką akcję chodzi — czyli dwóch silników zamiast
jednego, z osobnym strojeniem i osobnym trybem awarii.

Podejście z `tiny` obsługuje oba przypadki jednym przebiegiem: tekst wypowiedzi
już istnieje, wystarczy dopasować go do reguł ([05-agent.md](05-agent.md)).

Drugi argument: **zero nowych zależności**. Silero VAD, whisper-rs, ring buffer
i rozmyte dopasowywanie są budowane w etapach 1–3 niezależnie od wake worda.
Ta funkcja to ich złożenie, nie nowy podsystem.

Trzeci: frazy aktywujące są **zwykłym tekstem w TOML-u**. Dowolna liczba,
dowolna fraza, zmiana bez treningu i bez restartu. Żaden inny wariant tego nie
daje — rustpotter wymaga nagrania nowych próbek, openWakeWord ponownego
treningu.

### Znane ograniczenie

W pomieszczeniu z ciągłą mową w tle (telewizor, open space) `tiny` uruchamia
się przy każdej cudzej wypowiedzi. Przy pracy w pojedynkę jest to bez
znaczenia; przy stałym hałasie — uciążliwe.

Gdyby praktyka to pokazała, znanym rozwiązaniem jest **rustpotter** jako tani,
zależny od mówcy filtr wstępny przed `tiny`. Nic pod to nie budujemy z góry —
nie ma wymiennego silnika ani abstrakcji przygotowanej „na wszelki wypadek".
Ten akapit istnieje po to, żeby przy takiej potrzebie nie zaczynać od zera
z przeglądem opcji.

## Dopasowywanie fraz

Whisper `tiny` po polsku bywa niedokładny — „dyktuj" potrafi wyjść jako
„dyktui" albo „dyktu". Dopasowanie jest więc **rozmyte** (ta sama
implementacja, co w [05-agent.md](05-agent.md)), na tekście znormalizowanym:
małe litery, bez interpunkcji, opcjonalnie bez znaków diakrytycznych.

Próg dla wake worda jest **wyższy** niż dla reguł agenta. Fałszywe trafienie
otwiera okno albo wykonuje akcję bez udziału użytkownika, więc kosztuje więcej
niż przeoczenie, które użytkownik po prostu powtarza.

### Potwierdzenie dużym modelem

Przy trafieniu frazy agenta audio wciąż znajduje się w ring bufferze. Jeśli
pewność `tiny` jest niska (`avg_logprob` poniżej progu), ten sam fragment jest
transkrybowany ponownie modelem głównym, zanim reguła zostanie wykonana.
Kosztuje to jeden dodatkowy przebieg wyłącznie w przypadkach wątpliwych,
a chroni przed uruchomieniem nie tej akcji, co trzeba.

## Konfiguracja

```toml
[wakeword]
enabled = false               # domyślnie WYŁĄCZONY — stały nasłuch to świadoma decyzja
model = "tiny"                # model czuwania, osobny od [whisper].model
language = "pl"
match_threshold = 0.85        # wyższy niż u agenta
normalize_diacritics = true
confirm_with_main_model = true
confirm_below_logprob = -0.7

# Frazy otwierające dyktowanie
dictation_phrases = ["dyktuj", "notuj", "zapisz notatkę"]

# Frazy przekazywane do diktate-agent wraz z całą wypowiedzią
agent_keywords = ["zdzisław"]

[wakeword.limits]
max_utterance_ms = 6000       # dłuższe wypowiedzi nie są nawet transkrybowane
cooldown_ms = 1500            # blokada po trafieniu, chroni przed podwójnym wyzwoleniem
```

`max_utterance_ms` jest istotny dla kosztu: w trybie czuwania nie ma sensu
transkrybować dziesięciosekundowej cudzej wypowiedzi, skoro żadna fraza
aktywująca nie jest tak długa. Wypowiedzi przekraczające limit są odrzucane
przez VAD, zanim dotrą do modelu.

## Dwa modele naraz

W trybie czuwania w pamięci jest `tiny` (~75 MB). Po aktywacji dyktowania
daemon przełącza się na model główny. Dwie strategie, do rozstrzygnięcia przy
implementacji na podstawie pomiaru:

- **Trzymać oba w pamięci** — natychmiastowe przełączenie, koszt kilkuset MB
  RAM zajętych na stałe
- **Ładować główny na żądanie** — mniej pamięci w czuwaniu, ale kilkanaście
  sekund opóźnienia przy pierwszym dyktowaniu, co psuje cały sens aktywacji
  głosem

Wstępnie: oba w pamięci, z opcją `unload_main_model_after_idle_secs` dla osób,
którym zależy na RAM-ie.

## Prywatność

Stały nasłuch mikrofonu jest funkcją o realnych konsekwencjach, nie detalem
technicznym. Dlatego:

- **Domyślnie wyłączony.** `wakeword.enabled = false` w świeżej instalacji.
- **Nic nie jest zapisywane w trybie czuwania.** Audio żyje wyłącznie w ring
  bufferze i jest nadpisywane. Journal ([01](01-architektura.md#journal)) nie
  jest prowadzony, dopóki nie padnie fraza aktywująca.
- **Widoczny wskaźnik.** Ikona w tray-u jednoznacznie odróżnia czuwanie od
  bezczynności. Użytkownik nigdy nie ma wątpliwości, czy mikrofon jest otwarty.
- **Wyłączenie jednym kliknięciem** z menu tray-a, bez wchodzenia w ustawienia.
- **Nic nie opuszcza komputera** — jak w całej aplikacji.

## Kolejność wdrożenia

Etap 3, razem z `diktate-agent` ([09-roadmapa.md](09-roadmapa.md)). Oba
komponenty dzielą mechanizm rozmytego dopasowywania fraz i oba konsumują ten
sam strumień, więc budowanie ich osobno oznaczałoby napisanie tej samej logiki
dwa razy.
