// See http.h. Loopback-only OpenAI-compat client (PLAN §5.7.3 layer 2).

#include "http.h"

#include <arpa/inet.h>
#include <netinet/in.h>
#include <sys/socket.h>
#include <sys/time.h>
#include <unistd.h>

#include <cstdio>
#include <cstring>

namespace lisa {

std::string jsonEscape(const std::string &text) {
    std::string out;
    out.reserve(text.size() + 8);
    for (unsigned char c : text) {
        switch (c) {
        case '"': out += "\\\""; break;
        case '\\': out += "\\\\"; break;
        case '\n': out += "\\n"; break;
        case '\r': out += "\\r"; break;
        case '\t': out += "\\t"; break;
        default:
            if (c < 0x20) {
                char buf[8];
                std::snprintf(buf, sizeof(buf), "\\u%04x", c);
                out += buf;
            } else {
                out += static_cast<char>(c);
            }
        }
    }
    return out;
}

std::string jsonUnescape(const std::string &text) {
    std::string out;
    out.reserve(text.size());
    for (size_t i = 0; i < text.size(); ++i) {
        char c = text[i];
        if (c != '\\' || i + 1 >= text.size()) {
            out += c;
            continue;
        }
        char next = text[++i];
        switch (next) {
        case 'n': out += '\n'; break;
        case 'r': out += '\r'; break;
        case 't': out += '\t'; break;
        case '"': out += '"'; break;
        case '\\': out += '\\'; break;
        case '/': out += '/'; break;
        case 'u': {
            if (i + 4 < text.size()) {
                unsigned code = 0;
                if (std::sscanf(text.substr(i + 1, 4).c_str(), "%4x", &code) == 1) {
                    i += 4;
                    // Basic-multilingual-plane only; enough for the
                    // control chars we emit and typical model output.
                    if (code < 0x80) {
                        out += static_cast<char>(code);
                    } else if (code < 0x800) {
                        out += static_cast<char>(0xC0 | (code >> 6));
                        out += static_cast<char>(0x80 | (code & 0x3F));
                    } else {
                        out += static_cast<char>(0xE0 | (code >> 12));
                        out += static_cast<char>(0x80 | ((code >> 6) & 0x3F));
                        out += static_cast<char>(0x80 | (code & 0x3F));
                    }
                }
            }
            break;
        }
        default: out += next; break;
        }
    }
    return out;
}

std::string extractContent(const std::string &payload) {
    // Scan for the content field rather than fully parsing: the value
    // is the only JSON string we need, and scanning also rides over
    // HTTP chunked framing that may interleave the body. A chunk
    // boundary can in principle split the field mid-token; the v1
    // answer is re-trigger (the daemon's responses are small and
    // usually unframed).
    static const char *kField = "\"content\":";
    size_t at = payload.find(kField);
    if (at == std::string::npos)
        return "";
    at += std::strlen(kField);
    while (at < payload.size() &&
           (payload[at] == ' ' || payload[at] == '\t'))
        ++at;
    if (at >= payload.size() || payload[at] != '"')
        return "";
    ++at;
    std::string raw;
    for (size_t i = at; i < payload.size(); ++i) {
        if (payload[i] == '\\' && i + 1 < payload.size()) {
            raw += payload[i];
            raw += payload[i + 1];
            ++i;
        } else if (payload[i] == '"') {
            return jsonUnescape(raw);
        } else {
            raw += payload[i];
        }
    }
    return "";
}

std::string chatRequestBody(const std::string &systemPrompt,
                            const std::string &userText) {
    return std::string("{\"stream\":false,\"temperature\":0.2,"
                       "\"messages\":["
                       "{\"role\":\"system\",\"content\":\"") +
           jsonEscape(systemPrompt) +
           "\"},{\"role\":\"user\",\"content\":\"" + jsonEscape(userText) +
           "\"}]}";
}

std::string postChatCompletions(const std::string &host, int port,
                                const std::string &body,
                                int timeoutSeconds) {
    int fd = ::socket(AF_INET, SOCK_STREAM, 0);
    if (fd < 0)
        return "";

    timeval tv{};
    tv.tv_sec = timeoutSeconds;
    ::setsockopt(fd, SOL_SOCKET, SO_RCVTIMEO, &tv, sizeof(tv));
    ::setsockopt(fd, SOL_SOCKET, SO_SNDTIMEO, &tv, sizeof(tv));

    sockaddr_in addr{};
    addr.sin_family = AF_INET;
    addr.sin_port = htons(static_cast<uint16_t>(port));
    if (::inet_pton(AF_INET, host.c_str(), &addr.sin_addr) != 1 ||
        ::connect(fd, reinterpret_cast<sockaddr *>(&addr), sizeof(addr)) != 0) {
        ::close(fd);
        return "";
    }

    std::string request =
        "POST /v1/chat/completions HTTP/1.1\r\n"
        "Host: " + host + "\r\n"
        "Content-Type: application/json\r\n"
        "Content-Length: " + std::to_string(body.size()) + "\r\n"
        "Connection: close\r\n\r\n" + body;

    size_t sent = 0;
    while (sent < request.size()) {
        ssize_t n = ::send(fd, request.data() + sent, request.size() - sent, 0);
        if (n <= 0) {
            ::close(fd);
            return "";
        }
        sent += static_cast<size_t>(n);
    }

    std::string response;
    char buf[4096];
    for (;;) {
        ssize_t n = ::recv(fd, buf, sizeof(buf), 0);
        if (n <= 0)
            break;
        response.append(buf, static_cast<size_t>(n));
    }
    ::close(fd);
    return response;
}

} // namespace lisa
