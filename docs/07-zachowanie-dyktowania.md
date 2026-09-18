# 07 — Zachowanie przy dyktowaniu (serce aplikacji)

> **Status: szkic z otwartymi pytaniami.** Ten dokument opisuje semantykę
> dyktowania — to, co odróżnia użyteczne narzędzie od „whispera z okienkiem".
> Pozostałe dokumenty opisują architekturę, która ma tę semantykę udźwignąć;
> ten opisuje samą semantykę. Wymaga osobnej sesji projektowej przed
> implementacją etapu, który go dotyczy.

## Dlaczego to osobny dokument

Surowe wyjście whispera nie jest tekstem, który chce się mieć w notatce.
Model transkrybuje *wypowiedź*, a użytkownik chce *dokument*. Różnica obejmuje
interpunkcję, akapity, wtrącenia, autopoprawki wypowiedziane na głos,
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
   │  TU MIESZKA TEN       │   ← normalizacja, interpunkcja, polecenia
   │  DOKUMENT             │      wbudowane w mowę, autopoprawki
   └───────────────────────┘
           │
           ▼
      operacje na dokumencie (03-model-dokumentu.md)
```

Przetwarzanie jest **po stronie klienta**, nie daemona — daemon pozostaje
usługą transkrypcji ([01-architektura.md](01-architektura.md#podział-na-procesy)).
Implementacyjnie jest to potok transformacji w `diktate-document`, działający
między odebraniem `segment.added` a wyprodukowaniem operacji.

## Zakres — obszary do rozstrzygnięcia

### 1. Łączenie segmentów w akapity

Segment to jednostka techniczna wyznaczona przez ciszę, nie jednostka
redakcyjna. Trzysekundowa przerwa na zastanowienie nie powinna tworzyć nowego
akapitu, a wyraźna zmiana tematu powinna.

Do rozstrzygnięcia: czy akapity wyznacza próg czasowy ciszy (np. > 2 s), jawna
komenda, analiza treści, czy kombinacja. Czy segment kończący się bez kropki
skleja się z następnym.

### 2. Interpunkcja

Whisper stawia interpunkcję sam i robi to nierówno, zwłaszcza po polsku.
Alternatywy: zaufać modelowi, wymuszać przez `initial_prompt`, dyktować
interpunkcję jawnie („kropka", „przecinek", „nowy akapit"), albo pozwolić na
oba tryby przełączane w konfiguracji.

Wariant jawny wymaga listy słów-poleceń per język i rozstrzygnięcia konfliktu:
co, jeśli użytkownik chce napisać słowo „kropka".

### 3. Polecenia wbudowane w mowę

Odrębne od komend agenta ([05-agent.md](05-agent.md)) — tamte uruchamiają
programy, te modyfikują dyktowany tekst. Kandydaci: „nowy akapit", „duża
litera", „cofnij ostatnie zdanie", „popraw to na …", „punktor", „cudzysłów".

Wymaga tego samego rozmytego dopasowania co agent, ale progi są inne: fałszywe
trafienie zjada fragment notatki, więc próg musi być wyższy, a mechanizm
cofnięcia natychmiastowy.

### 4. Autopoprawki wypowiedziane na głos

Naturalna mowa zawiera „znaczy", „to jest", „nie, raczej". Człowiek to
filtruje, whisper transkrybuje dosłownie.

Do rozstrzygnięcia, czy narzędzie w ogóle ma to rozumieć, czy zostawić to
etapowi redakcji LLM-em. Argument za drugim wariantem: heurystyka będzie się
myliła, a LLM widzi kontekst.

### 5. Zastępowanie przez ponowne podyktowanie

Mechanika jest ustalona w [03-model-dokumentu.md](03-model-dokumentu.md).
Otwarte pozostaje zachowanie na krawędziach: czy zastępujący tekst dziedziczy
interpunkcję i wielkość liter z otoczenia; co ze spacjami na granicach; czy
zaznaczenie obejmujące pół słowa jest rozszerzane do granicy słowa.

### 6. Słownik i wymuszanie pisowni

Nazwy własne, terminy techniczne, nazwiska. Dwa poziomy: `initial_prompt`
(wpływa na model, działa miękko) i podstawienia po transkrypcji (działają
twardo, ale potrafią zepsuć poprawny tekst).

Do rozstrzygnięcia: czy podstawienia są dosłowne, czy rozmyte, i czy słownik
jest wspólny, czy per profil językowy.

### 7. Sygnalizacja niepewności

Whisper zwraca `avg_logprob`. Do rozstrzygnięcia, czy oznaczać segmenty
o niskiej pewności wizualnie, czy proponować alternatywy, czy ignorować
metrykę — jest zaszumiona i próg jest zależny od modelu.

### 8. Punkt wejścia dla LLM-a

Redakcja lokalnym LLM-em wpina się tutaj. Do rozstrzygnięcia: czy działa na
żądanie, czy w tle; na całej notatce, czy na akapicie; czy pokazuje diff przed
zastosowaniem. Mechanika jest gotowa — LLM produkuje operacje `Splice`, tak
samo odwracalne jak każde inne ([03](03-model-dokumentu.md#punkt-zaczepienia-dla-llm-a)).

## Zasada nadrzędna

**Użytkownik zawsze widzi, co narzędzie zrobiło, i zawsze może to cofnąć
jednym skrótem.** Każda automatyczna transformacja jest operacją w logu, nie
niewidzialnym przetworzeniem. To wynika z modelu dokumentu i nie podlega
negocjacji przy projektowaniu kolejnych zachowań — heurystyka, której nie da
się cofnąć, jest gorsza od braku heurystyki.

## Następny krok

Osobna sesja projektowa dla tego dokumentu, po zebraniu doświadczeń
z używania etapu 1 ([09-roadmapa.md](09-roadmapa.md)). Realne dyktowanie
kilkunastu notatek pokaże, które z powyższych punktów faktycznie przeszkadzają,
a które są problemem wyobrażonym.
