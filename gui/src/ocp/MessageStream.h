#pragma once

#include <QByteArray>
#include <QJsonObject>

#include <functional>

namespace openmso::ocp {

// Parses the OCP wire framing (docs/protocol.md §1) from a byte stream.
//
// The wire format is: one UTF-8 JSON object per LF-terminated line,
// optionally followed by `binlen` raw bytes of binary payload. Empty
// lines are skipped. This is a push-driven parser: feed bytes via
// feed(), and onMessage() is called for each complete message. EOF is
// signaled via onEof(). This is the C++ twin of python/openmso/
// framing.py's MessageStream.read_message loop, but inverted: instead
// of blocking reads, the caller pushes bytes when a QIODevice says
// they're available.
//
// Threading: not thread-safe. Intended to be driven from a single
// thread (the GUI thread, via QProcess/QTcpSocket readyRead signals).
class MessageStream {
public:
    using MessageCb =
        std::function<void(const QJsonObject &msg,
                           const QByteArray &payload)>;
    using EofCb = std::function<void()>;
    using ErrorCb = std::function<void(const QString &what)>;

    void onMessage(MessageCb cb) { msg_cb_ = std::move(cb); }
    void onEof(EofCb cb) { eof_cb_ = std::move(cb); }
    void onError(ErrorCb cb) { err_cb_ = std::move(cb); }

    // Append bytes from a QIODevice. Parses as many complete messages
    // as available; calls onMessage() for each. Returns the number of
    // messages dispatched.
    int feed(const QByteArray &bytes);

    // Called by the owner when the underlying QIODevice hit EOF.
    void feedEof();

private:
    // Returns the parsed message and its payload, or nullopt-ish
    // (outMsg left empty) if the buffer doesn't yet hold a full
    // message. Consumes the bytes from line_buf_ on success.
    bool tryParseOne(QJsonObject &outMsg, QByteArray &outPayload);

    QByteArray line_buf_;
    MessageCb msg_cb_;
    EofCb eof_cb_;
    ErrorCb err_cb_;
};

} // namespace openmso::ocp
