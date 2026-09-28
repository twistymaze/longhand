#pragma once
#include "types.h"

// Empty endpoint results must not take the preceding utterance's identity.
inline bool skip_empty_final(const nemo_speech::asr::Result& result) {
    return result.is_final &&
        (result.alternatives.empty() ||
         (result.alternatives.front().transcript.empty() && result.alternatives.front().words.empty()));
}
