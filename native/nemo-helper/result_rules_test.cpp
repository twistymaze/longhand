#include "result_rules.h"
#include <stdexcept>

void require(bool value) { if (!value) throw std::runtime_error("result contract failed"); }
int main() {
    nemo_speech::asr::Result result;
    require(skip_empty_final(result));
    result.alternatives.emplace_back();
    require(skip_empty_final(result));
    result.late_punctuation = ".";
    // The caller emits late punctuation first, without advancing the result ID.
    require(skip_empty_final(result));
    result.is_final = false;
    require(!skip_empty_final(result));
    result.is_final = true;
    result.alternatives.front().transcript = "word";
    require(!skip_empty_final(result));
    result.alternatives.front().transcript.clear();
    result.alternatives.front().words.emplace_back();
    require(!skip_empty_final(result));
}
