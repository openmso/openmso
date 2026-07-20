#include "PluginClient.h"

#include <QAbstractSocket>
#include <QEventLoop>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QProcess>
#include <QTcpSocket>
#include <QTimer>

namespace openmso::ocp {

namespace {

constexpr int kProtocolVersion = 0;

QByteArray serializeMessage(QJsonObject msg,
                            const QByteArray &payload)
{
    if (!payload.isEmpty())
        msg.insert(QStringLiteral("binlen"), payload.size());
    msg.insert(QStringLiteral("jsonrpc"), QStringLiteral("2.0"));
    return QJsonDocument(msg).toJson(QJsonDocument::Compact) + '\n';
}

} // namespace

PluginClient::PluginClient(QObject *parent)
    : QObject(parent)
{
    stream_.onMessage([this](const QJsonObject &msg, const QByteArray &payload) {
        handleParsedMessage(msg, payload);
    });
    stream_.onEof([this] { onAboutToClose(); });
    stream_.onError([this](const QString &what) {
        qWarning("ocp: stream parse error: %s", qPrintable(what));
    });
}

PluginClient::~PluginClient()
{
    shutdown();
}

bool PluginClient::isConnected() const
{
    return io_ != nullptr && io_->isOpen();
}

// ---- factories ---------------------------------------------------------

PluginClient *PluginClient::launch(const PluginManifest &manifest,
                                   QObject *parent)
{
    return launch(manifest.argv, manifest.pluginDir, parent);
}

PluginClient *PluginClient::launch(const QStringList &argv,
                                   const QString &workingDir,
                                   QObject *parent)
{
    auto *c = new PluginClient(parent);
    auto *p = new QProcess(c);
    p->setProcessChannelMode(QProcess::SeparateChannels); // stdout=OCP, stderr inherited
    if (!workingDir.isEmpty())
        p->setWorkingDirectory(workingDir);

    // Wait for the process to actually start; QProcess::start() is
    // async, but we want launch() to return a client whose io_ is
    // either usable or nullptr.
    p->start(argv.isEmpty() ? QString() : argv.first(),
             argv.isEmpty() ? QStringList{} : argv.mid(1));
    if (!p->waitForStarted(3000)) {
        qWarning("ocp: failed to start plugin: %s",
                 qPrintable(p->errorString()));
        delete c;
        return nullptr;
    }
    c->proc_ = p;
    c->attach(p);
    return c;
}

PluginClient *PluginClient::connectToHost(const QString &host, quint16 port,
                                         QObject *parent)
{
    auto *c = new PluginClient(parent);
    auto *s = new QTcpSocket(c);
    s->connectToHost(host, port);
    if (!s->waitForConnected(3000)) {
        qWarning("ocp: failed to connect to %s:%u: %s",
                 qPrintable(host), port, qPrintable(s->errorString()));
        delete c;
        return nullptr;
    }
    c->sock_ = s;
    c->attach(s);
    return c;
}

void PluginClient::attach(QIODevice *io)
{
    io_ = io;
    QObject::connect(io, &QIODevice::readyRead,
                     this, &PluginClient::onReadyRead);
    QObject::connect(io, &QIODevice::aboutToClose,
                     this, &PluginClient::onAboutToClose);
    // Drain anything already in the buffer (rare, but safe).
    if (io->bytesAvailable() > 0)
        onReadyRead();
}

// ---- request paths -----------------------------------------------------

QJsonObject PluginClient::request(const QString &method,
                                  const QJsonObject &params,
                                  const QByteArray &payload,
                                  int timeoutMs)
{
    if (!io_ || !io_->isOpen())
        throw PluginError(-1, QStringLiteral("client not connected"));

    const int id = ++nextId_;
    QJsonObject msg{
        {QStringLiteral("id"), id},
        {QStringLiteral("method"), method},
        {QStringLiteral("params"), params},
    };

    QJsonObject result;
    PluginError err(-1, QStringLiteral("no response"));
    bool gotResponse = false;
    QEventLoop loop;
    pending_[id] = PendingSlot{
        id, /*isAsync=*/false, ResponseCb{},
        &result, &err, &loop, &gotResponse
    };

    if (!writeMessage(msg, payload)) {
        pending_.remove(id);
        throw PluginError(-1, QStringLiteral("write failed"));
    }

    QTimer::singleShot(timeoutMs, &loop, &QEventLoop::quit);
    loop.exec();

    if (pending_.contains(id)) {
        // Timed out — no response ever arrived.
        pending_.remove(id);
        throw PluginError(-1, QStringLiteral("no response to '%1' within %2ms")
                              .arg(method).arg(timeoutMs));
    }
    if (!gotResponse) {
        // Stream closed before the response arrived (failAllPending ran).
        throw PluginError(-1, QStringLiteral("plugin exited before responding"));
    }
    if (err.code() != -1)
        throw err;
    return result;
}

int PluginClient::requestAsync(const QString &method,
                               const QJsonObject &params,
                               ResponseCb cb,
                               const QByteArray &payload)
{
    const int id = ++nextId_;
    QJsonObject msg{
        {QStringLiteral("id"), id},
        {QStringLiteral("method"), method},
        {QStringLiteral("params"), params},
    };
    pending_[id] = PendingSlot{
        id, /*isAsync=*/true, std::move(cb),
        /*syncResult=*/nullptr, /*syncError=*/nullptr, /*loop=*/nullptr
    };
    if (!writeMessage(msg, payload)) {
        pending_.remove(id);
        if (cb) cb({}, new PluginError(-1, QStringLiteral("write failed")));
    }
    return id;
}

void PluginClient::sendNotification(const QString &method,
                                   const QJsonObject &params,
                                   const QByteArray &payload)
{
    QJsonObject msg{
        {QStringLiteral("method"), method},
        {QStringLiteral("params"), params},
    };
    writeMessage(msg, payload);
}

QJsonObject PluginClient::initialize(const QString &clientName,
                                    const QString &clientVersion)
{
    return request(QStringLiteral("initialize"),
                   QJsonObject{
                       {QStringLiteral("protocol_version"), kProtocolVersion},
                       {QStringLiteral("client"), QJsonObject{
                           {QStringLiteral("name"), clientName},
                           {QStringLiteral("version"), clientVersion},
                       }},
                   });
}

void PluginClient::setNotificationHandler(NotificationHandler h)
{
    handler_ = std::move(h);
}

void PluginClient::shutdown()
{
    if (io_ && io_->isOpen()) {
        // Best-effort shutdown request; ignore errors.
        try {
            // Use a short timeout so a wedged plugin doesn't hang us.
            // We bypass request() because we don't want to throw.
            const int id = ++nextId_;
            QJsonObject msg{
                {QStringLiteral("id"), id},
                {QStringLiteral("method"), QStringLiteral("shutdown")},
                {QStringLiteral("params"), QJsonObject{}},
            };
            writeMessage(msg, {});
            // Don't wait for the response — closing the stream is
            // authoritative. The reader will failAllPending() on EOF.
        } catch (...) {
            // ignore
        }
    }

    if (proc_) {
        proc_->closeWriteChannel();
        if (proc_->state() != QProcess::NotRunning) {
            if (!proc_->waitForFinished(3000))
                proc_->kill();
        }
    }
    if (sock_) {
        sock_->abort();
    }
    if (io_) {
        io_->close();
        io_ = nullptr;
    }
    failAllPending(QStringLiteral("shutdown"));
}

// ---- reader ------------------------------------------------------------

void PluginClient::onReadyRead()
{
    if (!io_)
        return;
    // Read whatever is available; MessageStream will buffer partial
    // messages and dispatch complete ones.
    QByteArray chunk = io_->readAll();
    stream_.feed(chunk);
}

void PluginClient::onAboutToClose()
{
    failAllPending(QStringLiteral("stream closed"));
    if (io_) io_ = nullptr;
    emit disconnected();
}

void PluginClient::handleParsedMessage(const QJsonObject &msg,
                                       const QByteArray &payload)
{
    const auto idVal = msg.value(QStringLiteral("id"));

    // Response to a request?
    if (!idVal.isUndefined() && !msg.contains(QStringLiteral("method"))) {
        const int id = idVal.toInt(-1);
        auto it = pending_.find(id);
        if (it == pending_.end())
            return; // unknown id — probably a duplicate; drop it

        PendingSlot slot = it.value();
        pending_.erase(it);

        if (msg.contains(QStringLiteral("error"))) {
            const auto errObj = msg.value(QStringLiteral("error")).toObject();
            PluginError e(errObj.value(QStringLiteral("code")).toInt(-1),
                          errObj.value(QStringLiteral("message")).toString(),
                          errObj.value(QStringLiteral("data")));
            if (slot.isAsync) {
                if (slot.asyncCb)
                    slot.asyncCb({}, &e);
            } else if (slot.syncError) {
                *slot.syncError = e;
                if (slot.syncGotResponse) *slot.syncGotResponse = true;
            }
        } else {
            const QJsonObject result =
                msg.value(QStringLiteral("result")).toObject();
            if (slot.isAsync) {
                if (slot.asyncCb)
                    slot.asyncCb(result, nullptr);
            } else if (slot.syncResult) {
                *slot.syncResult = result;
                if (slot.syncGotResponse) *slot.syncGotResponse = true;
            }
        }
        if (slot.loop)
            QMetaObject::invokeMethod(slot.loop, "quit", Qt::QueuedConnection);
        return;
    }

    // Notification (has method, no id) or a server-initiated request
    // (has method AND id — we don't support those, treat as notif).
    if (msg.contains(QStringLiteral("method"))) {
        const QString method = msg.value(QStringLiteral("method")).toString();
        const QJsonObject params =
            msg.value(QStringLiteral("params")).toObject();
        // Always emit the Qt signal (queued from a worker thread if
        // we ever add one; direct here since we're on the GUI thread).
        emit notification(method, params, payload);
        if (handler_) {
            try {
                handler_(method, params, payload);
            } catch (const std::exception &e) {
                qWarning("ocp: notification handler threw: %s", e.what());
            }
        }
    }
}

void PluginClient::failAllPending(const QString &reason)
{
    auto pending = std::move(pending_);
    pending_.clear();
    for (auto &slot : pending) {
        if (slot.isAsync) {
            if (slot.asyncCb)
                slot.asyncCb({}, new PluginError(-1, reason));
        } else if (slot.syncError) {
            *slot.syncError = PluginError(-1, reason);
        }
        if (slot.loop)
            QMetaObject::invokeMethod(slot.loop, "quit", Qt::QueuedConnection);
    }
}

bool PluginClient::writeMessage(const QJsonObject &msg,
                                const QByteArray &payload)
{
    if (!io_ || !io_->isOpen())
        return false;
    const QByteArray bytes = serializeMessage(msg, payload);
    qint64 written = 0;
    while (written < bytes.size()) {
        const qint64 n = io_->write(bytes.constData() + written,
                                    bytes.size() - written);
        if (n < 0)
            return false;
        written += n;
    }
    // QProcess and QTcpSocket both have flush(); QIODevice doesn't.
    // For QProcess, write() already pushes to the OS pipe; for
    // QTcpSocket we want the bytes on the wire now.
    if (auto *p = qobject_cast<QProcess *>(io_))
        p->waitForBytesWritten(100);
    else if (auto *s = qobject_cast<QTcpSocket *>(io_))
        s->flush();
    return true;
}

} // namespace openmso::ocp
