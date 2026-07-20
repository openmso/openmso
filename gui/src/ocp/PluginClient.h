#pragma once

#include "MessageStream.h"
#include "PluginError.h"
#include "PluginManifest.h"

#include <QJsonObject>
#include <QObject>
#include <QPointer>

#include <functional>

class QIODevice;
class QProcess;
class QTcpSocket;
namespace openmso::ocp {

// Frontend-side OCP client. Launches (or connects to) a capture plugin
// and speaks JSON-RPC 2.0 with line + binary framing (docs/protocol.md).
//
// This is the C++ port of python/openmso/client.py's PluginClient. It
// is event-driven: the underlying QIODevice (QProcess stdout or
// QTcpSocket) emits readyRead on the GUI thread, MessageStream parses
// complete messages, and responses are dispatched to either the
// synchronous request() caller (via a QEventLoop) or the async
// callback. Notifications fire as a Qt signal.
//
// Threading: all I/O happens on the GUI thread. There is no reader
// thread; the Python implementation's threading.Event / reader loop
// maps to QEventLoop + readyRead. This is simpler and avoids
// cross-thread QIODevice access (which Qt warns against).
//
// Lifetime: owns its QProcess (if launched) and QTcpSocket (if
// connected). Delete the PluginClient or call shutdown() to tear down.
class PluginClient : public QObject {
    Q_OBJECT
public:
    using ResponseCb =
        std::function<void(const QJsonObject &result, PluginError *error)>;

    using NotificationHandler =
        std::function<void(const QString &method,
                           const QJsonObject &params,
                           const QByteArray &payload)>;

    // Spawn a plugin subprocess speaking OCP on its stdio. stderr is
    // inherited so plugin diagnostics reach the user. Returns nullptr
    // if the process fails to start.
    static PluginClient *launch(const PluginManifest &manifest,
                                QObject *parent = nullptr);
    static PluginClient *launch(const QStringList &argv,
                                const QString &workingDir = {},
                                QObject *parent = nullptr);

    // Connect to a plugin listening on TCP. Returns nullptr if the
    // socket fails to connect.
    static PluginClient *connectToHost(const QString &host, quint16 port,
                                      QObject *parent = nullptr);

    ~PluginClient() override;

    // Send a request synchronously, wait for the result. Throws
    // PluginError on JSON-RPC error, plugin exit, or timeout. Default
    // timeout (60s) matches python/openmso/client.py.
    QJsonObject request(const QString &method,
                        const QJsonObject &params = {},
                        const QByteArray &payload = {},
                        int timeoutMs = 60000);

    // Async variant: returns immediately. The callback runs on the
    // GUI thread when the response arrives (or on error/EOF).
    int requestAsync(const QString &method,
                     const QJsonObject &params,
                     ResponseCb cb,
                     const QByteArray &payload = {});

    // Fire-and-forget notification (no id, no response expected).
    void sendNotification(const QString &method,
                         const QJsonObject &params = {},
                         const QByteArray &payload = {});

    // Convenience for the common "initialize" handshake.
    QJsonObject initialize(const QString &clientName =
                               QStringLiteral("openmso-gui"),
                           const QString &clientVersion =
                               QStringLiteral("0.1.0"));

    void setNotificationHandler(NotificationHandler h);

    // Send shutdown, close the stream, wait for the process to exit.
    // Safe to call multiple times. After shutdown, the client is
    // inert; further requests will throw PluginError(-1, ...).
    void shutdown();

    bool isConnected() const;

signals:
    // Emitted on the GUI thread for every notification received.
    void notification(QString method, QJsonObject params, QByteArray payload);

    // Emitted when the underlying stream ends (process exit or socket
    // disconnect). The client is unusable afterwards.
    void disconnected();

private:
    explicit PluginClient(QObject *parent = nullptr);

    // Wire readyRead/error/finished of io_ to our slots. io_ must
    // already be opened.
    void attach(QIODevice *io);

    void onReadyRead();
    void onAboutToClose();
    void handleParsedMessage(const QJsonObject &msg,
                             const QByteArray &payload);
    void failAllPending(const QString &reason);

    // Serialize + write a message. Returns false on write failure.
    bool writeMessage(const QJsonObject &msg, const QByteArray &payload);

    QIODevice *io_ = nullptr;
    QPointer<QProcess> proc_;       // nullptr for TCP clients
    QPointer<QTcpSocket> sock_;     // nullptr for stdio clients

    MessageStream stream_;

    struct PendingSlot {
        int id = 0;
        bool isAsync = false;
        ResponseCb asyncCb;
        QJsonObject *syncResult = nullptr;
        PluginError *syncError = nullptr;
        QPointer<QObject> loop;     // QEventLoop for sync request()
        bool *syncGotResponse = nullptr;  // set true on response
    };
    QMap<int, PendingSlot> pending_;
    int nextId_ = 0;
    NotificationHandler handler_;
};

} // namespace openmso::ocp
