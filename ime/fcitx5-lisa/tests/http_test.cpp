// Unit tests for fcitx5-lisa's protocol half (PLAN §5.7.3, ADR-0007).
// Pure standard C++/POSIX — compiles and runs on any dev host:
//   c++ -std=c++17 -I../src http_test.cpp ../src/http.cpp && ./a.out
// (wired up via tests/CMakeLists.txt and `just ime-test`).

#include "http.h"

#include <cstdio>
#include <string>

static int failures = 0;

static void expectEq(const std::string &actual, const std::string &expected,
                     const char *name) {
    if (actual == expected) {
        std::printf("  ok    %s\n", name);
    } else {
        std::printf("  FAIL  %s: expected [%s], got [%s]\n", name,
                    expected.c_str(), actual.c_str());
        ++failures;
    }
}

int main() {
    using namespace lisa;

    expectEq(jsonEscape("plain"), "plain", "escape passthrough");
    expectEq(jsonEscape("a\"b\\c\nd\te"), "a\\\"b\\\\c\\nd\\te",
             "escape specials");
    expectEq(jsonEscape(std::string(1, '\x01')), "\\u0001",
             "escape control chars");
    expectEq(jsonUnescape("a\\\"b\\\\c\\nd\\te"), "a\"b\\c\nd\te",
             "unescape inverts escape");
    expectEq(jsonUnescape("caf\\u00e9"), "café", "unescape \\u BMP");

    expectEq(extractContent(
                 R"({"choices":[{"message":{"role":"assistant",)"
                 R"("content":"Fixed text."},"finish_reason":"stop"}]})"),
             "Fixed text.", "extract content from chat response");
    expectEq(extractContent(
                 "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\r\n"
                 R"({"choices":[{"message":{"content":"A \"quoted\" fix\n"}}]})"),
             "A \"quoted\" fix\n", "extract rides over HTTP headers");
    expectEq(extractContent(R"({"error":{"message":"boom"}})"), "",
             "no content field -> empty");
    expectEq(extractContent(R"({"content": "spaced"})"), "spaced",
             "tolerates space after colon");
    expectEq(extractContent(""), "", "empty payload -> empty");

    const std::string body = chatRequestBody("sys \"p\"", "user\ntext");
    expectEq(body,
             "{\"stream\":false,\"temperature\":0.2,\"messages\":["
             "{\"role\":\"system\",\"content\":\"sys \\\"p\\\"\"},"
             "{\"role\":\"user\",\"content\":\"user\\ntext\"}]}",
             "request body shape");
    // The body must itself round-trip through the extractor's escaping
    // rules: what we send is what the daemon sees.
    expectEq(extractContent("{\"content\":\"" + jsonEscape("user\ntext") +
                            "\"}"),
             "user\ntext", "escape/extract round-trip");

    // Network path: refused connection fails soft (empty), never throws.
    expectEq(postChatCompletions("127.0.0.1", 1, "{}", 1), "",
             "connection refused -> empty");
    expectEq(postChatCompletions("not-an-ip", 7777, "{}", 1), "",
             "bad host -> empty");

    std::printf("fcitx5-lisa http: %s (%d failure(s))\n",
                failures == 0 ? "all passed" : "FAILED", failures);
    return failures == 0 ? 0 : 1;
}
