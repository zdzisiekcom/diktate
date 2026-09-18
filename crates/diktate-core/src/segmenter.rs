//! Składanie ramek w segmenty na podstawie decyzji VAD.
//!
//! Zasada: prosta maszyna stanów. W stanie nasłuchu (`Idle`) trzymamy tylko
//! krótki bufor kroczący (`pre_roll_ms`) — próbki sprzed ewentualnego
//! początku mowy, żeby nie ucinać pierwszej głoski. Gdy VAD zgłosi mowę,
//! bufor kroczący staje się początkiem trwającego segmentu (`Active`).
//! Segment kończy się, gdy cisza trwa dłużej niż `hangover_ms` (ogon ciszy
//! zostaje w segmencie — whisper sobie z nim radzi) albo gdy jego długość
//! przekroczy `max_segment_ms` (twardy limit, cięcie mimo trwającej mowy).
//! Segment, w którym samej mowy było mniej niż `min_speech_ms`, jest
//! odrzucany — to filtr na stuknięcia i szum, nie na właściwą wypowiedź.

use std::collections::VecDeque;

use crate::SAMPLE_RATE;

/// Parametry składania ramek w segmenty. Wartości domyślne dobrane pod
/// dyktowanie mową ciągłą przy ramkach 512 próbek / 16 kHz (32 ms).
#[derive(Debug, Clone, Copy)]
pub struct SegmenterConfig {
    /// Ile ciszy przed początkiem mowy dołączyć do segmentu.
    pub pre_roll_ms: u32,
    /// Ile ciągłej ciszy kończy segment.
    pub hangover_ms: u32,
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
            hangover_ms: 700,
            min_speech_ms: 300,
            max_segment_ms: 15_000,
        }
    }
}

fn ms_to_samples(ms: u32) -> usize {
    (ms as u64 * SAMPLE_RATE as u64 / 1000) as usize
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

/// Składa strumień ramek (z decyzją VAD dla każdej) w segmenty wypowiedzi.
pub struct Segmenter {
    pre_roll_samples: usize,
    hangover_samples: usize,
    min_speech_samples: usize,
    max_segment_samples: usize,
    state: State,
}

impl Segmenter {
    pub fn new(config: SegmenterConfig) -> Self {
        Self {
            pre_roll_samples: ms_to_samples(config.pre_roll_ms),
            hangover_samples: ms_to_samples(config.hangover_ms),
            min_speech_samples: ms_to_samples(config.min_speech_ms),
            max_segment_samples: ms_to_samples(config.max_segment_ms),
            state: State::Idle {
                preroll: VecDeque::new(),
            },
        }
    }

    /// Dokłada ramkę wraz z decyzją VAD. Zwraca gotowy segment, gdy ten się
    /// właśnie domknął (przez hangover, limit długości albo odrzucenie jako
    /// zbyt krótki — w tym ostatnim przypadku zwraca `None`, tak jak przy
    /// braku domknięcia).
    pub fn push(&mut self, chunk: &[f32], is_speech: bool) -> Option<Vec<f32>> {
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
    pub fn flush(&mut self) -> Option<Vec<f32>> {
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
            } => self.finalize(buffer, speech_samples),
        }
    }

    /// Sprawdza limit długości i hangover po dołożeniu ramki do bufora
    /// segmentu i zwraca nowy stan wraz z ewentualnym gotowym segmentem.
    fn advance(
        &self,
        mut buffer: Vec<f32>,
        speech_samples: usize,
        silence_run: usize,
        is_speech: bool,
    ) -> (State, Option<Vec<f32>>) {
        if buffer.len() >= self.max_segment_samples {
            // Twardy limit: tniemy dokładnie na granicy, nadmiar (z tej
            // samej ramki, która przekroczyła limit) idzie dalej — jako
            // ciąg dalszy mowy albo jako nowy bufor kroczący ciszy.
            let overflow = buffer.split_off(self.max_segment_samples);
            let result = self.finalize(buffer, speech_samples);
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
        } else if silence_run >= self.hangover_samples {
            let result = self.finalize(buffer, speech_samples);
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

    /// Odrzuca segment, w którym mowy było mniej niż `min_speech_samples`.
    fn finalize(&self, buffer: Vec<f32>, speech_samples: usize) -> Option<Vec<f32>> {
        if speech_samples >= self.min_speech_samples {
            Some(buffer)
        } else {
            None
        }
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
        let hangover_samples = ms_to_samples(config.hangover_ms);
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

        // Cisza aż do przekroczenia hangover_ms.
        let silence_frames_needed = hangover_samples.div_ceil(CHUNK);
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
        assert_eq!(segment.len(), expected_len);

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

        // Domykamy hangoverem.
        let hangover_samples = ms_to_samples(config.hangover_ms);
        let silence_frames_needed = hangover_samples.div_ceil(CHUNK);
        let mut segment = None;
        for _ in 0..silence_frames_needed {
            if let Some(s) = seg.push(&frame(0.0), false) {
                segment = Some(s);
            }
        }
        let segment = segment.expect("segment powinien się domknąć");

        // Pierwsze `pre_roll_samples` próbek to bufor kroczący (wartość 9.0).
        assert!(segment[..pre_roll_samples].iter().all(|&s| s == 9.0));
        // Zaraz po nim — ramka mowy (wartość 3.0).
        assert!(segment[pre_roll_samples..pre_roll_samples + CHUNK]
            .iter()
            .all(|&s| s == 3.0));
    }

    #[test]
    fn krotkie_stukniecie_ponizej_min_speech_jest_odrzucone() {
        let config = SegmenterConfig::default();
        let hangover_samples = ms_to_samples(config.hangover_ms);
        let mut seg = Segmenter::new(config);

        // 2 ramki mowy = 1024 próbek < min_speech_samples (4800).
        assert!(seg.push(&frame(1.0), true).is_none());
        assert!(seg.push(&frame(1.0), true).is_none());

        let silence_frames_needed = hangover_samples.div_ceil(CHUNK);
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
        assert!(result.is_some(), "po odrzuceniu stuknięcia segmenter powinien działać dalej");
    }

    #[test]
    fn mowa_dluzsza_niz_max_segment_jest_przycieta() {
        let config = SegmenterConfig {
            pre_roll_ms: 0,
            hangover_ms: 1_000,
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
        assert_eq!(segment.len(), max_samples);
    }

    #[test]
    fn flush_domyka_trwajacy_segment() {
        let mut seg = Segmenter::new(SegmenterConfig::default());
        let speech_frames = 12; // 12 * 512 = 6144 > min_speech_samples (4800)
        for _ in 0..speech_frames {
            assert!(seg.push(&frame(1.0), true).is_none());
        }
        let segment = seg.flush().expect("flush powinien domknąć trwający segment");
        assert_eq!(segment.len(), speech_frames * CHUNK);

        // Po flushu nie ma już nic do domknięcia.
        assert!(seg.flush().is_none());
    }
}
