// Copyright 2026 mozrs contributors. BSD-3-Clause; see ../LICENSE.
// Calls the unmodified upstream builder and lookup implementation.
#include <cstdint>
#include <iostream>
#include <set>
#include <string>
#include <vector>
#include "absl/flags/flag.h"
#include "absl/flags/declare.h"
#include "absl/strings/string_view.h"
#include "dictionary/dictionary_interface.h"
#include "dictionary/dictionary_token.h"
#include "dictionary/text_dictionary_loader.h"
#include "dictionary/system/system_dictionary.h"
#include "dictionary/system/system_dictionary_builder.h"

ABSL_DECLARE_FLAG(int32_t, min_key_length_to_use_small_cost_encoding);
using mozc::dictionary::Token;
using mozc::dictionary::SystemDictionary;

std::string Json(absl::string_view value) {
  const char* hex = "0123456789abcdef";
  std::string out = "\"";
  for (unsigned char c : value) {
    if (c == '"' || c == '\\') { out += '\\'; out += c; }
    else if (c < 32) { out += "\\u00"; out += hex[c >> 4]; out += hex[c & 15]; }
    else { out += c; }
  }
  return out + '"';
}

class Collect : public mozc::dictionary::DictionaryInterface::Callback {
 public:
  std::vector<Token> tokens;
  ResultType OnToken(absl::string_view, absl::string_view, Token token) override {
    tokens.push_back(std::move(token));
    return TRAVERSE_CONTINUE;
  }
};

void Print(const SystemDictionary& dict, const std::string& mode,
           const std::string& query) {
  Collect callback;
  if (mode == "exact") dict.LookupExact(query, &callback);
  else dict.LookupPrefix(query, &callback);
  std::cout << "{\"query\":" << Json(query) << ",\"mode\":" << Json(mode)
            << ",\"tokens\":[";
  bool first = true;
  for (const auto& t : callback.tokens) {
    if (!first) std::cout << ',';
    first = false;
    std::cout << "{\"key\":" << Json(t.key) << ",\"value\":" << Json(t.value)
              << ",\"cost\":" << t.cost << ",\"lid\":" << t.lid
              << ",\"rid\":" << t.rid << ",\"attributes\":" << int(t.attributes) << '}';
  }
  std::cout << "]}\n";
}

int Fixture(const std::string& path, bool print_inputs = false) {
  absl::SetFlag(&FLAGS_min_key_length_to_use_small_cost_encoding, 1);
  std::vector<Token> tokens = {
      {"あ", "亜", 100, 50, 70, 0}, {"あ", "亜", 101, 51, 71, 0},
      {"あ", "あ", 200, 50, 70, 0}, {"あ", "ア", 300, 50, 70, 0},
      {"あい", "愛", 400, 50, 70, 0}, {"あい", "藍", 500, 50, 70, 0},
      {"あいう", "合いう", 600, 50, 70, 0},
      {"あぼがど", "アボカド", 700, 50, 70, Token::SPELLING_CORRECTION},
      {"う゛ぁ", "ヴァ", 800, 50, 70, 0},
      {"ゔゕゖゝゞ", "ヴゕゖゝゞ", 900, 50, 70, 0},
      {"かな", "かな", 50, 50, 70, 0}, {"かな", "カナ", 51, 50, 70, 0},
      {"がっこう", "学校", 777, 50, 70, 0}, {"かっこう", "格好", 888, 50, 70, 0},
      {"ながいよみ", "長い読み", 9999, 50, 70, 0},
      {"きごう", "龠😀𐀀𠀁𐐀　éᄀ", 32767, 4095, 4000, 0},
      {"ascii", "A\tB\n\"\\", 655, 4094, 4094, 0},
      {"ABC-123", "ascii result", 256, 50, 70, 0},
      {"ー・", "長音・中点", 1500, 50, 70, 0}, {"😀", "笑顔", 1, 50, 70, 0},
  };
  // >256 POS pairs exercise infrequent full/mono encoding. Two values per
  // pair exercise SAME_AS_PREV_POS without a second implementation of encoding.
  for (int i = 1; i <= 300; ++i) {
    for (int j = 0; j < 2; ++j) {
      tokens.emplace_back("pos" + std::to_string(i), "値" + std::to_string(j),
                          i * 23 + j, i, i % 2 ? i : i + 1, Token::NONE);
    }
  }
  if (print_inputs) {
    for (const auto& t : tokens) {
      std::cout << "{\"key\":" << Json(t.key) << ",\"value\":" << Json(t.value)
                << ",\"cost\":" << t.cost << ",\"lid\":" << t.lid
                << ",\"rid\":" << t.rid << ",\"attributes\":" << int(t.attributes) << "}\n";
    }
    return 0;
  }
  std::vector<Token*> pointers;
  std::set<std::string> queries = {"", "不存在", "あいうえお", "がつこう", "あぼがどです", "😀です"};
  for (auto& t : tokens) { pointers.push_back(&t); queries.insert(t.key); queries.insert(t.key + "です"); }
  mozc::dictionary::SystemDictionaryBuilder builder;
  builder.BuildFromTokens(pointers);
  builder.WriteToFile(path);
  auto dict = SystemDictionary::Builder(path).Build();
  if (!dict.ok()) { std::cerr << dict.status() << '\n'; return 1; }
  for (const auto& mode : {"exact", "prefix"}) {
    for (const auto& query : queries) Print(**dict, mode, query);
  }
  return 0;
}

int main(int argc, char** argv) {
  // Optional loader oracle: prepared TSV plus corrections, without dataset
  // packaging. Arguments are output, zipcode ID, isolated ID, corrections,
  // and one or more input files. '-' means no corrections.
  if (argc >= 7 && (std::string(argv[1]) == "build-tsv" || std::string(argv[1]) == "tsv-inputs")) {
    mozc::dictionary::TextDictionaryLoader loader(std::stoi(argv[3]), std::stoi(argv[4]));
    std::string files;
    for (int i = 6; i < argc; ++i) { if (i > 6) files += ','; files += argv[i]; }
    loader.Load(files, std::string(argv[5]) == "-" ? "" : argv[5]);
    if (std::string(argv[1]) == "tsv-inputs") {
      for (const auto& token : loader.tokens()) {
        const auto& t = *token;
        std::cout << "{\"key\":" << Json(t.key) << ",\"value\":" << Json(t.value)
                  << ",\"cost\":" << t.cost << ",\"lid\":" << t.lid
                  << ",\"rid\":" << t.rid << ",\"attributes\":" << int(t.attributes) << "}\n";
      }
    } else {
      mozc::dictionary::SystemDictionaryBuilder builder;
      builder.BuildFromTokens(loader.tokens());
      builder.WriteToFile(argv[2]);
    }
    return 0;
  }
  if (argc != 3) { std::cerr << "Usage: oracle fixture PATH | oracle PATH exact|prefix\n"; return 2; }
  if (std::string(argv[1]) == "fixture") return Fixture(argv[2]);
  if (std::string(argv[1]) == "fixture-inputs") return Fixture(argv[2], true);
  const std::string mode = argv[2];
  if (mode != "exact" && mode != "prefix") return 2;
  auto dict = SystemDictionary::Builder(argv[1]).Build();
  if (!dict.ok()) { std::cerr << dict.status() << '\n'; return 1; }
  std::string query;
  while (std::getline(std::cin, query)) Print(**dict, mode, query);
  return 0;
}
