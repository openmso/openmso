#pragma once

#include <QByteArray>
#include <QJsonObject>

#include <stdexcept>

namespace openmso::ocp {

// Exception raised when a plugin returns a JSON-RPC error response or
// the stream is lost mid-request. Mirrors python/openmso/client.py's
// PluginError.
class PluginError : public std::runtime_error {
public:
    PluginError(int code, const QString &message,
                const QJsonValue &data = QJsonValue());

    int code() const { return code_; }
    QJsonValue data() const { return data_; }

private:
    int code_;
    QJsonValue data_;
};

} // namespace openmso::ocp
