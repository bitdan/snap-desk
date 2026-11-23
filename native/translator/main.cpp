// 原生常驻翻译进程。stdout 专用于 JSONL；异常只返回固定错误码，避免泄漏原文或路径。
#include <ctranslate2/translator.h>
#include <sentencepiece_processor.h>
#include <nlohmann/json.hpp>
#include <algorithm>
#include <fstream>
#include <iostream>
#include <memory>
#include <regex>
#include <set>
#include <stdexcept>
#include <string>
#include <vector>
#include <filesystem>
#ifdef _WIN32
#include <windows.h>
#endif
using json = nlohmann::json;

struct Engine {
    std::unique_ptr<ctranslate2::Translator> translator;
    sentencepiece::SentencePieceProcessor source, target;
    std::string language_token;
    std::set<std::string> vocabulary;

    void load(const json& request) {
        translator.reset(); // LRU(1)：切换方向先释放旧模型，控制峰值内存。
        const auto directory = std::filesystem::u8path(request.at("directory").get<std::string>());
        std::ifstream stream(directory / "pack.json");
        const auto pack = json::parse(stream);
        const auto id = request.at("modelId").get<std::string>();
        if (pack.at("modelId") != id || (id != "opus-mt-en-zh-int8" && id != "opus-mt-zh-en-int8")
            || pack.at("runtime") != "4.6.0" || pack.at("tokenizerProfile") != "transformers-marian-spm-v1")
            throw std::runtime_error("invalid_pack");
        language_token = pack.at("targetLanguageToken");
        if (language_token != (id == "opus-mt-en-zh-int8" ? ">>cmn_Hans<<" : "")) throw std::runtime_error("invalid_pack");
        // 二进制读取支持 Windows 中文路径；SPM 的窄字符串文件 API 不保证此行为。
        auto load_spm = [&](const char* name, sentencepiece::SentencePieceProcessor& spm) {
            std::ifstream file(directory / name, std::ios::binary);
            std::string bytes((std::istreambuf_iterator<char>(file)), {});
            if (bytes.empty() || !spm.LoadFromSerializedProto(bytes).ok()) throw std::runtime_error("invalid_pack");
        };
        load_spm("source.spm", source);
        load_spm("target.spm", target);
        auto vocabulary_path = directory / "shared_vocabulary.json";
        if (!std::filesystem::exists(vocabulary_path)) vocabulary_path = directory / "source_vocabulary.json";
        std::ifstream vocabulary_file(vocabulary_path);
        auto words = json::parse(vocabulary_file).get<std::vector<std::string>>();
        vocabulary = std::set<std::string>(words.begin(), words.end());
        if (!language_token.empty() && !vocabulary.count(language_token)) throw std::runtime_error("invalid_pack");
        // 模型目录通过 current_path 使用宽字符路径设置，CT2 只读取 ASCII 相对文件名。
        std::filesystem::current_path(directory);
        ctranslate2::models::ModelLoader loader(".");
        loader.device = ctranslate2::Device::CPU;
        loader.compute_type = ctranslate2::ComputeType::INT8;
        ctranslate2::ReplicaPoolConfig pool;
        pool.num_threads_per_replica = 2;
        translator = std::make_unique<ctranslate2::Translator>(loader, pool);
    }

    std::vector<std::string> encode(const std::string& text) {
        std::vector<std::string> tokens;
        if (!source.Encode(text, &tokens).ok()) throw std::runtime_error("invalid_text");
        // HF encode 会经 Marian 词表映射未知片段；直接传 SPM 的未知原字符会破坏一致性。
        for (auto& token : tokens) if (!vocabulary.count(token)) token = "<unk>";
        if (!language_token.empty()) tokens.insert(tokens.begin(), language_token);
        // 仅接受 Transformers 转换的包。此路线 CT2 不自动附加源 EOS，必须只加一次。
        tokens.push_back("</s>");
        if (tokens.size() > 480) throw std::runtime_error("input_too_long");
        return tokens;
    }
    std::string decode(std::vector<std::string> tokens) {
        tokens.erase(std::remove_if(tokens.begin(), tokens.end(), [&](const auto& t) {
            return t == "</s>" || t == "<pad>" || t == "<s>" || t == "<unk>" || t == "<eop>" || t == "<eod>" || (!language_token.empty() && t == language_token);
        }), tokens.end());
        std::string text;
        if (!target.Decode(tokens, &text).ok()) throw std::runtime_error("invalid_output");
        return text;
    }
    std::string translate_plain(const std::string& text) {
        if (text.find_first_not_of(" \t\r") == std::string::npos) return text;
        ctranslate2::TranslationOptions options;
        options.beam_size = 2;
        options.max_decoding_length = 512;
        options.max_input_length = 0; // 禁止静默截断，encode 已显式检查。
        auto outputs = translator->translate_batch({encode(text)}, options);
        if (outputs.size() != 1 || outputs[0].output().size() >= 512) throw std::runtime_error("output_too_long");
        const auto start = text.find_first_not_of(" \t\r");
        const auto end = text.find_last_not_of(" \t\r");
        return text.substr(0, start) + decode(outputs[0].output()) + text.substr(end + 1);
    }
    // 将路径、URL、占位符视为不可翻译片段；直接拼回，避免模型改写占位符造成语义损坏。
    std::string translate(const std::string& text) {
        static const std::regex protected_text(R"((https?://[^\s]+|[A-Za-z]:[\\/][^\s]+|\{\{?[^{}\r\n]+\}?\}|%[0-9$]*[sdif]|(?:Ctrl|Alt|Shift)(?:\+[A-Za-z0-9]+)+|`[^`\r\n]+`))");
        std::string output;
        size_t position = 0;
        for (std::sregex_iterator it(text.begin(), text.end(), protected_text), end; it != end; ++it) {
            auto start = static_cast<size_t>(it->position());
            output += translate_plain(text.substr(position, start - position));
            output += it->str();
            position = start + it->length();
        }
        return output + translate_plain(text.substr(position));
    }
};

int main() {
#ifdef _WIN32
    SetErrorMode(SEM_FAILCRITICALERRORS | SEM_NOGPFAULTERRORBOX);
#endif
    Engine engine;
    // 有界逐字节读取，避免 getline 在检测长度之前分配任意大的输入。
    for (;;) {
        std::string line;
        char ch;
        while (std::cin.get(ch) && ch != '\n') {
            if (line.size() >= 1024 * 1024) return 2;
            line.push_back(ch);
        }
        if (line.empty() && !std::cin) break;
        json response = {{"protocol", 1}, {"id", ""}, {"ok", false}};
        try {
            auto request = json::parse(line);
            response["id"] = request.at("id").get<std::string>();
            if (request.at("protocol") != 1 || response["id"].get<std::string>().size() > 80) throw std::runtime_error("invalid_request");
            auto action = request.at("action").get<std::string>();
            if (action == "health") response["runtime"] = "4.6.0";
            else if (action == "load") engine.load(request);
            else if (action == "translate" || action == "tokenize" || action == "decode") {
                if (!engine.translator) throw std::runtime_error("not_loaded");
                if (action == "tokenize") response["tokens"] = engine.encode(request.at("text"));
                else if (action == "decode") response["text"] = engine.decode(request.at("tokens").get<std::vector<std::string>>());
                else {
                    auto segments = request.at("segments");
                    if (!segments.is_array() || segments.empty() || segments.size() > 64) throw std::runtime_error("invalid_request");
                    response["items"] = json::array();
                    std::set<std::string> ids;
                    size_t bytes = 0;
                    for (const auto& segment : segments) {
                        auto id = segment.at("id").get<std::string>();
                        auto text = segment.at("text").get<std::string>();
                        bytes += text.size();
                        if (id.size() > 80 || !ids.insert(id).second || bytes > 32000) throw std::runtime_error("invalid_request");
                        response["items"].push_back({{"id", id}, {"text", engine.translate(text)}});
                    }
                }
            } else if (action == "unload") engine.translator.reset();
            else if (action == "shutdown") return 0;
            else throw std::runtime_error("invalid_request");
            response["ok"] = true;
        } catch (const std::exception& error) {
            const std::string reason(error.what());
            response = {{"protocol", 1}, {"id", response["id"]}, {"ok", false}, {"error", reason == "input_too_long" || reason == "output_too_long" ? reason : "engine_error"}};
        }
        auto wire = response.dump();
        if (wire.size() >= 256 * 1024) return 3;
        std::cout << wire << '\n' << std::flush;
    }
}
