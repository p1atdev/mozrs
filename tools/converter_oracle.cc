// Copyright 2026 mozrs contributors. BSD-3-Clause; see ../LICENSE.
// Optional reference: unchanged upstream Connector and private Viterbi exposed
// through its existing test peer. We supply the same deliberately basic nodes
// as Rust, bypassing MakeLattice/MakeSegments/NBest/rewriters and history.
#include <cstdint>
#include <fstream>
#include <iostream>
#include <iterator>
#include <memory>
#include <stdexcept>
#include <string>
#include <unordered_set>
#include <vector>
#include "absl/strings/string_view.h"
#include "converter/connector.h"
#include "converter/immutable_converter.h"
#include "converter/lattice.h"
#include "converter/node.h"
#include "converter/segments.h"
#include "data_manager/data_manager.h"
#include "dictionary/dictionary_interface.h"
#include "dictionary/dictionary_token.h"
#include "dictionary/system/system_dictionary.h"
#include "engine/modules.h"
#include "request/options.h"

namespace mozc {
class ImmutableConverterTestPeer {
 public:
  static bool Run(const ImmutableConverter& converter, Segments& segments, Lattice& lattice) {
    ConversionOptions options;
    options.particle_omission_transition_cost_bonus = 0;
    return converter.Viterbi(options, segments, &lattice);
  }
};
}

std::string Read(const char* path) {
  std::ifstream stream(path, std::ios::binary);
  if (!stream) throw std::runtime_error("cannot open input file");
  return std::string(std::istreambuf_iterator<char>(stream), {});
}
std::string Json(absl::string_view value) {
  const char* hex = "0123456789abcdef";
  std::string out = "\"";
  for (unsigned char c : value) {
    if (c == '"' || c == '\\') { out += '\\'; out += c; }
    else if (c < 32) { out += "\\u00"; out += hex[c >> 4]; out += hex[c & 15]; }
    else out += c;
  }
  return out + '"';
}

class Data : public mozc::DataManager {
 public:
  Data(const char* dataset, const char* connection) : connection_(Read(connection)) {
    const auto status = InitFromFile(dataset);
    if (!status.ok()) throw std::runtime_error(std::string(status.message()));
  }
  absl::string_view GetConnectorData() const override { return connection_; }
 private:
  std::string connection_;
};
class Collect : public mozc::dictionary::DictionaryInterface::Callback {
 public:
  std::vector<mozc::dictionary::Token> tokens;
  ResultType OnToken(absl::string_view, absl::string_view, mozc::dictionary::Token token) override {
    tokens.push_back(std::move(token));
    return TRAVERSE_CONTINUE;
  }
};

int Run(int argc, char** argv) {
  if (argc == 3 && std::string(argv[1]) == "connection") {
    const std::string image = Read(argv[2]);
    auto conn = mozc::Connector::Create(image);
    if (!conn.ok()) throw std::runtime_error(std::string(conn.status().message()));
    const uint16_t size = uint8_t(image[4]) | (uint16_t(uint8_t(image[5])) << 8);
    // Exhaustive row-major cost dump in explicit little-endian, no Rust decoder.
    for (uint16_t rid = 0; rid < size; ++rid) for (uint16_t lid = 0; lid < size; ++lid) {
      const int cost = conn->GetTransitionCost(rid, lid);
      std::cout.put(char(cost & 255)); std::cout.put(char((cost >> 8) & 255));
    }
    return 0;
  }
  if (argc != 6 || std::string(argv[1]) != "convert") {
    std::cerr << "Usage: oracle connection CONNECTION | oracle convert OSS_MOZC.data DICTIONARY CONNECTION UNKNOWN_ID\n";
    return 2;
  }
  auto modules = mozc::engine::Modules::Create(std::make_unique<Data>(argv[2], argv[4]));
  if (!modules.ok()) throw std::runtime_error(std::string(modules.status().message()));
  const mozc::ImmutableConverter converter(**modules);
  auto dictionary = mozc::dictionary::SystemDictionary::Builder(argv[3]).Build();
  if (!dictionary.ok()) throw std::runtime_error(std::string(dictionary.status().message()));
  const uint16_t unknown = std::stoi(argv[5]);
  const auto& connector = (*modules)->GetConnector();
  std::string query;
  while (std::getline(std::cin, query)) {
    if (query.size() > 4096 || query.find('\0') != std::string::npos) throw std::runtime_error("invalid query");
    if (query.empty()) {
      // API policy for empty queries; upstream Viterbi expects a nonempty key.
      std::cout << "{\"query\":\"\",\"value\":\"\",\"cost\":0,\"eos_cost\":0,\"tokens\":[]}\n";
      continue;
    }
    mozc::Lattice lattice;
    lattice.SetKey(query);
    std::unordered_set<const mozc::Node*> fallback;
    for (size_t pos = 0; pos < query.size();) {
      Collect collector;
      (*dictionary)->LookupPrefix(absl::string_view(query).substr(pos), &collector);
      for (auto& token : collector.tokens) {
        auto* node = lattice.NewNode();
        node->InitFromToken(std::move(token));
        lattice.Insert(pos, node);
      }
      const unsigned char lead = query[pos];
      const size_t length = lead < 0x80 ? 1 : lead < 0xe0 ? 2 : lead < 0xf0 ? 3 : 4;
      auto* node = lattice.NewNode();
      node->InitFromToken({query.substr(pos, length), query.substr(pos, length), 32767, unknown, unknown, 0});
      lattice.Insert(pos, node);
      fallback.insert(node);
      pos += length;
    }
    mozc::Segments segments;
    segments.add_segment()->set_key(query);
    if (!mozc::ImmutableConverterTestPeer::Run(converter, segments, lattice)) throw std::runtime_error("no path");
    std::string value;
    for (const auto* n = lattice.bos_node()->next; n != lattice.eos_node(); n = n->next) value += n->value;
    std::cout << "{\"query\":" << Json(query) << ",\"value\":" << Json(value)
              << ",\"cost\":" << lattice.eos_node()->cost
              << ",\"eos_cost\":" << connector.GetTransitionCost(lattice.eos_node()->prev->rid, 0)
              << ",\"tokens\":[";
    bool first = true;
    for (const auto* n = lattice.bos_node()->next; n != lattice.eos_node(); n = n->next) {
      if (!first) std::cout << ',';
      first = false;
      std::cout << "{\"start\":" << n->begin_pos << ",\"end\":" << n->end_pos
                << ",\"key\":" << Json(n->key) << ",\"value\":" << Json(n->value)
                << ",\"cost\":" << n->wcost << ",\"lid\":" << n->lid << ",\"rid\":" << n->rid
                << ",\"attributes\":" << ((n->attributes & mozc::Node::SPELLING_CORRECTION) ? 1 : 0)
                << ",\"unknown\":" << (fallback.contains(n) ? "true" : "false")
                << ",\"transition_cost\":" << connector.GetTransitionCost(n->prev->rid, n->lid) << '}';
    }
    std::cout << "]}\n";
  }
  return 0;
}
int main(int argc, char** argv) {
  try { return Run(argc, argv); }
  catch (const std::exception& e) { std::cerr << e.what() << '\n'; return 1; }
}
