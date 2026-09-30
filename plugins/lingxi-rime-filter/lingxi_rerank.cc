// LingXi rerank filter for librime.
//
// This RIME filter plugin sends the engine's top-K candidates to the LingXi
// IPC server (TCP 127.0.0.1:9527) for AI-based reranking, then replaces the
// candidate list with the reranked result.
//
// Build: compile as a shared library and place in RIME's plugin directory.
//   - On Windows (Weasel): build as lingxi_filter.dll
//   - Link against librime headers (no lib needed for a filter plugin).
//
// RIME configuration (e.g. in default.custom.yaml or schema.yaml):
//   engine/filters:
//     - lingxi_rerank

#include <rime/filter.h>
#include <rime/candidate.h>
#include <rime/translation.h>
#include <rime/gear/filter_commons.h>

#include <string>
#include <vector>
#include <sstream>

// Minimal TCP client — connects to localhost:9527, sends one line, reads one
// line. No external dependencies (uses WinSock on Windows, POSIX sockets on
// *nix). Returns empty string on failure.
#ifdef _WIN32
#include <winsock2.h>
#include <ws2tcpip.h>
#pragma comment(lib, "ws2_32.lib")
#else
#include <sys/socket.h>
#include <netinet/in.h>
#include <arpa/inet.h>
#include <unistd.h>
#define closesocket close
typedef int SOCKET;
#define INVALID_SOCKET (-1)
#endif

namespace {

std::string ipc_request(const std::string& json_line) {
#ifdef _WIN32
  static bool wsa_init = false;
  if (!wsa_init) {
    WSADATA wsa;
    WSAStartup(MAKEWORD(2, 2), &wsa);
    wsa_init = true;
  }
#endif

  SOCKET sock = socket(AF_INET, SOCK_STREAM, 0);
  if (sock == INVALID_SOCKET) return "";

  sockaddr_in addr{};
  addr.sin_family = AF_INET;
  addr.sin_port = htons(9527);
  inet_pton(AF_INET, "127.0.0.1", &addr.sin_addr);

  if (connect(sock, (sockaddr*)&addr, sizeof(addr)) != 0) {
    closesocket(sock);
    return "";
  }

  std::string payload = json_line + "\n";
  send(sock, payload.c_str(), (int)payload.size(), 0);

  // Shutdown write side to signal end of request.
  shutdown(sock, 1); // SD_SEND

  std::string response;
  char buf[4096];
  int n;
  while ((n = recv(sock, buf, sizeof(buf) - 1, 0)) > 0) {
    buf[n] = '\0';
    response += buf;
  }
  closesocket(sock);
  return response;
}

// Minimal JSON construction (avoid pulling a JSON library into a RIME plugin).
std::string escape_json(const std::string& s) {
  std::string out;
  for (char c : s) {
    switch (c) {
      case '"': out += "\\\""; break;
      case '\\': out += "\\\\"; break;
      case '\n': out += "\\n"; break;
      default: out += c;
    }
  }
  return out;
}

std::string build_rerank_request(const std::vector<std::string>& candidates,
                                  const std::string& context) {
  std::ostringstream oss;
  oss << R"({"type":"rerank","candidates":[)";
  for (size_t i = 0; i < candidates.size(); ++i) {
    if (i > 0) oss << ",";
    oss << "\"" << escape_json(candidates[i]) << "\"";
  }
  oss << R"(],"context":")" << escape_json(context) << R"(","limit":9})";
  return oss.str();
}

// Minimal JSON array-of-objects parse: extract "text" fields in order.
// Expected: {"candidates":[{"text":"...","score":...}, ...]}
std::vector<std::string> parse_response_texts(const std::string& json) {
  std::vector<std::string> out;
  size_t pos = 0;
  while (true) {
    pos = json.find("\"text\"", pos);
    if (pos == std::string::npos) break;
    pos = json.find(':', pos);
    if (pos == std::string::npos) break;
    pos = json.find('"', pos + 1);
    if (pos == std::string::npos) break;
    ++pos; // skip opening quote
    std::string text;
    while (pos < json.size() && json[pos] != '"') {
      if (json[pos] == '\\' && pos + 1 < json.size()) {
        ++pos;
        switch (json[pos]) {
          case 'n': text += '\n'; break;
          case '"': text += '"'; break;
          case '\\': text += '\\'; break;
          default: text += json[pos];
        }
      } else {
        text += json[pos];
      }
      ++pos;
    }
    out.push_back(text);
    ++pos; // skip closing quote
  }
  return out;
}

} // anonymous namespace

namespace rime {

// The filter class. RIME instantiates it per schema where it's listed.
class LingxiRerankFilter : public Filter {
 public:
  explicit LingxiRerankFilter(const Ticket& ticket) : Filter(ticket) {}

  // Called by RIME's engine after translators produce candidates. We collect
  // the top-K, send to the Rust server for reranking, then yield them in the
  // new order. Candidates beyond top-K pass through unchanged.
  an<Translation> Apply(an<Translation> translation,
                        CandidateList* recruited) override {
    const int TOP_K = 9;
    std::vector<an<Candidate>> original;
    std::vector<std::string> texts;

    // Collect top-K candidates.
    while (original.size() < TOP_K && !translation->exhausted()) {
      auto cand = translation->Peek();
      if (!cand) break;
      texts.push_back(cand->text());
      original.push_back(cand);
      translation->Next();
    }

    if (texts.empty()) return translation;

    // Get preceding committed text for context (RIME exposes this via the
    // composition's committed string in some configurations; for now use "").
    std::string context;

    // IPC call to LingXi Rust server.
    std::string request = build_rerank_request(texts, context);
    std::string response = ipc_request(request);
    std::vector<std::string> reranked = parse_response_texts(response);

    // If IPC failed or returned empty, fall back to original order.
    if (reranked.empty()) {
      // Re-yield originals as-is.
      auto result = New<FifoTranslation>();
      for (auto& c : original) result->Append(c);
      // Append the rest of the translation.
      while (!translation->exhausted()) {
        auto cand = translation->Peek();
        if (cand) result->Append(cand);
        translation->Next();
      }
      return result;
    }

    // Build reordered list: for each text in `reranked`, find the matching
    // original candidate. Unmatched reranked texts are skipped; unmatched
    // originals are appended at the end.
    auto result = New<FifoTranslation>();
    std::vector<bool> used(original.size(), false);
    for (auto& text : reranked) {
      for (size_t i = 0; i < original.size(); ++i) {
        if (!used[i] && original[i]->text() == text) {
          result->Append(original[i]);
          used[i] = true;
          break;
        }
      }
    }
    // Append remaining originals not in reranked list.
    for (size_t i = 0; i < original.size(); ++i) {
      if (!used[i]) result->Append(original[i]);
    }
    // Append the rest of the stream.
    while (!translation->exhausted()) {
      auto cand = translation->Peek();
      if (cand) result->Append(cand);
      translation->Next();
    }
    return result;
  }
};

// Register this filter with RIME's component registry.
// The string "lingxi_rerank" is used in schema YAML: `engine/filters`.

} // namespace rime

// RIME plugin entry point — called when the plugin DLL is loaded.
// Registers the filter component so RIME schemas can reference it.
extern "C" {
#ifdef _WIN32
__declspec(dllexport)
#endif
void rime_lingxi_initialize() {
  // Registration depends on the RIME API version; the exact call differs
  // between librime 1.x plugin API styles. A typical registration:
  //   rime::Registry::instance().Register("lingxi_rerank",
  //       new rime::Component<rime::LingxiRerankFilter>);
  //
  // Adjust to match the librime version bundled with your Weasel build.
}
}
