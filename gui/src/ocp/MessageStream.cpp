#include "MessageStream.h"

#include <QJsonDocument>
#include <QJsonParseError>

namespace openmso::ocp {

int MessageStream::feed(const QByteArray &bytes)
{
    int dispatched = 0;
    line_buf_.append(bytes);

    forever {
        QJsonObject msg;
        QByteArray payload;
        if (!tryParseOne(msg, payload))
            break;
        if (msg_cb_) msg_cb_(msg, payload);
        ++dispatched;
    }
    return dispatched;
}

void MessageStream::feedEof()
{
    if (!line_buf_.isEmpty()) {
        // Trailing bytes without a newline. If they form a complete
        // JSON object without a payload, accept it (some plugins may
        // omit the final LF); otherwise it's a truncated message.
        if (!line_buf_.contains('\n')) {
            QJsonParseError err;
            auto doc = QJsonDocument::fromJson(line_buf_, &err);
            if (err.error == QJsonParseError::NoError && doc.isObject()) {
                if (msg_cb_) msg_cb_(doc.object(), QByteArray());
                line_buf_.clear();
            } else if (err_cb_) {
                err_cb_(QStringLiteral("truncated final message: %1")
                            .arg(QString::fromLatin1(line_buf_.left(200))));
                line_buf_.clear();
            }
        } else if (err_cb_) {
            err_cb_(QStringLiteral("unconsumed bytes at EOF"));
            line_buf_.clear();
        }
    }
    if (eof_cb_) eof_cb_();
}

bool MessageStream::tryParseOne(QJsonObject &outMsg, QByteArray &outPayload)
{
    int nl = line_buf_.indexOf('\n');
    if (nl < 0)
        return false;

    QByteArray line = line_buf_.left(nl);
    // Remove the line + its terminator.
    line_buf_.remove(0, nl + 1);

    if (line.trimmed().isEmpty())
        return tryParseOne(outMsg, outPayload); // skip blank lines, recurse

    QJsonParseError parseErr;
    auto doc = QJsonDocument::fromJson(line, &parseErr);
    if (parseErr.error != QJsonParseError::NoError || !doc.isObject()) {
        if (err_cb_) {
            err_cb_(QStringLiteral("bad JSON line: %1: %2")
                        .arg(parseErr.errorString(),
                             QString::fromLatin1(line.left(200))));
        }
        return tryParseOne(outMsg, outPayload); // resync on next line
    }

    outMsg = doc.object();

    // Optional binary payload.
    auto binlen = outMsg.value(QStringLiteral("binlen"));
    if (binlen.isUndefined() || binlen.isNull()) {
        outPayload.clear();
        return true;
    }
    if (!binlen.isDouble()) {
        if (err_cb_) {
            err_cb_(QStringLiteral("invalid binlen (not a number): %1")
                        .arg(binlen.toVariant().toString()));
        }
        return tryParseOne(outMsg, outPayload); // resync
    }
    int n = binlen.toInt(-1);
    if (n < 0) {
        if (err_cb_) {
            err_cb_(QStringLiteral("invalid binlen (negative): %1").arg(n));
        }
        return tryParseOne(outMsg, outPayload); // resync
    }
    if (line_buf_.size() < n) {
        // Not enough bytes yet. Put the line back so the next feed()
        // can retry. We prepend the JSON line + '\n' again.
        line_buf_.prepend('\n');
        line_buf_.prepend(line);
        return false;
    }
    outPayload = line_buf_.left(n);
    line_buf_.remove(0, n);
    return true;
}

} // namespace openmso::ocp
