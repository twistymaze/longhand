#include <algorithm>
#include <chrono>
#include <cmath>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <limits>
#include <memory>
#include <stdexcept>
#include <string>
#include <thread>
#include <vector>
#include <nlohmann/json.hpp>
#include "nemo_speech/diar.h"
#include "recognizer.h"
#include "result_rules.h"
#include "ggml-backend.h"
#ifdef _WIN32
#include <windows.h>
#include <fcntl.h>
#include <io.h>
#else
#include <unistd.h>
#endif
using Json = nlohmann::json;
namespace asr = nemo_speech::asr;
constexpr size_t max_json = 1024 * 1024;
constexpr size_t max_pcm = 16000;
constexpr uint64_t sample_rate = 16000;
constexpr const char* revision = "97a15afa5caa9bce5baaa86c1184103877af4101";
FILE* protocol_output = nullptr;

uint32_t le32(const unsigned char* bytes) {
    return uint32_t(bytes[0]) | uint32_t(bytes[1]) << 8 | uint32_t(bytes[2]) << 16 | uint32_t(bytes[3]) << 24;
}
void read_exact(void* data, size_t size) {
    if (size && fread(data, 1, size, stdin) != size) throw std::runtime_error("truncated frame");
}
struct Frame { Json metadata; std::vector<float> pcm; };
Frame read_frame() {
    unsigned char header[8]; read_exact(header, sizeof(header));
    const auto json_size = le32(header), pcm_size = le32(header + 4);
    if (!json_size || json_size > max_json || pcm_size > max_pcm * 4 || pcm_size % 4) throw std::runtime_error("invalid frame size");
    std::string metadata(json_size, '\0'); read_exact(metadata.data(), metadata.size());
    std::vector<unsigned char> raw(pcm_size); read_exact(raw.data(), raw.size());
    Frame frame{Json::parse(metadata), std::vector<float>(pcm_size / 4)};
    for (size_t i = 0; i < frame.pcm.size(); ++i) {
        auto bits = le32(raw.data() + i * 4); memcpy(&frame.pcm[i], &bits, 4);
        if (!std::isfinite(frame.pcm[i])) throw std::runtime_error("non-finite PCM");
    }
    return frame;
}
void write_json(const Json& metadata) {
    auto data = metadata.dump();
    if (data.size() > max_json) throw std::runtime_error("result frame too large");
    unsigned char header[8]{};
    for (int i = 0; i < 4; ++i) header[i] = (data.size() >> (i * 8)) & 255;
    if (fwrite(header, 1, 8, protocol_output) != 8 || fwrite(data.data(), 1, data.size(), protocol_output) != data.size() || fflush(protocol_output)) throw std::runtime_error("output pipe closed");
}
void check(nemo_speech_asr_status status) {
    if (status != NEMO_SPEECH_ASR_OK) throw std::runtime_error(nemo_speech_asr_last_error());
}
void watch_parent(unsigned long parent) {
#ifdef _WIN32
    HANDLE handle = OpenProcess(SYNCHRONIZE, FALSE, parent);
    if (!handle) std::_Exit(72);
    std::thread([handle] { WaitForSingleObject(handle, INFINITE); std::_Exit(72); }).detach();
#else
    if (getppid() != static_cast<pid_t>(parent)) std::_Exit(72);
    std::thread([parent] { for (;;) { if (getppid() != static_cast<pid_t>(parent)) std::_Exit(72); std::this_thread::sleep_for(std::chrono::milliseconds(100)); } }).detach();
#endif
}
struct DiarModelDelete { void operator()(nemo_speech_diar_model* p) const { nemo_speech_diar_destroy(p); } };
struct DiarStreamDelete { void operator()(nemo_speech_diar_stream* p) const { nemo_speech_diar_stream_close(p); } };
struct RecognizerDelete { void operator()(nemo_speech_asr_recognizer* p) const { nemo_speech_asr_destroy(p); } };
struct ResultDelete { void operator()(nemo_speech_asr_result* p) const { nemo_speech_asr_result_destroy(p); } };
struct Pending { uint64_t id; uint64_t first_word = 0; asr::Result result; };
class Session {
    uint64_t generation = 0, epoch = 0, seq = 0, next_seq = 0, start = 0, accepted = 0, budget = 0, result_id = 0, update = 0;
    uint64_t offline_start = 0;
    int64_t last_segment_frames = -1;
    std::string route, language;
    bool started = false, finished = false;
    std::unique_ptr<nemo_speech_diar_model, DiarModelDelete> diar_model;
    std::unique_ptr<nemo_speech_diar_stream, DiarStreamDelete> diar;
    std::unique_ptr<nemo_speech_asr_recognizer, RecognizerDelete> offline;
    std::unique_ptr<asr::Recognizer> recognizer;
    std::unique_ptr<asr::RecognitionStream> stream;
    std::vector<Pending> pending;
    std::vector<float> offline_audio;
    uint64_t sample(double seconds) const {
        if (!std::isfinite(seconds)) return start + accepted;
        return start + std::min(accepted, static_cast<uint64_t>(std::max(0.0, std::round(seconds * sample_rate))));
    }
    void emit(Json value) {
        value["version"] = 1; value["generation"] = generation; value["epoch"] = epoch; value["seq"] = seq;
        write_json(value);
    }
    std::string backend(int gpu) {
        if (gpu < 0) return "CPU";
        int index = 0;
        for (size_t i = 0; i < ggml_backend_dev_count(); ++i) {
            auto device = ggml_backend_dev_get(i); auto type = ggml_backend_dev_type(device);
            if (type == GGML_BACKEND_DEVICE_TYPE_GPU || type == GGML_BACKEND_DEVICE_TYPE_IGPU) {
                if (index++ == gpu) return ggml_backend_dev_name(device);
            }
        }
        throw std::runtime_error("selected GPU is unavailable");
    }
    void open(const Json& m) {
        route = m.at("route").get<std::string>(); language = m.value("language", "");
        start = m.at("epoch_start").get<uint64_t>(); budget = m.at("max_epoch_samples").get<uint64_t>();
        if (!budget || budget > sample_rate * 60 * 60 || start > UINT64_MAX - budget) throw std::runtime_error("epoch budget must be at most one hour");
        int gpu = m.at("gpu").get<int>(); if (gpu < -1) throw std::runtime_error("invalid GPU index");
        std::string speaker_path = m.at("speaker_model").get<std::string>();
        std::string preset = m.value("diar_preset", "");
        emit({{"event", "hello"}, {"runtime_revision", revision}, {"routes", {"separate", "combined_stream", "combined_offline"}}, {"sample_rate", sample_rate}});
        if (route == "combined_stream") {
            asr::RecognizerConfig cfg;
            cfg.model.path = m.at("asr_model").get<std::string>(); cfg.backend.gpu = gpu;
            cfg.diar.model_path = speaker_path; cfg.diar.preset = preset; cfg.log_status = false;
            recognizer = std::make_unique<asr::Recognizer>(cfg);
            asr::AsrRequestOptions options;
            options.enable_word_time_offsets = true; options.enable_speaker_diarization = true; options.verbatim_transcripts = true;
            stream = recognizer->streaming_recognize(options, language); stream->set_interim_words(true);
        } else if (route == "separate" || route == "combined_offline") {
            nemo_speech_diar_model_config cfg{}; cfg.size = sizeof(cfg); cfg.model_path = speaker_path.c_str(); cfg.gpu = gpu; cfg.preset = preset.c_str(); cfg.left_context_frames = -1;
            nemo_speech_diar_model* model = nullptr; check(nemo_speech_diar_create(&cfg, &model)); diar_model.reset(model);
            nemo_speech_diar_stream* handle = nullptr; check(nemo_speech_diar_stream_open(model, &handle)); diar.reset(handle);
            if (route == "combined_offline") {
                auto path = m.at("asr_model").get<std::string>();
                nemo_speech_asr_backend_config b{sizeof(b), gpu};
                nemo_speech_asr_model_config model_cfg{sizeof(model_cfg), path.c_str(), nullptr};
                nemo_speech_asr_recognizer_config asr_cfg{}; asr_cfg.size = sizeof(asr_cfg); asr_cfg.backend = &b; asr_cfg.model = &model_cfg;
                nemo_speech_asr_recognizer* handle = nullptr; check(nemo_speech_asr_create(&asr_cfg, &handle)); offline.reset(handle);
            }
        } else throw std::runtime_error("unsupported route");
        started = true;
        emit({{"event", "ready"}, {"backend", backend(gpu)}, {"epoch_start", start}, {"max_epoch_samples", budget}});
    }
    void emit_words(Pending& item, bool final) {
        stream->refresh_speaker_tags(item.result);
        const double stable = final ? std::numeric_limits<double>::infinity() : stream->stable_speaker_time();
        if (item.result.alternatives.empty()) return;
        auto& words = item.result.alternatives.front().words;
        Json batch = Json::array();
        size_t stable_count = 0;
        for (size_t i = 0; i < words.size(); ++i) {
            const auto& word = words[i];
            const bool stable_word = final || word.end_time / 1000.0 < stable;
            if (stable_word && i == stable_count) ++stable_count;
            batch.push_back({{"index", item.first_word + i}, {"text", word.word}, {"start", sample(word.start_time / 1000.0)}, {"end", sample(word.end_time / 1000.0)}, {"speaker", word.speaker_tag}, {"speaker_final", stable_word}});
            if (batch.size() == 256 || i + 1 == words.size()) {
                emit({{"event", "words"}, {"result_id", item.id}, {"revision", ++update}, {"text_final", true}, {"words", batch}}); batch.clear();
            }
        }
        words.erase(words.begin(), words.begin() + stable_count); item.first_word += stable_count;
    }
    void refresh(bool final = false) {
        for (auto& item : pending) emit_words(item, final);
        pending.erase(std::remove_if(pending.begin(), pending.end(), [](const Pending& item) { return item.result.alternatives.empty() || item.result.alternatives.front().words.empty(); }), pending.end());
    }
    void result(asr::Result value) {
        if (!value.late_punctuation.empty()) emit({{"event", "punctuation"}, {"result_id", result_id}, {"revision", ++update}, {"text", value.late_punctuation}});
        if (value.alternatives.empty() || skip_empty_final(value)) return;
        if (!value.is_final) {
            Json words = Json::array();
            for (const auto& word : value.alternatives.front().words) words.push_back({{"text", word.word}, {"start", sample(word.start_time / 1000.0)}, {"end", sample(word.end_time / 1000.0)}, {"speaker", word.speaker_tag}});
            emit({{"event", "interim"}, {"result_id", result_id + 1}, {"revision", ++update}, {"text", value.alternatives.front().transcript}, {"text_final", false}, {"speaker_final", false}, {"words", words}});
        } else {
            ++result_id;
            emit({{"event", "text"}, {"result_id", result_id}, {"revision", ++update}, {"text", value.alternatives.front().transcript}, {"text_final", true}});
            value.alternatives.front().transcript.clear();
            pending.push_back({result_id, 0, std::move(value)});
        }
        refresh();
    }
    void segments(bool final) {
        const auto frames = nemo_speech_diar_frame_count(diar.get());
        if (!final && frames == last_segment_frames) return;
        last_segment_frames = frames;
        size_t count = 0; check(nemo_speech_diar_segments(diar.get(), nullptr, nullptr, 0, &count));
        std::vector<nemo_speech_diar_segment> values(count);
        check(nemo_speech_diar_segments(diar.get(), nullptr, values.data(), count, &count));
        const auto base = nemo_speech_diar_frame_probs_start(diar.get());
        const double stable = base * nemo_speech_diar_seconds_per_frame(diar_model.get());
        const uint64_t snapshot = ++update;
        emit({{"event", "segments_begin"}, {"revision", snapshot}, {"replace_from", start}, {"accepted_end", start + accepted}, {"speaker_final", final}, {"retained_probability_start", sample(stable)}});
        Json batch = Json::array();
        for (size_t i = 0; i < count; ++i) {
            const auto& value = values[i]; auto begin = sample(value.start_time), end = sample(value.end_time);
            if (end > begin) batch.push_back({{"start", begin}, {"end", end}, {"speaker", value.speaker}, {"speaker_final", final || value.end_time < stable}});
            if (batch.size() == 256 || i + 1 == count) { emit({{"event", "segments"}, {"revision", snapshot}, {"segments", batch}}); batch.clear(); }
        }
        emit({{"event", "segments_end"}, {"revision", snapshot}});
    }
    void offline_chunk() {
        if (offline_audio.empty()) return;
        auto opts = nemo_speech_asr_recognition_options_default(); opts.enable_word_time_offsets = true; opts.verbatim_transcripts = true; opts.language_code = language.c_str();
        nemo_speech_asr_result* handle = nullptr; check(nemo_speech_asr_recognize_f32(offline.get(), &opts, offline_audio.data(), offline_audio.size(), sample_rate, &handle));
        std::unique_ptr<nemo_speech_asr_result, ResultDelete> value(handle);
        ++result_id;
        emit({{"event", "text"}, {"result_id", result_id}, {"revision", ++update}, {"text", nemo_speech_asr_result_transcript(handle, 0)}, {"text_final", true}});
        Json words = Json::array();
        size_t count = nemo_speech_asr_result_word_count(handle, 0);
        auto offset = [&](int32_t ms) { return start + offline_start + std::min<uint64_t>(offline_audio.size(), static_cast<uint64_t>(std::max(0, ms)) * 16); };
        for (size_t i = 0; i < count; ++i) {
            words.push_back({{"index", i}, {"text", nemo_speech_asr_result_word_text(handle, 0, i)}, {"start", offset(nemo_speech_asr_result_word_start_time(handle, 0, i))}, {"end", offset(nemo_speech_asr_result_word_end_time(handle, 0, i))}, {"speaker", 0}, {"speaker_final", false}});
            if (words.size() == 256 || i + 1 == count) { emit({{"event", "words"}, {"result_id", result_id}, {"revision", ++update}, {"text_final", true}, {"speaker_source", "timeline"}, {"words", words}}); words.clear(); }
        }
        offline_start += offline_audio.size(); offline_audio.clear();
    }
public:
    void run() {
        for (;;) {
            auto frame = read_frame(); const auto& m = frame.metadata;
            if (m.at("version") != 1) throw std::runtime_error("protocol version mismatch");
            if (!started) { generation = m.at("generation").get<uint64_t>(); epoch = m.at("epoch").get<uint64_t>(); }
            if (m.at("generation") != generation || m.at("epoch") != epoch) throw std::runtime_error("stale generation or epoch");
            seq = m.at("seq").get<uint64_t>(); if (seq != next_seq++) throw std::runtime_error("request sequence mismatch");
            auto command = m.at("command").get<std::string>();
            if (command == "start" && !started && frame.pcm.empty()) { open(m); continue; }
            if (!started || finished) throw std::runtime_error("invalid session state");
            if (command == "push") {
                if (m.at("offset") != start + accepted || frame.pcm.empty() || frame.pcm.size() > budget - accepted) throw std::runtime_error("noncontiguous input or epoch budget exceeded");
                // Publish old stable words before the next native call can compact their probabilities.
                if (stream) refresh();
                accepted += frame.pcm.size();
                if (stream) {
                    stream->push(frame.pcm.data(), frame.pcm.size(), sample_rate);
                    while (auto value = stream->next()) { bool final = value->is_final; result(std::move(*value)); if (!final) break; }
                    refresh();
                } else {
                    check(nemo_speech_diar_stream_push_f32(diar.get(), frame.pcm.data(), frame.pcm.size(), sample_rate));
                    if (offline) { offline_audio.insert(offline_audio.end(), frame.pcm.begin(), frame.pcm.end()); if (offline_audio.size() >= sample_rate * 30) offline_chunk(); }
                    segments(false);
                }
                emit({{"event", "ack"}, {"accepted_end", start + accepted}});
            } else if (command == "finish" && frame.pcm.empty()) {
                if (stream) { refresh(); result(stream->finish()); refresh(true); }
                else { if (offline) offline_chunk(); check(nemo_speech_diar_stream_finish(diar.get())); segments(true); }
                finished = true;
                emit({{"event", "finished"}, {"coverage_start", start}, {"coverage_end", start + accepted}, {"text_final", true}, {"speaker_final", true}});
                return;
            } else throw std::runtime_error("unknown command or unexpected PCM");
        }
    }
    void error(const char* message) { emit({{"event", "error"}, {"code", "native_failure"}, {"message", message}, {"accepted_end", start + accepted}, {"complete", false}}); }
};
int main(int argc, char** argv) {
    if (argc != 3 || std::string(argv[1]) != "--parent") return 64;
    watch_parent(std::stoul(argv[2]));
#ifdef _WIN32
    _setmode(_fileno(stdin), _O_BINARY); _setmode(_fileno(stdout), _O_BINARY);
    protocol_output = _fdopen(_dup(_fileno(stdout)), "wb"); _dup2(_fileno(stderr), _fileno(stdout));
#else
    protocol_output = fdopen(dup(fileno(stdout)), "wb"); dup2(fileno(stderr), fileno(stdout));
#endif
    if (!protocol_output) return 74;
    Session session;
    try { session.run(); return 0; }
    catch (const std::exception& error) { try { session.error(error.what()); } catch (...) {} return 70; }
}
