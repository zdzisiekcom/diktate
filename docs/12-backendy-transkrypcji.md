# Backendy transkrypcji

Daemon ma dwa silniki zamiany dźwięku na tekst i wybiera między nimi jednym
polem konfiguracji. Segmentacja, VAD, kolejka i filtry halucynacji są dla
obu **te same** — podmienia się wyłącznie ostatni krok potoku.

| backend | model | WER (polski) | koszt segmentu 5 s |
|---------|-------|--------------|--------------------|
| `whisper` | OpenAI Whisper `small-q5_1`, Vulkan | 5,1% | ~1,44 s |
| `parakeet` | NVIDIA Parakeet TDT 0.6B v3 `q8_0`, CPU | 14,8% | ~1,06 s |

Parakeet myli treść, a nie interpunkcję: `zgubiłem siostrę` → `wykupiłem
siostry`. Jego rolą miał być **podgląd na żywo**, a nie tekst końcowy.

**Ta tabela wygląda dziś inaczej niż tydzień temu i to jest jej najważniejsza
treść.** Dopóki whisper chodził na whisper.cpp 1.7.4, segment 5 s kosztował go
3,0 s wobec 0,9 s parakeeta — przewaga 3,3×, za którą warto było płacić
trzykrotnie gorszym WER-em. Po przejściu na 1.9.4 whisper robi ten sam segment
w 1,44 s, a parakeet liczony w procesie (a nie przez serwer, który raportował
tylko własny czas liczenia) potrzebuje 1,06 s. Przewaga stopniała do **1,36×**.

Za 36% szybszy tekst płaci się tu trzykrotnie wyższym WER-em i to już nie jest
oczywisty interes. Parakeet zostaje jako backend do porównań i jako materiał
pod docelowy pomysł z sekcji „Co dalej", ale **whisper pozostaje wyborem
domyślnym** i po tej zmianie ma po temu mocniejszy powód niż wcześniej.

## To nie są dwa programy, tylko dwa modele

Oba silniki liczą **w procesie daemona** i oba pochodzą z tego samego
źródła: submodułu `vendor/whisper.cpp` przypiętego do wydania v1.9.4.
whisper.cpp jest repozytorium i silnikiem — w środku siedzi `ggml` z
backendami (CPU, Vulkan) — a na tym fundamencie zbudowane są dwa modele
o różnej architekturze:

| | Whisper | Parakeet TDT |
|---|---|---|
| autor modelu | OpenAI | NVIDIA |
| architektura | enkoder-dekoder, stałe okno 30 s | transducer (TDT), bez stałego okna |
| biblioteka | `libwhisper` | `libparakeet` |
| API | `whisper.h` | `parakeet.h` |
| plik modelu | `ggml-small-q5_1.bin` | `ggml-parakeet-tdt-0.6b-v3-q8_0.bin` |

Dzielą `ggml` i backendy, ale to osobne biblioteki z osobnym API i
niewymiennymi plikami modeli. Jedna kopia ggml w procesie jest tu warunkiem,
a nie wygodą: dwie znaczyłyby kolizję symboli.

Ta różnica architektur tłumaczy wszystkie liczby w tym dokumencie. Whisper
zawsze przepuszcza okno 30 sekund, więc dwusekundowy wycinek kosztuje prawie
tyle, co pięciosekundowy — stąd reguła z `CLAUDE.md`, że krótkie segmenty się
nie opłacają. Parakeet stałego okna nie ma, więc koszt skaluje się z długością
materiału. Cała jego przewaga w dyktowaniu na żywo bierze się z tej jednej
właściwości, i cała jego wada też jest architektoniczna: sam wykrywa język,
narzucić mu go nie można, i myli treść.

## Wybór backendu

```toml
[transcription]
backend = "whisper"           # albo "parakeet"

[parakeet]
model = ""                    # puste = $DIKTATE_PARAKEET_MODEL, potem
                              # ~/.diktate/models/ggml-parakeet-tdt-0.6b-v3-q8_0.bin
threads = 0                   # 0 = tyle, ile widzi system
```

Nazwa spoza listy zatrzymuje start daemona — cichy powrót do whispera
znaczyłby, że literówka wygląda jak działająca konfiguracja.

Sekcja `[parakeet]` nie ma nic do sprawdzenia przy walidacji: puste `model`
znaczy „ścieżka domyślna", zerowe `threads` — „wybierz sam". Istnienie pliku
modelu rozstrzyga się przy starcie daemona, tak samo jak przy whisperze, bo
to stan dysku, a nie poprawność konfiguracji.

## Co daemon robi przy starcie

Oba backendy wczytują model i oba potrafią wtedy odmówić. To celowe — daemon,
który wystartował, ma transkrybować, a nie odkrywać przy pierwszym zdaniu, że
nie ma czym.

Model parakeeta nie jest pakowany ani pobierany przez `diktate model pull`:
katalog modeli (`crates/diktate-cli/src/model/catalog.rs`) wozi wyłącznie
warianty whispera z HuggingFace, a plik ggml parakeeta powstaje lokalnie
z archiwum `.nemo` NVIDII skryptem `models/convert-parakeet-to-ggml.py`
z whisper.cpp. Dopóki nie ma go pod publicznym adresem, kreator nie ma skąd
go wziąć i backend wymaga ręcznego położenia pliku w `~/.diktate/models`.

## Minimum 3 sekundy, czyli skąd biorą się dwa mechanizmy

Parakeet myli język przy krótkim materiale: dwie sekundy polskiego wracają
jako czeski, sekunda jako pustka. Nie jest to arbitralny limit i nie da się
go obejść parametrem — `parakeet_full_params` nie ma pola języka. Wartość
minimalna nie jest wpisana w konfigurację, tylko pochodzi z warstwy wiązań
(`dk_parakeet_min_audio_s`), żeby dokument nie rozjechał się z kodem.

Diktate radzi sobie z tym na dwa sposoby, bo pilnują czego innego:

1. **Progi segmentacji** dbają o przypadek normalny. Przy `long_segment_ms =
   3000` rozpędzona wypowiedź domyka się dopiero po trzech sekundach, więc
   segment z pre-rollem i ogonem ciszy ma ~3,5 s — powyżej progu, bez
   sztuczek.
2. **Dopełnianie ciszą** dba o wyjątek. Jedno słowo rzucone w ciszę wciąż
   staje się segmentem (`min_speech_ms` zostaje przy 300 ms) i wciąż idzie do
   modelu — dobite zerami do minimum. Wynik bywa wtedy bzdurny albo w obcym
   języku, ale log mówi wprost, który segment był dopełniony, więc przy ocenie
   jakości widać, co jest czym.

Porzucanie krótkich segmentów byłoby trzecią opcją i zostało odrzucone:
zjedzone „tak" jest gorsze od „tak" przetłumaczonego na czeski, bo tego
pierwszego nie widać.

Daemon ostrzega przy starcie, gdy `pre_roll_ms + long_segment_ms +
silence_ms_to_end_long` wychodzi poniżej minimum — wtedy większość segmentów
pójdzie dopełniona. To ostrzeżenie, nie odmowa startu: dopełnianie działa,
po prostu jakość spada.

### Zalecany profil `[vad]` dla Parakeeta

```toml
[vad]
long_segment_ms = 3000        # zamiast 2500 — segment ma sam z siebie mieć 3 s
min_speech_ms = 300           # bez zmian: jedno słowo nadal ma trafić do modelu
silence_ms_to_end_long = 200  # bez zmian
```

Podniesienie `long_segment_ms` o pół sekundy jest dla tego backendu darmowe:
parakeet liczy chunk 5 s w ~1,06 s, czyli ~4,7× szybciej niż mowa, więc
kolejka nie ma szans urosnąć. Opóźnienie rośnie dokładnie o te pół sekundy,
którą segment dłużej się zbiera.

Warto przy tym pamiętać, że whisper na 1.9.4 też ma dziś spory zapas (~35%
przepustowości przy progu 2500 ms wobec ~85% na 1.7.4), więc ten profil `[vad]`
przestał być czymś, co odróżnia backendy — patrz `CLAUDE.md`.

## Vulkan: pomaga whisperowi, szkodzi parakeetowi

Cecha Cargo `vulkan` (domyślna) buduje backend Vulkan w ggml. Whisper z niego
korzysta, parakeet **nie** — i nie jest to przeoczenie, tylko wynik pomiaru:
enkoder parakeeta na iGPU Meteor Lake liczy 4888 ms tam, gdzie na rdzeniach
1981 ms (`testdata/jfk.wav`, 11 s). Dlatego `dk_parakeet_open` dostaje
`use_gpu = false` niezależnie od tego, jak zbudowano ggml.

Whisper ma odwrotnie i to od dawna: Vulkan skraca transkrypcję `jfk.wav`
z ~18 s do ~3 s. Oba wnioski są związane z tym konkretnym GPU. Na mocniejszej
karcie rachunek dla parakeeta może wyjść inaczej — wtedy trzeba go powtórzyć,
a nie przyjąć te liczby na wiarę.

### Liczba wątków

Zmierzone na Core Ultra 7 165U, chunk 5 s, model `q8_0`, pełny czas ścienny
w procesie (`--example parakeet-wav`, średnia z dwóch kawałków):

| wątki | 4 | 8 | 10 | 12 | 14 |
|---|---|---|---|---|---|
| czas | 1640 ms | 1065 ms | 1115 ms | 1100 ms | 1105 ms |

Powyżej ośmiu wątków jest **płasko** — różnice mieszczą się w szumie, więc
SMT nie szkodzi tu tak, jak przy whisperze. Dlatego `threads = 0`, czyli
„tyle, ile widzi system", jest dobrym ustawieniem domyślnym i nie ma czego
stroić; sens ma tylko zejście w dół, gdy ktoś chce zostawić rdzenie czemuś
innemu.

Wcześniejsza wersja tego dokumentu podawała tu 855 ms dla dziesięciu wątków.
Ta liczba pochodziła z serwera HTTP i była czasem **samego liczenia**, bez
przygotowania stanu i kopii próbek. Po wbudowaniu modelu w daemon mierzymy
pełny czas od wywołania do tekstu, i to on decyduje o opóźnieniu, jakie widzi
piszący.

## Sprawdzanie bez mikrofonu

```bash
cargo run -p diktate-core --example parakeet-wav -- plik.wav 5
```

Tnie plik na kawałki po 5 s i transkrybuje każdy osobno, wypisując czas
i tekst. Cięcie jest tu istotne: daemon nie transkrybuje całych nagrań, tylko
segmenty, więc dopiero pomiar kawałek po kawałku pokazuje realne opóźnienie
i jakość przy krótkim audio. Model wskazuje `$DIKTATE_PARAKEET_MODEL`, liczbę
wątków `$DIKTATE_PARAKEET_THREADS`.

## Co dalej

Docelowy pomysł — doklejać szybki tekst z Parakeeta natychmiast, a potem
podmieniać go na wersję z whispera — **nie jest częścią tej pracy**. Wymaga
dwóch transkrypcji tego samego segmentu naraz i operacji podmiany
w dokumencie ([03-model-dokumentu.md](03-model-dokumentu.md)), które muszą
przeżyć edycję tekstu przez użytkownika w międzyczasie. Dziś backendy
wykluczają się wzajemnie i służą porównaniu.

Teraz, gdy oba modele siedzą w jednym procesie i dzielą ggml, ten pomysł jest
wyraźnie bliżej: nie trzeba już nikogo pytać po sieci, wystarczy drugi
transkryber obok pierwszego.
