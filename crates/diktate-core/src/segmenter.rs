//! Składanie ramek w segmenty na podstawie decyzji VAD.
//!
//! Zasada: prosta maszyna stanów. W stanie nasłuchu (`Idle`) trzymamy tylko
//! krótki bufor kroczący (`pre_roll_ms`) — próbki sprzed ewentualnego
//! początku mowy, żeby nie ucinać pierwszej głoski. Gdy VAD zgłosi mowę,
//! bufor kroczący staje się początkiem trwającego segmentu (`Active`).
//!
//! Segment domyka cisza — ale próg tej ciszy **zależy od tego, jak długo
//! trwa już bieżąca wypowiedź** (`docs/07-zachowanie-dyktowania.md`,
//! „Rytm: adaptacyjny próg ciszy"). Jeden stały próg nie obsługuje obu
//! sytuacji naraz: krótki tnie zdanie na pauzie przed trudnym słowem,
//! długi każe czekać po każdej wypowiedzi i nie kończy monologu, dopóki
//! mówiący nie zrobi wyraźnej przerwy. Stąd:
//!
//! ```text
//! długość segmentu <= long_segment_ms  →  cisza kończąca = silence_ms_to_end
//! długość segmentu >  long_segment_ms  →  cisza kończąca = silence_ms_to_end_long
//! ```
//!
//! Na początku wypowiedzi użytkownik ma czas na zastanowienie. Gdy mówi już
//! dłuższą chwilę, każda pauza kończy segment niemal natychmiast — tekst
//! zaczyna płynąć na ekran w trakcie mówienia, zamiast spaść blokiem na
//! końcu. Za „długość segmentu" bierzemy rozmiar bufora, a więc czas od
//! początku wypowiedzi wraz z ciszą w środku, nie samą ilość mowy.
//!
//! `max_segment_ms` pozostaje twardym cięciem — po nim tniemy mimo
//! trwającej mowy, bo okno whispera to 30 s.
//!
//! Segment, w którym samej mowy było mniej niż `min_speech_ms`, jest
//! odrzucany — to filtr na stuknięcia i szum, nie na właściwą wypowiedź.
//! `flush()` domyka trwający segment od razu (koniec sesji — nie ma po co
//! dalej zbierać), ale nadal odrzuca go, jeśli mowy było za mało.

use std::collections::VecDeque;

use crate::SAMPLE_RATE;

/// Parametry składania ramek w segmenty. Wartości domyślne pochodzą
/// z prototypu, w którym rytm dyktowania sprawdził się w praktyce
/// (`docs/07`), i zakładają ramki 512 próbek / 16 kHz (32 ms).
#[derive(Debug, Clone, Copy)]
pub struct SegmenterConfig {
    /// Ile ciszy przed początkiem mowy dołączyć do segmentu.
    pub pre_roll_ms: u32,
    /// Cisza domykająca segment, dopóki wypowiedź jest krótsza niż
    /// `long_segment_ms` — na tyle długa, żeby wybaczyć zawahanie przed
    /// trudnym słowem.
    pub silence_ms_to_end: u32,
    /// Cisza domykająca segment, gdy wypowiedź trwa już dłużej niż
    /// `long_segment_ms`. Krótka, bo mówiący jest w rytmie i każda pauza
    /// między zdaniami ma od razu wypchnąć tekst na ekran.
    pub silence_ms_to_end_long: u32,
    /// Po jakiej długości wypowiedzi przełączyć się na krótszy próg ciszy.
    pub long_segment_ms: u32,
    /// Minimalna ilość samej mowy w segmencie; krócej — odrzucamy jako
    /// stuknięcie lub szum.
    pub min_speech_ms: u32,
    /// Twardy limit długości segmentu — po nim tniemy mimo trwającej mowy.
    pub max_segment_ms: u32,
}

impl Default for SegmenterConfig {
    fn default() -> Self {
        Self {
            pre_roll_ms: 300,
            silence_ms_to_end: 1_500,
            silence_ms_to_end_long: 200,
            long_segment_ms: 2_500,
            min_speech_ms: 300,
            max_segment_ms: 15_000,
        }
    }
}

fn ms_to_samples(ms: u32) -> usize {
    (ms as u64 * SAMPLE_RATE as u64 / 1000) as usize
}

fn samples_to_ms(samples: usize) -> u64 {
    samples as u64 * 1000 / SAMPLE_RATE as u64
}

/// Wewnętrzny stan maszyny.
enum State {
    /// Nasłuch: `preroll` to bufor kroczący, capped na `pre_roll_samples`.
    Idle { preroll: VecDeque<f32> },
    /// Trwający segment: `buffer` to zebrane próbki, `speech_samples` to
    /// ile z nich VAD oznaczył jako mowę, `silence_run` to długość
    /// bieżącej, nieprzerwanej ciszy na końcu bufora.
    Active {
        buffer: Vec<f32>,
        speech_samples: usize,
        silence_run: usize,
    },
}

/// Gotowa wypowiedź wraz z jej położeniem w osi czasu sesji.
///
/// Oś czasu audio — a nie kolejność przybycia — jest jedynym prawdziwym
/// porządkiem wypowiedzi (`docs/02-protokol-ipc.md`): przy zaległościach
/// kolejki transkrypcji segment wypowiedziany wcześniej potrafi dotrzeć do
/// klienta później. Dlatego segmenter podaje pozycję wycinka w próbkach od
/// początku sesji, a nie tylko same próbki.
#[derive(Debug, Clone, PartialEq)]
pub struct Segment {
    /// Próbki wypowiedzi (mono, 16 kHz) wraz z pre-rollem i ciszą domykającą.
    pub audio: Vec<f32>,
    /// Numer pierwszej próbki segmentu, licząc od początku sesji.
    pub start_sample: usize,
    /// Numer próbki tuż za ostatnią próbką segmentu.
    pub end_sample: usize,
}

impl Segment {
    /// Początek segmentu w milisekundach od startu sesji.
    pub fn start_ms(&self) -> u64 {
        samples_to_ms(self.start_sample)
    }

    /// Koniec segmentu w milisekundach od startu sesji.
    pub fn end_ms(&self) -> u64 {
        samples_to_ms(self.end_sample)
    }
}

/// Składa strumień ramek (z decyzją VAD dla każdej) w segmenty wypowiedzi.
pub struct Segmenter {
    pre_roll_samples: usize,
    silence_to_end_samples: usize,
    silence_to_end_long_samples: usize,
    long_segment_samples: usize,
    min_speech_samples: usize,
    max_segment_samples: usize,
    state: State,
    /// Liczba próbek przyjętych od początku sesji — oś czasu, w której
    /// osadzany jest każdy zwrócony [`Segment`].
    consumed: usize,
}

impl Segmenter {
    pub fn new(config: SegmenterConfig) -> Self {
        Self {
            pre_roll_samples: ms_to_samples(config.pre_roll_ms),
            silence_to_end_samples: ms_to_samples(config.silence_ms_to_end),
            silence_to_end_long_samples: ms_to_samples(config.silence_ms_to_end_long),
            long_segment_samples: ms_to_samples(config.long_segment_ms),
            min_speech_samples: ms_to_samples(config.min_speech_ms),
            max_segment_samples: ms_to_samples(config.max_segment_ms),
            state: State::Idle {
                preroll: VecDeque::new(),
            },
            consumed: 0,
        }
    }

    /// Dokłada ramkę wraz z decyzją VAD. Zwraca gotowy segment, gdy ten się
    /// właśnie domknął (przez hangover, limit długości albo odrzucenie jako
    /// zbyt krótki — w tym ostatnim przypadku zwraca `None`, tak jak przy
    /// braku domknięcia).
    pub fn push(&mut self, chunk: &[f32], is_speech: bool) -> Option<Segment> {
        self.consumed += chunk.len();
        let state = std::mem::replace(
            &mut self.state,
            State::Idle {
                preroll: VecDeque::new(),
            },
        );
        let (new_state, result) = match state {
            State::Idle { mut preroll } => {
                if is_speech {
                    let mut buffer = Vec::with_capacity(preroll.len() + chunk.len());
                    buffer.extend(preroll.drain(..));
                    buffer.extend_from_slice(chunk);
                    let speech_samples = chunk.len();
                    self.advance(buffer, speech_samples, 0, true)
                } else {
                    preroll.extend(chunk.iter().copied());
                    while preroll.len() > self.pre_roll_samples {
                        preroll.pop_front();
                    }
                    (State::Idle { preroll }, None)
                }
            }
            State::Active {
                mut buffer,
                mut speech_samples,
                mut silence_run,
            } => {
                buffer.extend_from_slice(chunk);
                if is_speech {
                    speech_samples += chunk.len();
                    silence_run = 0;
                } else {
                    silence_run += chunk.len();
                }
                self.advance(buffer, speech_samples, silence_run, is_speech)
            }
        };
        self.state = new_state;
        result
    }

    /// Domyka trwający segment na koniec sesji (ta sama reguła
    /// `min_speech_ms` co w `push`). Bez trwającego segmentu — `None`.
    pub fn flush(&mut self) -> Option<Segment> {
        let state = std::mem::replace(
            &mut self.state,
            State::Idle {
                preroll: VecDeque::new(),
            },
        );
        match state {
            State::Idle { .. } => None,
            State::Active {
                buffer,
                speech_samples,
                ..
            } => self.finalize(buffer, speech_samples, self.consumed),
        }
    }

    /// Sprawdza limit długości i regułę domykania ciszą po dołożeniu ramki
    /// do bufora segmentu i zwraca nowy stan wraz z ewentualnym gotowym
    /// segmentem.
    fn advance(
        &self,
        mut buffer: Vec<f32>,
        speech_samples: usize,
        silence_run: usize,
        is_speech: bool,
    ) -> (State, Option<Segment>) {
        if buffer.len() >= self.max_segment_samples {
            // Twardy limit: tniemy dokładnie na granicy, nadmiar (z tej
            // samej ramki, która przekroczyła limit) idzie dalej — jako
            // ciąg dalszy mowy albo jako nowy bufor kroczący ciszy.
            let overflow = buffer.split_off(self.max_segment_samples);
            // Nadmiar wraca do następnego segmentu, więc ten kończy się
            // tyle próbek przed bieżącą pozycją, ile go odłożyliśmy.
            let result = self.finalize(buffer, speech_samples, self.consumed - overflow.len());
            let new_state = if overflow.is_empty() {
                State::Idle {
                    preroll: VecDeque::new(),
                }
            } else if is_speech {
                State::Active {
                    speech_samples: overflow.len(),
                    silence_run: 0,
                    buffer: overflow,
                }
            } else {
                let mut preroll: VecDeque<f32> = overflow.into();
                while preroll.len() > self.pre_roll_samples {
                    preroll.pop_front();
                }
                State::Idle { preroll }
            };
            (new_state, result)
        } else if silence_run >= self.silence_to_end(buffer.len()) {
            // Cisza przekroczyła próg domykający dla tej długości
            // wypowiedzi — uznajemy ją za skończoną.
            let result = self.finalize(buffer, speech_samples, self.consumed);
            (
                State::Idle {
                    preroll: VecDeque::new(),
                },
                result,
            )
        } else {
            (
                State::Active {
                    buffer,
                    speech_samples,
                    silence_run,
                },
                None,
            )
        }
    }

    /// Próg ciszy domykającej segment o podanej długości bufora.
    ///
    /// Krótka wypowiedź dostaje długi próg — użytkownik ma czas na
    /// zastanowienie przed trudnym słowem. Gdy mówi już dłużej niż
    /// `long_segment_ms`, jest w rytmie: wtedy każda pauza kończy segment
    /// niemal natychmiast i tekst płynie na ekran w trakcie mówienia
    /// (`docs/07-zachowanie-dyktowania.md`).
    fn silence_to_end(&self, buffer_len: usize) -> usize {
        if buffer_len > self.long_segment_samples {
            self.silence_to_end_long_samples
        } else {
            self.silence_to_end_samples
        }
    }

    /// Odrzuca segment, w którym mowy było mniej niż `min_speech_samples`.
    /// `end_sample` to pozycja tuż za ostatnią próbką segmentu w osi czasu
    /// sesji; początek wynika z niej i z długości bufora.
    fn finalize(
        &self,
        buffer: Vec<f32>,
        speech_samples: usize,
        end_sample: usize,
    ) -> Option<Segment> {
        if speech_samples < self.min_speech_samples {
            return None;
        }
        Some(Segment {
            start_sample: end_sample - buffer.len(),
            end_sample,
            audio: buffer,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CHUNK: usize = 512;

    fn frame(value: f32) -> Vec<f32> {
        vec![value; CHUNK]
    }

    #[test]
    fn sama_cisza_nigdy_nie_zwraca_segmentu() {
        let mut seg = Segmenter::new(SegmenterConfig::default());
        for _ in 0..50 {
            assert!(seg.push(&frame(0.0), false).is_none());
        }
        assert!(seg.flush().is_none());
    }

    #[test]
    fn mowa_potem_cisza_zwraca_segment_dokladnie_raz() {
        let config = SegmenterConfig::default();
        let pre_roll_samples = ms_to_samples(config.pre_roll_ms);
        let silence_samples = ms_to_samples(config.silence_ms_to_end);
        let mut seg = Segmenter::new(config);

        // Trochę ciszy przed mową — mniej niż limit pre-rolla, więc w
        // całości trafi do segmentu.
        let silence_before = 3;
        for _ in 0..silence_before {
            assert!(seg.push(&frame(0.0), false).is_none());
        }

        // Mowa — wyraźnie ponad min_speech_ms (12 * 512 = 6144 > 4800).
        let speech_frames = 12;
        for _ in 0..speech_frames {
            assert!(seg.push(&frame(1.0), true).is_none());
        }

        // Cisza aż do przekroczenia silence_ms_to_end.
        let silence_frames_needed = silence_samples.div_ceil(CHUNK);
        let mut result = None;
        for i in 0..silence_frames_needed {
            let r = seg.push(&frame(0.0), false);
            if i + 1 < silence_frames_needed {
                assert!(r.is_none(), "segment domknął się za wcześnie");
            } else {
                result = r;
            }
        }

        let segment = result.expect("segment powinien się domknąć");
        // Cisza przed mową była krótsza niż limit pre-rolla, więc trafiła
        // do segmentu w całości — długość to prosta suma wszystkich ramek.
        assert!(silence_before * CHUNK <= pre_roll_samples);
        let expected_len =
            silence_before * CHUNK + speech_frames * CHUNK + silence_frames_needed * CHUNK;
        assert_eq!(segment.audio.len(), expected_len);

        // Kolejna cisza już nie zwraca niczego — segment wrócił dokładnie raz.
        assert!(seg.push(&frame(0.0), false).is_none());
        assert!(seg.flush().is_none());
    }

    #[test]
    fn pre_roll_dokleja_probki_sprzed_poczatku_mowy() {
        let config = SegmenterConfig::default();
        let pre_roll_samples = ms_to_samples(config.pre_roll_ms);
        let mut seg = Segmenter::new(config);

        // Więcej ciszy niż mieści pre-roll — bufor kroczący ma się ustabilizować
        // dokładnie na `pre_roll_samples`, licząc od najnowszych próbek.
        let silence_frames = (pre_roll_samples / CHUNK) + 8;
        for _ in 0..silence_frames {
            assert!(seg.push(&frame(9.0), false).is_none());
        }

        // Wystarczająco dużo mowy, żeby segment nie został odrzucony jako
        // zbyt krótki (12 * 512 = 6144 > min_speech_samples = 4800).
        for _ in 0..12 {
            assert!(seg.push(&frame(3.0), true).is_none());
        }

        // Domykamy zwykłym progiem ciszy.
        let silence_samples = ms_to_samples(config.silence_ms_to_end);
        let silence_frames_needed = silence_samples.div_ceil(CHUNK);
        let mut segment = None;
        for _ in 0..silence_frames_needed {
            if let Some(s) = seg.push(&frame(0.0), false) {
                segment = Some(s);
            }
        }
        let segment = segment.expect("segment powinien się domknąć");

        // Pierwsze `pre_roll_samples` próbek to bufor kroczący (wartość 9.0).
        assert!(segment.audio[..pre_roll_samples].iter().all(|&s| s == 9.0));
        // Zaraz po nim — ramka mowy (wartość 3.0).
        assert!(segment.audio[pre_roll_samples..pre_roll_samples + CHUNK]
            .iter()
            .all(|&s| s == 3.0));
    }

    #[test]
    fn krotkie_stukniecie_ponizej_min_speech_jest_odrzucone() {
        let config = SegmenterConfig::default();
        let silence_samples = ms_to_samples(config.silence_ms_to_end);
        let mut seg = Segmenter::new(config);

        // 2 ramki mowy = 1024 próbek < min_speech_samples (4800).
        assert!(seg.push(&frame(1.0), true).is_none());
        assert!(seg.push(&frame(1.0), true).is_none());

        let silence_frames_needed = silence_samples.div_ceil(CHUNK);
        for _ in 0..silence_frames_needed {
            assert!(
                seg.push(&frame(0.0), false).is_none(),
                "zbyt krótki segment nie powinien zostać zwrócony"
            );
        }

        // Stan wrócił do nasłuchu — kolejna, właściwa wypowiedź działa normalnie.
        for _ in 0..12 {
            assert!(seg.push(&frame(1.0), true).is_none());
        }
        let mut result = None;
        for _ in 0..silence_frames_needed {
            if let Some(s) = seg.push(&frame(0.0), false) {
                result = Some(s);
            }
        }
        assert!(
            result.is_some(),
            "po odrzuceniu stuknięcia segmenter powinien działać dalej"
        );
    }

    #[test]
    fn mowa_dluzsza_niz_max_segment_jest_przycieta() {
        let config = SegmenterConfig {
            pre_roll_ms: 0,
            silence_ms_to_end: 1_000,
            silence_ms_to_end_long: 1_000,
            long_segment_ms: 1_000,
            min_speech_ms: 0,
            max_segment_ms: 100, // 100 ms = 1600 próbek przy 16 kHz
        };
        let max_samples = ms_to_samples(config.max_segment_ms);
        let mut seg = Segmenter::new(config);

        let mut result = None;
        let mut pushed = 0;
        for _ in 0..10 {
            pushed += CHUNK;
            let r = seg.push(&frame(1.0), true);
            if pushed >= max_samples {
                result = r;
                break;
            }
            assert!(r.is_none());
        }

        let segment = result.expect("segment powinien zostać przycięty i zwrócony");
        assert_eq!(segment.audio.len(), max_samples);
    }

    #[test]
    fn flush_domyka_trwajacy_segment() {
        let mut seg = Segmenter::new(SegmenterConfig::default());
        let speech_frames = 12; // 12 * 512 = 6144 > min_speech_samples (4800)
        for _ in 0..speech_frames {
            assert!(seg.push(&frame(1.0), true).is_none());
        }
        let segment = seg
            .flush()
            .expect("flush powinien domknąć trwający segment");
        assert_eq!(segment.audio.len(), speech_frames * CHUNK);

        // Po flushu nie ma już nic do domknięcia.
        assert!(seg.flush().is_none());
    }
    #[test]
    fn pauza_krotsza_niz_prog_nie_dzieli_wypowiedzi_na_dwa_segmenty() {
        // Dopóki wypowiedź jest krótka, pauza poniżej `silence_ms_to_end`
        // jest zawahaniem, a nie końcem zdania — obie części mają trafić
        // do jednego segmentu.
        let config = SegmenterConfig {
            pre_roll_ms: 0,
            silence_ms_to_end: 1_500,
            silence_ms_to_end_long: 200,
            long_segment_ms: 2_500,
            min_speech_ms: 100,
            max_segment_ms: 15_000,
        };
        let mut seg = Segmenter::new(config);

        // Pierwsza krótka wypowiedź.
        for _ in 0..6 {
            assert!(seg.push(&frame(1.0), true).is_none());
        }

        // Pauza krótsza niż `silence_ms_to_end` (6 ramek = 192 ms).
        let pause_frames = 6;
        for _ in 0..pause_frames {
            assert!(
                seg.push(&frame(0.0), false).is_none(),
                "segment nie powinien domknąć się na krótkiej pauzie"
            );
        }

        // Druga krótka wypowiedź — powinna trafić do TEGO SAMEGO segmentu.
        for _ in 0..6 {
            assert!(seg.push(&frame(1.0), true).is_none());
        }

        // Dopiero flush domyka — potwierdza, że przez cały czas był to
        // jeden, wciąż otwarty segment, a nie dwa oddzielnie zwrócone.
        let segment = seg.flush().expect("flush powinien domknąć zebrany segment");
        assert_eq!(segment.audio.len(), (6 + pause_frames + 6) * CHUNK);
    }

    #[test]
    fn krotka_wypowiedz_czeka_na_dlugi_prog_ciszy() {
        // Wypowiedź poniżej `long_segment_ms` domyka dopiero pełny
        // `silence_ms_to_end` — krótsza cisza ma ją zostawić otwartą.
        let config = SegmenterConfig {
            pre_roll_ms: 0,
            silence_ms_to_end: 1_500,
            silence_ms_to_end_long: 200,
            long_segment_ms: 2_500,
            min_speech_ms: 100,
            max_segment_ms: 15_000,
        };
        let short_silence_frames = ms_to_samples(config.silence_ms_to_end_long).div_ceil(CHUNK);
        let full_silence_frames = ms_to_samples(config.silence_ms_to_end).div_ceil(CHUNK);
        let mut seg = Segmenter::new(config);

        // 1 s mowy — wyraźnie poniżej long_segment_ms.
        for _ in 0..32 {
            assert!(seg.push(&frame(1.0), true).is_none());
        }

        // Krótki próg nie wystarcza: po tylu ramkach ciszy segment nadal trwa.
        for _ in 0..short_silence_frames {
            assert!(
                seg.push(&frame(0.0), false).is_none(),
                "krótka wypowiedź nie powinna domykać się krótkim progiem"
            );
        }

        // Dopiero pełny próg domyka.
        let mut result = None;
        for _ in short_silence_frames..full_silence_frames {
            result = seg.push(&frame(0.0), false).or(result);
        }
        result.expect("po silence_ms_to_end krótka wypowiedź powinna się domknąć");
    }

    #[test]
    fn dluga_wypowiedz_domyka_sie_juz_na_krotkiej_pauzie() {
        // Sedno adaptacyjnego progu: gdy mówiący jest w rytmie dłużej niż
        // `long_segment_ms`, pauza między zdaniami ma natychmiast wypchnąć
        // tekst na ekran, zamiast kazać czekać na pełny `silence_ms_to_end`.
        let config = SegmenterConfig {
            pre_roll_ms: 0,
            silence_ms_to_end: 1_500,
            silence_ms_to_end_long: 300,
            long_segment_ms: 1_000,
            min_speech_ms: 100,
            max_segment_ms: 15_000,
        };
        let short_silence_frames = ms_to_samples(config.silence_ms_to_end_long).div_ceil(CHUNK);
        let full_silence_frames = ms_to_samples(config.silence_ms_to_end).div_ceil(CHUNK);
        let mut seg = Segmenter::new(config);

        // 1,5 s mowy — powyżej long_segment_ms (1 s).
        for _ in 0..48 {
            assert!(seg.push(&frame(1.0), true).is_none());
        }

        let mut result = None;
        for i in 0..short_silence_frames {
            let r = seg.push(&frame(0.0), false);
            if i + 1 < short_silence_frames {
                assert!(r.is_none(), "segment domknął się za wcześnie");
            } else {
                result = r;
            }
        }

        result.expect("długa wypowiedź powinna domknąć się już po silence_ms_to_end_long");
        assert!(
            short_silence_frames < full_silence_frames,
            "test ma sens tylko wtedy, gdy krótki próg jest istotnie krótszy"
        );
    }

    #[test]
    fn wypowiedz_dokladnie_na_progu_uzywa_jeszcze_dlugiej_ciszy() {
        // Granica jest ostra: na krótki próg przechodzimy dopiero POWYŻEJ
        // `long_segment_ms`, nie dokładnie na nim. Liczby dobrane tak, by
        // krótki próg ciszy dopełniał się w tej samej ramce, w której bufor
        // osiąga dokładnie `long_segment_ms` — zamiana `>` na `>=` w regule
        // wywala ten test.
        let config = SegmenterConfig {
            pre_roll_ms: 0,
            silence_ms_to_end: 1_500,
            silence_ms_to_end_long: 320, // 5120 próbek = 10 ramek
            long_segment_ms: 992,        // 15872 próbek = 31 ramek
            min_speech_ms: 100,
            max_segment_ms: 15_000,
        };
        let long_segment_samples = ms_to_samples(config.long_segment_ms);
        let short_silence_frames = ms_to_samples(config.silence_ms_to_end_long) / CHUNK;
        let speech_frames = long_segment_samples / CHUNK - short_silence_frames;
        let mut seg = Segmenter::new(config);

        for _ in 0..speech_frames {
            assert!(seg.push(&frame(1.0), true).is_none());
        }

        // Po tych ramkach bufor ma dokładnie `long_segment_ms`, a cisza
        // dokładnie `silence_ms_to_end_long`. Obowiązuje jeszcze długi próg,
        // więc segment ma pozostać otwarty.
        for _ in 0..short_silence_frames {
            assert!(
                seg.push(&frame(0.0), false).is_none(),
                "na samym progu długości obowiązuje jeszcze silence_ms_to_end"
            );
        }

        // Kolejna ramka przekracza próg długości — teraz krótki próg
        // (już przekroczony) domyka segment natychmiast.
        seg.push(&frame(0.0), false)
            .expect("powyżej progu długości krótka cisza powinna domknąć segment");
    }
}

#[cfg(test)]
mod timeline_tests {
    use super::*;

    const CHUNK: usize = 512;

    fn frame(value: f32) -> Vec<f32> {
        vec![value; CHUNK]
    }

    #[test]
    fn segment_niesie_swoje_polozenie_w_osi_czasu_sesji() {
        // Oś czasu audio jest jedynym prawdziwym porządkiem wypowiedzi
        // (docs/02), więc segmenter musi podać, gdzie w nagraniu leży
        // zwrócony wycinek — a nie tylko same próbki.
        let config = SegmenterConfig::default();
        let pre_roll_samples = ms_to_samples(config.pre_roll_ms);
        let silence_samples = ms_to_samples(config.silence_ms_to_end);
        let mut seg = Segmenter::new(config);

        // Dużo ciszy na początku — więcej, niż mieści pre-roll, żeby
        // początek segmentu nie wypadł przypadkiem na zerze.
        let silence_before = 40;
        for _ in 0..silence_before {
            assert!(seg.push(&frame(0.0), false).is_none());
        }
        let speech_frames = 12;
        for _ in 0..speech_frames {
            assert!(seg.push(&frame(1.0), true).is_none());
        }
        let silence_frames = silence_samples.div_ceil(CHUNK);
        let mut result = None;
        for _ in 0..silence_frames {
            result = seg.push(&frame(0.0), false).or(result);
        }

        let segment = result.expect("segment powinien się domknąć");
        let consumed = (silence_before + speech_frames + silence_frames) * CHUNK;
        assert_eq!(segment.end_sample, consumed);
        assert_eq!(segment.start_sample, consumed - segment.audio.len());
        // Segment zaczyna się dokładnie o pre-roll przed pierwszą ramką mowy.
        assert_eq!(
            segment.start_sample,
            silence_before * CHUNK - pre_roll_samples
        );
    }

    #[test]
    fn kolejne_segmenty_nie_nachodza_na_siebie_w_osi_czasu() {
        let config = SegmenterConfig {
            pre_roll_ms: 0,
            ..SegmenterConfig::default()
        };
        let silence_samples = ms_to_samples(config.silence_ms_to_end);
        let mut seg = Segmenter::new(config);
        let silence_frames = silence_samples.div_ceil(CHUNK);

        let mut segments = Vec::new();
        for _ in 0..2 {
            for _ in 0..12 {
                if let Some(s) = seg.push(&frame(1.0), true) {
                    segments.push(s);
                }
            }
            for _ in 0..silence_frames {
                if let Some(s) = seg.push(&frame(0.0), false) {
                    segments.push(s);
                }
            }
        }

        assert_eq!(segments.len(), 2);
        assert!(
            segments[0].end_sample <= segments[1].start_sample,
            "segmenty nachodzą na siebie: {} > {}",
            segments[0].end_sample,
            segments[1].start_sample
        );
    }

    #[test]
    fn pozycja_w_probkach_przelicza_sie_na_milisekundy() {
        // 16 kHz: 16 próbek to 1 ms. Tę zamianę robi segmenter, bo tylko on
        // zna częstotliwość próbkowania strumienia.
        let segment = Segment {
            audio: vec![0.0; SAMPLE_RATE as usize],
            start_sample: SAMPLE_RATE as usize / 2,
            end_sample: SAMPLE_RATE as usize + SAMPLE_RATE as usize / 2,
        };
        assert_eq!(segment.start_ms(), 500);
        assert_eq!(segment.end_ms(), 1_500);
    }

    #[test]
    fn flush_domyka_segment_z_poprawnym_konncem_osi_czasu() {
        let mut seg = Segmenter::new(SegmenterConfig::default());
        let frames = 20;
        for _ in 0..frames {
            assert!(seg.push(&frame(1.0), true).is_none());
        }
        let segment = seg.flush().expect("flush domyka trwający segment");
        assert_eq!(segment.end_sample, frames * CHUNK);
        assert_eq!(segment.start_sample, 0);
    }
}
