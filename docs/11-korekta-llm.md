# 11 — Korekta tekstu lokalnym LLM-em

Whisper zwraca tekst z błędami rozpoznania: rozbija jedno słowo na kilka
(„ten wis per"), myli wyrazy podobnie brzmiące („morze" zamiast „może"),
gubi końcówki. Dokument opisuje rozpoznanie przeprowadzone 2026-09-20: czy
lokalny model językowy nadaje się do naprawiania takiego tekstu, jak go
wywoływać i gdzie leży granica jego użyteczności.

**Status: wdrożone** jako dwa ręczne przyciski w oknie dyktowania — korekta
transkrypcji ([Wdrożenie](#wdrożenie)) i redakcja poleceniem głosowym
([niżej](#redakcja-poleceniem-głosowym)). Pomiary poniżej są tym, na czym stoją jego
wartości domyślne, więc zmiana progu albo promptu zaczyna się od ich
przeczytania, a nie od zgadywania.

## Serwer

Model działa poza `diktate`, jako osobny serwer `llama.cpp` z API zgodnym
z OpenAI:

```
http://home-host.tarpan-flops.ts.net:8080/v1/
```

Model bywa podmieniany, więc przed użyciem warto sprawdzić, co jest wgrane:

```bash
curl -s http://home-host.tarpan-flops.ts.net:8080/v1/models | jq '.data[].id'
```

Pomiary poniżej dotyczą **`bielik-11b-v3`** — 11,2 mld parametrów,
kwantyzacja Q4_K_Medium (6,7 GB), kontekst 10240 tokenów.

### Thinking

Bielik **nie ma trybu rozumowania**. Parametry `chat_template_kwargs.enable_thinking`
oraz `reasoning_effort` są przez ten model ignorowane — dwanaście próbek
przepuszczonych z flagą i bez niej dało wyniki identyczne co do bajtu, a pole
`message.reasoning_content` zawsze wracało puste.

Ma to znaczenie przy podmianie modelu: wcześniej testowany `ornith-15-35b`
thinking miał, i to **włączony domyślnie**. Model z thinkingiem trzeba wyciszyć
jednym z dwóch równoważnych sposobów, inaczej odpowiedź przychodzi 5–45× wolniej:

```json
"chat_template_kwargs": {"enable_thinking": false}
"reasoning_effort": "none"
```

Druga pułapka modeli z thinkingiem: rozumowanie liczy się do `max_tokens`.
Przy zbyt ciasnym budżecie cały limit idzie na rozumowanie, a `content` wraca
**pusty** — co w pomiarze wyglądało jak awaria serwera, a było wyczerpaniem
tokenów.

## Wywołanie

```bash
curl -s http://home-host.tarpan-flops.ts.net:8080/v1/chat/completions \
  -H 'Content-Type: application/json' -d '{
    "model": "bielik-11b-v3",
    "temperature": 0.0,
    "max_tokens": 2000,
    "messages": [
      {"role": "system", "content": "…prompt korektora, patrz niżej…"},
      {"role": "user", "content": "Popraw:\n\nten wis per przetwarza całe okno"}
    ]}'
```

Mimo `temperature: 0` powtórzone przebiegi tego samego tekstu **bywają różne**.
Nie wolno zakładać, że korekta jest powtarzalna — testy złotych odpowiedzi na
tym serwerze będą migotać.

## Prompt decyduje o wszystkim

To najważniejszy wniosek z pomiaru. Ten sam model, ten sam tekst, dwa różne
prompty systemowe — i dwa przeciwne zachowania.

**Prompt ostrożny** („poprawiasz interpunkcję i słowa błędnie rozpoznane,
fragment poprawny zostaw") daje model bierny. Nie naprawił nawet oczywistego
`ten wis per` — a próbując, pogorszył go na `Ten WIS`. Interpunkcję wstawiał
dobrze, ale błędów rozpoznania praktycznie nie ruszał.

**Prompt z przykładami napraw** uruchamia model. Kluczowe jest pokazanie, że
system rozpoznawania mowy *rozbija słowa*, wraz z konkretnymi parami:

```
Poprawiasz transkrypcje z systemu rozpoznawania mowy. System często ROZBIJA
jedno słowo na kilka albo myli słowa podobnie brzmiące — Twoim głównym
zadaniem jest je odtworzyć.
Przykłady napraw:
  'ten wis per' → 'ten Whisper'
  'w kontener że' → 'w kontenerze'
  'morze tam być' → 'może tam być'
  'na gita bie' → 'na GitHubie'
Usuwaj też wtrącenia (eee, yyy, mmm) i znaczniki typu [muzyka], [śmiech].
Wstawiaj interpunkcję i wielkie litery.
ZAKAZY: nie dodawaj nowych treści, nie skracaj wypowiedzi, nie zmieniaj liczb
ani nazw technicznych, nie zmieniaj stylu. Fragment już poprawny zostaw
nietknięty.
Odpowiadasz wyłącznie poprawionym tekstem.
```

Wyniki tego promptu na korpusie testowym:

| Wejście (transkrypcja) | Wyjście | |
|---|---|---|
| `chrzą szczy brzmi w trzcinie` | **chrząszcz** brzmi w trzcinie | ✔ |
| `ten wis per … koszt segment tu` | Ten **Whisper** … koszt **segmentu** | ✔ |
| `bo morze tam będzie cieplej` | bo **może** tam być cieplej | ✔ |
| `plik konfiguracyjn … pamięc operacyjn` | konfiguracyjny … pamięci operacyjnej | ✔ |
| `[muzyka] dobra to zaczynamy` | Dobra, zaczynamy | ✔ |
| `no i tak eee … jest yyy za wolny` | wtrącenia usunięte | ✔ |
| `po włączeniu wulkana` | po włączeniu **wulkanu** (miało być: Vulkana) | ✘ |
| `W szczeprze szynie` | bez zmian (drugi człon naprawiony) | ✘ |
| `spadła mi rzuta` | spadła mi **rzutka** — słowo zmyślone | ✘ |

## Co model robi dobrze

**Polszczyzny nie psuje.** To nie jest oczywiste: wcześniej testowany
`ornith-15-35b` deterministycznie uszkadzał poprawne wyrazy („trzydzieści" →
„trzydziści") i wstawiał w środek słów znaki spoza alfabetu („trzydzie三十").
Bielik nie zrobił tego ani razu.

**Tekstu poprawnego nie rusza.** Trzy próbki kontrolne — zdania bez błędów,
w tym jedno z `small-q5_1`, `181 MB` i `0,6 s` — wróciły zmienione w **0,0%**.
Liczb i nazw technicznych nie tyka.

**Jest szybki.** Około **0,8 s** na pojedyncze zdanie, **5,6 s** na akapit
129 słów.

## Granica: na akapicie przestaje korygować, a zaczyna redagować

Puszczony na akapit 129 słów model zmienia **33–37% tekstu** i oddaje o 11 słów
mniej, niż dostał. Przepisuje „potem" na „Następnie", „i jak segment się domknie
to leci transkrypcja" na „Gdy segment się domknie, następuje transkrypcja",
dokłada podział na akapity. Efekt czyta się znakomicie — i nie jest już
wypowiedzią użytkownika. Zakaz zmiany stylu wpisany do promptu tego nie
powstrzymał.

Stąd wniosek konstrukcyjny: **korekta ma działać na segmencie, nie na akapicie**.
Segment jest krótki, więc model nie ma pola do przepisywania struktury, a 0,8 s
mieści się w rytmie dyktowania. Zgrywa się to z architekturą — segment jest
i tak naturalną jednostką, w jakiej tekst powstaje
([07-zachowanie-dyktowania.md](07-zachowanie-dyktowania.md)).

## Zabezpieczenia, bez których nie wolno tego włączać

Obowiązuje ta sama reguła, co przy filtrach halucynacji: **narzędzie zjadające
prawdziwy tekst jest gorsze od przepuszczonego błędu**. Model zmyślił już
„rzutkę" tam, gdzie nie wiedział, co usłyszał — a zrobił to cicho, bez żadnego
sygnału, że zgaduje.

1. **Diff zamiast podmiany.** Użytkownik widzi, co się zmieniło, i to on
   zatwierdza. Korekta wchodzi jako operacja odwracalna na modelu dokumentu
   ([03-model-dokumentu.md](03-model-dokumentu.md)), nie jako nadpisanie tekstu.
2. **Próg odsetka zmian — zależny od długości.** Jedna liczba nie wystarczy:
   na zdaniu bezpieczny próg to ~10%, a na akapicie model rutynowo przekracza
   30%, robiąc przy tym coś innego, niż o to proszono.
3. **Walidacja znakowa wyniku.** Odrzucenie odpowiedzi zawierającej znaki spoza
   alfabetu polskiego i interpunkcji. Bielik tego nie potrzebuje, ale po
   podmianie modelu na serwerze zabezpieczenie zadziała bez zmiany kodu — to
   dokładnie ten błąd, który popełniał `ornith`.
4. **Krok ręczny.** Korekta nie uruchamia się sama w trakcie dyktowania.

## Testy

Ważniejsze od testów naprawiania są testy tego, czego naprawić **nie wolno** —
tak samo jak przy filtrach halucynacji
([08-dystrybucja-i-testy.md](08-dystrybucja-i-testy.md#testy-z-nagraniami)).
Korpus musi zawierać próbki kontrolne — zdania poprawne, w tym z nazwami
technicznymi i liczbami — a test ma pilnować, że wracają nietknięte.

Same wywołania modelu nie mogą być częścią `cargo test --workspace`: wymagają
sieci, a wyniki migoczą mimo `temperature: 0`. Logika wokół modelu — liczenie
odsetka zmian, walidacja znakowa, składanie diffu — ma testy jednostkowe
działające bez sieci, na zapisanych parach wejście/wyjście.

## Wdrożenie

Przycisk w pasku nagłówka okna dyktowania (`tools-check-spelling-symbolic`),
opisany od strony interfejsu w [06-gui.md](06-gui.md#przycisk-korekty-tekstu).
Wszystkie pokrętła — łącznie z promptem — leżą w `gtk.toml`, w sekcji
`[correction]` ([04-konfiguracja.md](04-konfiguracja.md#gtktoml)).

### Cała notatka, nie segment

Rozpoznanie kończyło się wnioskiem „korekta ma działać na segmencie".
Wdrożona funkcja robi co innego: poprawia **całą notatkę na żądanie**. To
świadome odstępstwo, a nie przeoczenie — obie rzeczy odpowiadają na inne
pytanie. Korekta segmentu byłaby automatyczna i wchodziłaby w rytm
dyktowania; ta jest ręcznym krokiem po dyktowaniu, kiedy tekst już stoi
i widać go w całości.

Cena jest dokładnie ta, którą pomiar opisał: na dłuższym tekście model
przechodzi z korekty w redakcję. Dlatego to właśnie **na długiej notatce**
próg zmian jest ostrzejszy, a nie odwrotnie (`max_change_ratio_long` = 0,35
wobec `max_change_ratio_short` = 0,5). Krótkie zdanie użytkownik ogarnia
wzrokiem i sam widzi, co się zmieniło; akapitu już nie.

### Zabezpieczenia — co z listy weszło

1. **Diff zamiast podmiany** — weszło w wersji słabszej: zamiast widoku
   różnic użytkownik dostaje `Toast` z odsetkiem zmienionych słów, a korekta
   jest **jedną operacją** logu, więc `Ctrl+Z` cofa ją w całości. Widok
   różnic zostaje do zrobienia.
2. **Próg odsetka zmian zależny od długości** — weszło, wraz z drugą liczbą,
   której rozpoznanie nie przewidziało: `min_changed_words`. Sam odsetek
   alarmował przy każdej uczciwej naprawie krótkiego zdania, bo w notatce
   z ośmiu słów dwie poprawki to 25%. Odsetek zmienionych słów liczony jest
   przy tym **z pominięciem wielkich liter i interpunkcji** — ich wstawianie
   jest zadaniem korekty, a nie jej skutkiem ubocznym, więc miara, która je
   liczy, przestaje cokolwiek znaczyć.
3. **Walidacja znakowa wyniku** — weszło. Odrzucany jest znak spoza alfabetu
   polskiego, cyfr i interpunkcji, **którego nie było w tekście wejściowym**:
   strzałka czy litera grecka wpisana wcześniej ręcznie przez użytkownika nie
   może wywracać korekty.
4. **Krok ręczny** — weszło. Korekta nie uruchamia się sama.

Ponadto, czego lista nie przewidywała: notatka zmieniona w trakcie oczekiwania
na odpowiedź **unieważnia korektę**. Wypowiedź, która wyszła z transkrypcji
w tym czasie, zostałaby podmianą całości zjedzona bez śladu — a to dokładnie
ten rodzaj straty, przed którym korekta ma bronić.

## Redakcja poleceniem głosowym

Druga czynność na tym samym serwerze, odpowiadająca na inne pytanie. Korekta
pyta „co whisper przekręcił" i ma **zakaz** zmiany stylu. Redakcja pyta „zrób
z tym tekstem to, o co proszę" — instrukcją jest zdanie użytkownika, a nie
prompt z konfiguracji.

Przebieg: przycisk `document-edit-symbolic` (skrót `Ctrl+E`) otwiera okno
polecenia, użytkownik **mówi** polecenie, model wykonuje je na zaznaczonym
fragmencie albo, gdy nic nie jest zaznaczone, na całej notatce. Strona
interfejsu: [06-gui.md](06-gui.md#przycisk-redakcji-poleceniem-głosowym).

### Polecenie idzie tą samą drogą co dyktowanie

Protokół nie zmienił się o nic: daemon nadal wysyła `segment.added` i nie wie,
czy wypowiedź jest treścią notatki, czy poleceniem. Rozstrzyga o tym okno —
otwarte okno polecenia rejestruje się jako odbiorca wypowiedzi i przez ten
czas transkrypcja nie dopływa do notatki. Druga sesja z daemonem byłaby tu
drugim mikrofonem i drugim kompletem stanu do pilnowania, a odpowiedź na to
samo pytanie mieszka w jednym `if`.

Zostaje pytanie, co zrobić z zaległościami: wypowiedź powiedziana do notatki
potrafi wyjść z transkrypcji sekundę po otwarciu okna polecenia i wpaść do
niego zamiast do tekstu. Rozstrzyga **oś czasu audio**, nie moment odebrania
wiadomości — polecenie to tylko to, co padło po otwarciu okna. Reguła jest ta
sama, co przy zastępowaniu zaznaczenia (`docs/07`), i ten sam kod ją liczy.

Pierwsze podejście szło na skróty: przycisk nieczynny w trakcie dyktowania.
Okazało się nie do użycia w pierwszej minucie — tekst pojawia się właśnie
w trakcie mówienia, więc przycisk umierał dokładnie wtedy, kiedy był
potrzebny. Znacznik na osi czasu rozwiązuje to samo, nie zabierając nic.

Stąd też druga zasada: okno polecenia gasi mikrofon przy zamknięciu **tylko
wtedy, gdy samo go włączyło**. Przerwane w pół słowa dyktowanie byłoby gorsze
od niedokończonego polecenia.

Cel „cała notatka" pobierany jest ponownie w chwili wykonania polecenia, a nie
otwarcia okna — zaległe zdanie, które dopłynęło w międzyczasie, ma być
zredagowane razem z resztą, a nie unieważnić całą operację. Zaznaczenie
zostaje takie, jakie było: po to trzymają je kotwice.

### Zabezpieczenia — dlaczego inne niż przy korekcie

Progi odsetka zmian tu **nie obowiązują**. Przy korekcie przepisany akapit
jest alarmem: model miał poprawiać, a zaczął redagować. Przy redakcji
przepisany akapit jest wykonanym poleceniem — „zmień na bardziej formalny"
ma zmienić dużo, więc pytanie o zgodę przy 41% zmian byłoby czystym szumem
i nauczyłoby użytkownika klikać „zastosuj" bez patrzenia.

Zostaje reszta i to ona niesie tu cały ciężar:

1. **Jedna operacja logu** — `Ctrl+Z` cofa redakcję w całości. To jest
   właściwa odpowiedź na „model zrobił nie to, o co prosiłem".
2. **Walidacja znakowa** — ta sama co przy korekcie, wspólny kod.
3. **Zakres zapamiętany kotwicami, nie offsetami** — między naciśnięciem
   przycisku a odpowiedzią modelu mija kilka sekund, w których do notatki
   może dopłynąć zaległa wypowiedź i przesunąć każdy offset.
4. **Cel zmieniony w trakcie unieważnia podmianę** — jeśli tekst pod
   kotwicami jest już inny niż ten, który poszedł do modelu, nie podmieniamy
   nic. Odpowiedź dotyczy wtedy czegoś, czego w notatce nie ma.

### Prompt redaktora

Domyślny prompt jest lustrzanym odbiciem korektorskiego: zamiast zakazu
zmiany stylu — polecenie wykonania polecenia, przy zachowanych zakazach
chroniących treść (nie dopisuj, nie zmieniaj liczb i nazw, nie komentuj).
Polecenie użytkownika wchodzi **wiadomością użytkownika**, a nie doklejeniem
do promptu systemowego: to dane, a nie instrukcja programu, i tak też ma je
widzieć model.

Pomiar na `bielik-11b-v3`: „dobra to zaczynamy bo mamy mało czasu" +
polecenie „zmień na bardziej formalny" → „Rozpoczynamy, ponieważ czas jest
ograniczony." — 0,5 s.

### Testy

Logika decyzyjna (odsetek zmian, walidacja znakowa, wyjmowanie tekstu
z odpowiedzi) ma testy jednostkowe bez sieci, w `crates/diktate-gtk/src/correct.rs`.
Wywołanie prawdziwego serwera jest jednym testem oznaczonym `#[ignore]` —
sprawdza drogę, nie treść:

```bash
cargo test -p diktate-gtk -- --ignored korekta
cargo test -p diktate-gtk -- --ignored redakcja
```

Rozgałęzienie wypowiedzi (`redact::Sink`) i wybór celu (`view::Target`) mają
własne testy: pierwsze bez GTK, drugie razem z resztą testów bufora, więc
pod `--ignored`, bo wymagają sesji graficznej.
