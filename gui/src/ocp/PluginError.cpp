#include "PluginError.h"

#include <QString>

namespace openmso::ocp {

PluginError::PluginError(int code, const QString &message,
                         const QJsonValue &data)
    : std::runtime_error(
          QStringLiteral("[%1] %2").arg(code).arg(message).toStdString()),
      code_(code),
      data_(data) {}

} // namespace openmso::ocp
