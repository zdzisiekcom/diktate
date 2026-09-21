# 07 — Zachowanie przy dyktowaniu (serce aplikacji)

> **Status: rdzeń rozstrzygnięty, warstwa redakcyjna otwarta.** Semantyka
> wstawiania, zastępowania i kontynuacji dyktowania jest ustalona — pochodzi
> z prototypu `gadajj`, który był używany na co dzień i którego zachowanie
> uznajemy za punkt odniesienia. Otwarte pozostają decyzje wyższego poziomu
> (akapity, interpunkcja jawna, autopoprawki, LLM) — sekcja
> [Sprawy otwarte](#sprawy-otwarte).

## Dlaczego to osobny dokument

Surowe wyjście whispera nie jest tekstem, który chce się mieć w notatce.
Model transkrybuje *wypowiedź*, a użytkownik chce *dokument*. Różnica obejmuje
punkt wstawienia, sklejanie z tym, co już jest, poprawki, interpunkcję,
formatowanie i polecenia skierowane do narzędzia zamiast do tekstu.

Każda z tych rzeczy to decyzja projektowa z konsekwencjami dla użytkownika,
a razem stanowią one właściwą wartość aplikacji. Wrzucenie ich do dokumentu
o architekturze rozmyłoby jedno i drugie.

## Warstwa, której dotyczy

```
whisper → surowy tekst segmentu
           │
           ▼
   ┌───────────────────────┐
   │  TU MIESZKA TEN       │   ← punkt wstawienia, sklejanie, zastępowanie,
   │  DOKUMENT             │      normalizacja, polecenia w mowie
   └───────────────────────┘
           │
           ▼
      operacje na dokumencie (03-model-dokumentu.md)
```

Przetwarzanie jest **po stronie klienta**, nie daemona — daemon pozostaje
usługą transkrypcji ([01-architektura.md](01-architektura.md#podział-na-procesy)).
Implementacyjnie jest to potok transformacji w `diktate-document`, działający
między odebraniem `segment.added` a wyprodukowaniem operacji.

## Dziedzictwo prototypu

Pierwszy prototyp (`gadajj`, Python + Textual + whisper) zweryfikował
w praktyce model interakcji, który tutaj utrwalamy. Istotne jest, że działał
on na **jednym polu tekstowym z kursorem i zaznaczeniem**, a nie na strumieniu
dopisywanym na koniec. Z tego wynikają cztery zachowania przeniesione tu
bez zmian co do odczuwalnego efektu:

| Zachowanie prototypu | Dlaczego zostaje |
|----------------------|------------------|
| Tekst wchodzi **w miejscu kursora**, nie na końcu | Pozwala wrócić w środek notatki i dyktować dalej właśnie tam |
| **Zaznaczenie = polecenie zastąpienia**, bez żadnego trybu | Poprawka kosztuje jeden ruch myszą/klawiszem, nie dwa; rytm mowy się nie łamie |
| Po wstawieniu **kursor stoi za tekstem, fokus wraca do pola** | Kolejna wypowiedź płynnie kontynuuje poprzednią |
| **Adaptacyjny próg ciszy** — długa wypowiedź jest cięta wcześniej | Pauza na myślenie nie kończy zdania, a długi monolog nie każe czekać na tekst |

Nowe względem prototypu jest to, co prototyp robił źle albo wcale:
rozstrzyganie po osi czasu audio (prototyp zastępował zaznaczenie pierwszym
segmentem, jaki przyszedł — także takim, który padł *przed* zaznaczeniem),
odwracalność każdej automatycznej zmiany oraz kontekst promptu brany
z dokumentu, a nie z całego pola tekstowego bez względu na kursor.

## Punkt wstawiania

Klient utrzymuje **kotwicę wstawiania** (`insert_anchor`, typ `Anchor`
z [03](03-model-dokumentu.md#kotwice--dlaczego-to-jest-spójne)). To ona,
a nie koniec dokumentu, jest miejscem, w którym ląduje nowy segment.

Reguły aktualizacji:

1. Start sesji — kotwica na końcu dokumentu (dla pustej notatki: początek).
2. Po wstawieniu segmentu — tuż za wstawionym tekstem.
3. Po przesunięciu karetki przez użytkownika (klik, strzałki) bez zaznaczenia
   — w nowe miejsce karetki.
4. Po edycji klawiaturą — za wprowadzoną zmianą, zgodnie z karetką.
5. Przy niepustym zaznaczeniu — **nie zmienia się**; kotwica pamięta miejsce
   sprzed zaznaczenia i służy do wstawiania zaległych segmentów
   ([niżej](#zaznaczenie-i-zastępowanie)).

Wstawienie realizuje operacja `Insert { at, span }`. Dopisanie na koniec to jej
przypadek szczególny, nie osobna ścieżka.

**Autoprzewijanie**: okno przewija się do punktu wstawiania tylko wtedy, gdy
ten punkt był widoczny przed wstawieniem. Użytkownik, który przewinął w górę,
żeby coś przeczytać, nie zostaje z tego wyrwany.

## Sklejanie z otoczeniem

Whisper zwraca wypowiedź jako samodzielne zdanie, bez wiedzy o tym, co stoi
obok. Przed wstawieniem tekst segmentu przechodzi przez normalizację:

| Krok | Reguła | Parametr |
|------|--------|----------|
| Obcięcie białych znaków | `trim()` na wejściu | — |
| Spacja rozdzielająca | Dodaj spację przed wstawianym tekstem, jeśli znak przed punktem wstawiania nie jest białym znakiem, otwierającym nawiasem ani cudzysłowem otwierającym | `auto_space` |
| Brak spacji przed interpunkcją | Nie dodawaj spacji, jeśli wstawiany tekst zaczyna się od `, . ! ? : ; ) ” »` | `auto_space` |
| Spacja za wstawianym tekstem | Dodaj, jeśli znak za punktem wstawiania jest znakiem słowa (wstawianie w środku zdania) | `auto_space` |
| Usunięcie wielokropka | Jeśli punkt wstawiania jest na końcu dokumentu, a dokument kończy się `...` lub `…`, usuń go przed wstawieniem | `strip_trailing_ellipsis` |

Ostatnia reguła pochodzi wprost z prototypu i jest mniej ezoteryczna, niż
wygląda: whisper kończy wielokropkiem wypowiedź, która została ucięta w pół
zdania — czyli dokładnie każdą wypowiedź przerwaną pauzą na oddech. Bez tego
notatka jest usiana wielokropkami w miejscach, gdzie użytkownik po prostu
zaczerpnął powietrza.

Normalizacja jest **częścią operacji wstawienia**, a nie osobnym krokiem po
niej: `Ctrl+Z` cofa wstawienie razem z dodaną spacją i usuniętym wielokropkiem,
jednym ruchem. Usunięcie wielokropka, które trafiałoby do logu jako osobna
operacja, wymagałoby dwóch cofnięć i łamałoby zasadę nadrzędną.

Wielkość liter **nie jest** korygowana automatycznie w pierwszej wersji —
patrz [Sprawy otwarte](#sprawy-otwarte).

## Zaznaczenie i zastępowanie

### Zasada

**Niepuste zaznaczenie w oknie znaczy: „następną wypowiedzią zastąp to".**
Nie trzeba niczego uzbrajać. Użytkownik zaznacza fragment, który mu nie pasuje,
mówi go od nowa i dyktuje dalej.

Okno sygnalizuje uzbrojenie: zaznaczenie zmienia kolor na
`gtk.replace.highlight_color`, a na pasku pojawia się „zastępuję…".
`Escape` znosi zaznaczenie i wraca do zwykłego wstawiania.

`Ctrl+R` pozostaje jako jawne uzbrojenie — potrzebne, gdy
`gtk.dictation.implicit_replace = false` albo gdy tryb to `until_confirm`
i zastąpienie ma objąć więcej niż jedną wypowiedź.

### Rozstrzyganie po osi czasu audio

Moment powstania zaznaczenia zapisujemy jako `replace_armed_at` — pozycję
w osi czasu audio sesji (nie czas ścienny). Dla każdego przychodzącego
segmentu:

| Warunek | Działanie |
|---------|-----------|
| `audio_start_ms >= replace_armed_at` | `Splice` w kotwice zaznaczenia |
| `audio_start_ms < replace_armed_at` | `Insert` w `insert_anchor`; zaznaczenie **pozostaje uzbrojone** |

Drugi wiersz to poprawka względem prototypu i jedyny powód, dla którego ten
mechanizm w ogóle wymaga projektowania. Przy mowie szybszej niż przepustowość
GPU segment wypowiedziany dziesięć sekund temu potrafi dotrzeć po zaznaczeniu.
Prototyp zjadał wtedy zaznaczenie zdaniem, które z poprawką nie miało nic
wspólnego, a użytkownik tracił jedno i drugie. Granicę wyznacza timestamp
audio, nie kolejność przybycia.

Zaległy segment nie unieważnia zaznaczenia, bo `Insert` nie dotyka cudzych
kotwic — gwarancja z [03](03-model-dokumentu.md#kotwice--dlaczego-to-jest-spójne).

### Zakończenie zastąpienia

Jedno zastąpienie zjada **dokładnie jedną wypowiedź** — jeden segment zamknięty
ciszą (`gtk.replace.mode = "one_utterance"`). Po wykonaniu `Splice` zaznaczenie
znika, kotwica wstawiania ląduje za nowym tekstem, tryb wraca do wstawiania —
i dyktowanie płynie dalej od poprawionego miejsca. To jest „kontynuowanie
dyktowania" z prototypu.

Tryb `until_confirm` (zastępowanie trwa, aż użytkownik potwierdzi) zostaje jako
furtka konfiguracyjna; wymaga sięgnięcia po klawiaturę w trakcie mówienia,
więc nie jest domyślny.

### Granice zaznaczenia

- Zaznaczenie obejmujące **część słowa** jest przed zastąpieniem rozszerzane do
  granic słowa (`snap_to_word_boundary`, domyślnie włączone). Zaznaczenie
  precyzyjne co do znaku jest przy poprawce głosowej niemal zawsze
  przypadkiem, nie intencją.
- Białe znaki na brzegach zaznaczenia zostają **poza** zastąpieniem — nowy
  tekst wchodzi między te same spacje, co stary.
- Interpunkcja bezpośrednio za zaznaczeniem (`.`, `,`, `?`) **nie jest**
  zastępowana, jeśli zaznaczenie jej nie obejmuje, a nowy tekst nie kończy się
  własną interpunkcją. Inaczej poprawka jednego słowa gubiłaby kropkę zdania.

## Rytm: adaptacyjny próg ciszy

Segment powstaje, gdy VAD wykryje ciszę dłuższą niż próg. Jeden próg nie
obsługuje obu sytuacji naraz:

- **krótki próg** (ok. 0,3 s) tnie zdanie na pauzie przed trudnym słowem;
- **długi próg** (ok. 1,5 s) każe czekać półtorej sekundy na tekst po każdej
  wypowiedzi i nie kończy monologu, dopóki mówiący nie zrobi wyraźnej pauzy.

Prototyp rozwiązał to progiem zależnym od długości trwającej wypowiedzi
i rozwiązanie zostaje:

```
czas trwania wypowiedzi <= long_segment_ms  →  cisza kończąca = silence_ms_to_end
czas trwania wypowiedzi >  long_segment_ms  →  cisza kończąca = silence_ms_to_end_long
```

Na początku wypowiedzi użytkownik ma czas na zastanowienie. Gdy mówi już
dłuższą chwilę, każda pauza kończy segment niemal natychmiast — tekst zaczyna
płynąć na ekran w trakcie mówienia, zamiast spaść blokiem na końcu.
`max_segment_ms` pozostaje twardym cięciem (okno whispera to 30 s).

To jest parametr daemona, nie klienta ([04](04-konfiguracja.md#daemontoml),
sekcja `[vad]`) — VAD żyje w daemonie i tnie strumień, zanim ktokolwiek
zobaczy tekst.

## Kontekst dla whispera

Whisper transkrybuje spójniej, gdy dostanie ogon wcześniejszego tekstu jako
`prompt` — trzyma terminologię, pisownię nazw własnych i styl interpunkcji.
Prototyp podawał ostatnie ~1200 znaków z pola tekstowego i był to jeden
z wyraźniej odczuwalnych efektów.

W diktate kontekstem jest **tekst dokumentu przed punktem wstawiania**
(a przy uzbrojonym zastąpieniu — tekst przed **początkiem zaznaczenia**, bo
tam trafi nowy tekst), nie surowy log transkrypcji. Różnica ma znaczenie:
jeśli użytkownik poprawił błędnie rozpoznane nazwisko, model ma zobaczyć
wersję poprawioną, a nie własną pomyłkę sprzed minuty. Tę samą drogę
przechodzą więc segmenty od daemona i ręczne poprawki z klawiatury.

Daemon nie zna dokumentu, więc kontekst dostarcza klient metodą
`session.context` ([02](02-protokol-ipc.md#kontekst-transkrypcji)) — prompt,
który dostaje whisper, to szablon z `gtk.toml` (`[whisper] prompt`) ze
sklejonym w miejscu `{kontekst}` ogonem dokumentu, obciętym do
`context_chars`. Wysyłka jest podpięta pod sygnał `changed` bufora tekstowego,
a ten odpala się przy zmianie treści, nie przy ruchu kursora — sam ruch
karetki więc nic nie planuje. Idzie z odbiciem 400 ms (inaczej pisanie
z klawiatury zmieniałoby prompt przy każdym znaku) i jest pomijana, gdy
sklejony prompt nie różni się od ostatnio wysłanego. Pusty szablon znaczy
„nie wysyłaj kontekstu w ogóle": daemon zostaje wtedy przy
`whisper.initial_prompt` z `daemon.toml`, czyli zachowaniu sprzed tej funkcji,
i to jest wartość domyślna.

Jest to podpowiedź, nie zobowiązanie — daemon transkrybuje normalnie także
wtedy, gdy żaden klient kontekstu nie przysłał.

Zabezpieczenie przed zapętleniem (`repetition_guard`) zostaje: prompt potrafi
skłonić model do powtórzenia własnego kontekstu zamiast transkrypcji, więc
segment zbyt podobny do promptu jest liczony powtórnie bez kontekstu.

## Filtry: co nie trafia do dokumentu

Do reguł z `[filters]` ([04](04-konfiguracja.md#daemontoml)) dochodzi jedno
doświadczenie prototypu: przy `language = "auto"` whisper na szumie
i na pojedynczych sylabach potrafi „wykryć" język, którego nigdy nie było,
i wyprodukować zdanie po niemiecku albo po walijsku.

Stąd `allowed_languages`: przy `language = "auto"` segment z wykrytym językiem
spoza listy jest odrzucany (`segment.dropped`, `reason = "language"`),
a klient pokazuje to w oknie zamiast wstawiać do notatki. Dla `language = "pl"`
reguła nie działa — język jest wtedy narzucony.

Odrzucenie segmentu jest **widoczne**, nie ciche: użytkownik widzi, że coś
padło i nie weszło do tekstu. Cichy filtr wygląda identycznie jak zgubiony
segment i niszczy zaufanie do narzędzia.

## Sprawy otwarte

Poniższe wymagają osobnej decyzji, po okresie używania etapu 1 i 2. Żadne
z nich nie blokuje implementacji rdzenia opisanego wyżej.

### 1. Łączenie segmentów w akapity

Segment to jednostka techniczna wyznaczona przez ciszę, nie redakcyjna.
Trzysekundowa przerwa na zastanowienie nie powinna tworzyć akapitu, a wyraźna
zmiana tematu powinna. Do rozstrzygnięcia: próg czasowy ciszy, jawna komenda,
analiza treści, czy kombinacja. W pierwszej wersji akapit powstaje wyłącznie
z `Enter` z klawiatury.

### 2. Interpunkcja dyktowana jawnie

Whisper stawia interpunkcję sam i robi to nierówno, zwłaszcza po polsku.
Alternatywa: tryb, w którym „kropka", „przecinek", „nowy akapit" są
poleceniami. Wymaga listy słów per język i rozstrzygnięcia konfliktu: co,
jeśli użytkownik chce napisać słowo „kropka". Domyślnie ufamy modelowi.

### 3. Polecenia wbudowane w mowę

Odrębne od komend agenta ([05-agent.md](05-agent.md)) — tamte uruchamiają
programy, te modyfikują tekst. Kandydaci: „nowy akapit", „duża litera",
„cofnij ostatnie zdanie", „punktor". Wymaga tego samego rozmytego dopasowania
co agent, ale próg musi być wyższy: fałszywe trafienie zjada fragment notatki.

### 4. Wielkość liter na łączeniach

Segment wstawiony w środek zdania zaczyna się wielką literą, bo whisper widzi
go jako samodzielną wypowiedź. Automatyczna korekta (`auto_case`) potrafi
zepsuć nazwę własną, więc w pierwszej wersji jej nie ma — użytkownik poprawia
klawiaturą albo zostawia.

### 5. Autopoprawki wypowiedziane na głos

„Znaczy", „to jest", „nie, raczej". Człowiek to filtruje, whisper
transkrybuje dosłownie. Argument za oddaniem tego etapowi redakcji LLM-em:
heurystyka będzie się myliła, a LLM widzi kontekst.

### 6. Słownik i wymuszanie pisowni

Dwa poziomy: `initial_prompt` (miękko) i podstawienia po transkrypcji
(twardo, ale potrafią zepsuć poprawny tekst). Do rozstrzygnięcia: dosłowne
czy rozmyte, wspólne czy per profil.

### 7. Sygnalizacja niepewności

`avg_logprob` jest zaszumiony, a próg zależy od modelu. Wyróżnianie kolorem
jest w [06](06-gui.md); propozycje alternatyw — nie.

### 8. Punkt wejścia dla LLM-a

Redakcja wpina się przez operacje `Splice`, tak samo odwracalne jak każde
inne. Do rozstrzygnięcia: na żądanie czy w tle, na akapicie czy na całości,
z podglądem diffu czy bez.

## Zasada nadrzędna

**Użytkownik zawsze widzi, co narzędzie zrobiło, i zawsze może to cofnąć
jednym skrótem.** Każda automatyczna transformacja — dodana spacja, usunięty
wielokropek, zastąpiony fragment — jest częścią operacji w logu, nie
niewidzialnym przetworzeniem. To wynika z modelu dokumentu i nie podlega
negocjacji przy projektowaniu kolejnych zachowań: heurystyka, której nie da
się cofnąć, jest gorsza od braku heurystyki.
