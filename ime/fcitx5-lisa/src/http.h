// Minimal loopback OpenAI-compat client for fcitx5-lisa
// (docs/PLAN.md §5.7.3 layer 2, ADR-0007).
//
// Deliberately dependency-free (plain POSIX sockets, no libcurl/TLS):
// it only ever talks to lisa-inferenced on 127.0.0.1 (§5.1's
// OpenAI-compat endpoint, the zero-dependency path per §5.6). Pure
// standard C++/POSIX so the protocol half compiles and unit-tests on
// any dev host; the fcitx5 glue (lisa.cpp) needs Linux.

#pragma once

#include <string>

namespace lisa {

// JSON string escaping for request bodies (RFC 8259 minimal set).
std::string jsonEscape(const std::string &text);

// Inverse of the encoder for the single string field we read back.
std::string jsonUnescape(const std::string &text);

// Extract choices[0].message.content from a chat-completions response
// body (non-streaming). Tolerates surrounding HTTP noise (headers,
// chunked framing) by scanning for the field; returns "" on failure.
std::string extractContent(const std::string &payload);

// Build the request body for a writing-tools action.
std::string chatRequestBody(const std::string &systemPrompt,
                            const std::string &userText);

// POST `body` to http://host:port/v1/chat/completions and return the
// raw response payload ("" on any failure). Blocking; call off the UI
// thread. `timeoutSeconds` bounds connect/send/receive.
std::string postChatCompletions(const std::string &host, int port,
                                const std::string &body,
                                int timeoutSeconds = 30);

} // namespace lisa
