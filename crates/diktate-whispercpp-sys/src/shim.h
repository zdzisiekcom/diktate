// Wąskie C API nad whisper.cpp — jedyne, co widzi Rust.
//
// Po co warstwa pośrednia, skoro whisper.h i parakeet.h są już w C:
// `whisper_full_params` ma 61 pól i jest przekazywana przez wartość, a
// `parakeet_full_params` niewiele mniej. Odwzorowanie ich układu po stronie
// Rusta znaczyłoby, że każda zmiana pola w whisper.cpp psuje nas po cichu —
// bez błędu kompilacji, za to z przesuniętymi wartościami w czasie pracy.
// Tutaj układ struktur zna kompilator C++, a przez granicę FFI przechodzą
// wyłącznie wskaźniki i liczby.
//
// Zasada właścicielstwa: tekst i komunikat błędu należą do kontekstu i żyją
// do następnego wywołania. Nic nie jest alokowane po jednej stronie granicy,
// a zwalniane po drugiej.

#ifndef DIKTATE_SHIM_H
#define DIKTATE_SHIM_H

#include <stdbool.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct dk_whisper  dk_whisper;
typedef struct dk_parakeet dk_parakeet;

// NULL = nie udało się wczytać modelu.
dk_whisper * dk_whisper_open(const char * model_path, bool use_gpu);
void         dk_whisper_close(dk_whisper * ctx);

// 0 = sukces, wartość ujemna = błąd (opis pod `dk_whisper_error`).
// `language` i `initial_prompt` mogą być NULL. `beam_size <= 1` znaczy
// dekodowanie zachłanne, `threads <= 0` zostawia decyzję whisperowi.
int dk_whisper_transcribe(dk_whisper * ctx,
                          const float * pcm, int n_samples,
                          const char * language, const char * initial_prompt,
                          int beam_size, int threads);

// Wynik ostatniej udanej transkrypcji: segmenty rozdzielone znakiem nowej
// linii. Wskaźnik jest ważny do następnego wywołania na tym kontekście.
const char * dk_whisper_text(const dk_whisper * ctx);
const char * dk_whisper_error(const dk_whisper * ctx);

// `use_gpu` jest tu osobno i celowo: na Meteor Lake enkoder parakeeta liczy
// się na iGPU 2,5× wolniej niż na rdzeniach, więc daemon podaje `false`,
// mimo że whisper w tym samym procesie chodzi po Vulkanie.
dk_parakeet * dk_parakeet_open(const char * model_path, bool use_gpu);
void          dk_parakeet_close(dk_parakeet * ctx);

int dk_parakeet_transcribe(dk_parakeet * ctx,
                           const float * pcm, int n_samples,
                           int threads);

const char * dk_parakeet_text(const dk_parakeet * ctx);
const char * dk_parakeet_error(const dk_parakeet * ctx);

// Minimalna długość audio, jakiej wymaga parakeet (sekundy). Model sam
// wykrywa język i przy krótszym materiale myli go — szczegóły w docs/12.
float dk_parakeet_min_audio_s(void);

#ifdef __cplusplus
}
#endif

#endif
