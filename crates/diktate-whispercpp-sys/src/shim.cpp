// Implementacja API z shim.h. Wszystko, co wie o układzie struktur
// whisper.cpp, mieszka tutaj — patrz komentarz w nagłówku.

#include "shim.h"

#include "whisper.h"
#include "parakeet.h"

#include <string>
#include <vector>

namespace {

// Skleja segmenty w jeden tekst rozdzielony znakami nowej linii. Podziału na
// segmenty nie zjadamy tu na własną rękę: to Rust decyduje, jak je złożyć,
// a granica FFI ma być głupia.
template <typename Count, typename Text>
std::string join_segments(Count count, Text text) {
    std::string out;
    const int n = count();
    for (int i = 0; i < n; ++i) {
        const char * segment = text(i);
        if (segment == nullptr) {
            continue;
        }
        if (!out.empty()) {
            out.push_back('\n');
        }
        out += segment;
    }
    return out;
}

} // namespace

struct dk_whisper {
    whisper_context * ctx = nullptr;
    std::string text;
    std::string error;
};

struct dk_parakeet {
    parakeet_context * ctx = nullptr;
    std::string text;
    std::string error;
};

extern "C" {

dk_whisper * dk_whisper_open(const char * model_path, bool use_gpu) {
    if (model_path == nullptr) {
        return nullptr;
    }
    whisper_context_params ctx_params = whisper_context_default_params();
    ctx_params.use_gpu = use_gpu;

    whisper_context * ctx = whisper_init_from_file_with_params(model_path, ctx_params);
    if (ctx == nullptr) {
        return nullptr;
    }
    dk_whisper * handle = new dk_whisper();
    handle->ctx = ctx;
    return handle;
}

void dk_whisper_close(dk_whisper * handle) {
    if (handle == nullptr) {
        return;
    }
    if (handle->ctx != nullptr) {
        whisper_free(handle->ctx);
    }
    delete handle;
}

int dk_whisper_transcribe(dk_whisper * handle,
                          const float * pcm, int n_samples,
                          const char * language, const char * initial_prompt,
                          int beam_size, int threads) {
    if (handle == nullptr || handle->ctx == nullptr) {
        return -1;
    }
    handle->text.clear();
    handle->error.clear();
    if (pcm == nullptr || n_samples <= 0) {
        return 0;
    }

    const bool beam = beam_size > 1;
    whisper_full_params params = whisper_full_default_params(
        beam ? WHISPER_SAMPLING_BEAM_SEARCH : WHISPER_SAMPLING_GREEDY);
    if (beam) {
        params.beam_search.beam_size = beam_size;
        params.beam_search.patience  = 0.0f;
    } else {
        params.greedy.best_of = 1;
    }
    if (language != nullptr && language[0] != '\0') {
        params.language = language;
    }
    if (initial_prompt != nullptr && initial_prompt[0] != '\0') {
        params.initial_prompt = initial_prompt;
    }
    if (threads > 0) {
        params.n_threads = threads;
    }
    params.translate        = false;
    params.print_special    = false;
    params.print_progress   = false;
    params.print_realtime   = false;
    params.print_timestamps = false;

    const int ret = whisper_full(handle->ctx, params, pcm, n_samples);
    if (ret != 0) {
        handle->error = "whisper_full zwrócił " + std::to_string(ret);
        return -2;
    }
    whisper_context * ctx = handle->ctx;
    handle->text = join_segments(
        [ctx] { return whisper_full_n_segments(ctx); },
        [ctx](int i) { return whisper_full_get_segment_text(ctx, i); });
    return 0;
}

const char * dk_whisper_text(const dk_whisper * handle) {
    return handle == nullptr ? "" : handle->text.c_str();
}

const char * dk_whisper_error(const dk_whisper * handle) {
    return handle == nullptr ? "" : handle->error.c_str();
}

dk_parakeet * dk_parakeet_open(const char * model_path, bool use_gpu) {
    if (model_path == nullptr) {
        return nullptr;
    }
    parakeet_context_params ctx_params = parakeet_context_default_params();
    ctx_params.use_gpu = use_gpu;

    parakeet_context * ctx = parakeet_init_from_file_with_params(model_path, ctx_params);
    if (ctx == nullptr) {
        return nullptr;
    }
    dk_parakeet * handle = new dk_parakeet();
    handle->ctx = ctx;
    return handle;
}

void dk_parakeet_close(dk_parakeet * handle) {
    if (handle == nullptr) {
        return;
    }
    if (handle->ctx != nullptr) {
        parakeet_free(handle->ctx);
    }
    delete handle;
}

int dk_parakeet_transcribe(dk_parakeet * handle,
                           const float * pcm, int n_samples,
                           int threads) {
    if (handle == nullptr || handle->ctx == nullptr) {
        return -1;
    }
    handle->text.clear();
    handle->error.clear();
    if (pcm == nullptr || n_samples <= 0) {
        return 0;
    }

    parakeet_full_params params = parakeet_full_default_params(PARAKEET_SAMPLING_GREEDY);
    if (threads > 0) {
        params.n_threads = threads;
    }

    const int ret = parakeet_full(handle->ctx, params, pcm, n_samples);
    if (ret != 0) {
        handle->error = "parakeet_full zwrócił " + std::to_string(ret);
        return -2;
    }
    parakeet_context * ctx = handle->ctx;
    handle->text = join_segments(
        [ctx] { return parakeet_full_n_segments(ctx); },
        [ctx](int i) { return parakeet_full_get_segment_text(ctx, i); });
    return 0;
}

const char * dk_parakeet_text(const dk_parakeet * handle) {
    return handle == nullptr ? "" : handle->text.c_str();
}

const char * dk_parakeet_error(const dk_parakeet * handle) {
    return handle == nullptr ? "" : handle->error.c_str();
}

float dk_parakeet_min_audio_s(void) {
    return 3.0f;
}

} // extern "C"
