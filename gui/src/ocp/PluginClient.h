#pragma once

#include <openmso/client.h>

#include <QObject>
#include <QString>
#include <QThread>

#include <atomic>
#include <memory>

#include "PluginManifest.h"

Q_DECLARE_METATYPE(openmso::pb::Event)

namespace openmso::ocp {

// Pulls events off the stream socket so a saturated data pipe never stalls
// the GUI thread. Lives on its own QThread; every emission is queued.
class EventReader : public QObject {
    Q_OBJECT
public:
    explicit EventReader(::openmso::EventStream stream);

    void stop() { stop_ = true; }

public slots:
    void run();

signals:
    void event(const ::openmso::pb::Event &event);
    void failed(const QString &what);
    void finished();

private:
    ::openmso::EventStream stream_;
    std::atomic<bool> stop_{false};
};

// Frontend-side OCP client: a plugin subprocess driven over nng.
//
// Control requests block the calling thread until the plugin replies, which
// is what a REQ socket is; events arrive as queued signals from EventReader.
class PluginClient : public QObject {
    Q_OBJECT
public:
    // Null on failure, with `error` set to why.
    static PluginClient *launch(const PluginManifest &manifest,
                                const QString &device, QString *error,
                                QObject *parent = nullptr);

    ~PluginClient() override;

    ::openmso::pb::HelloResult hello(const QString &clientName,
                                     const QString &clientVersion);
    ::openmso::pb::Description describe();
    ::openmso::pb::Config getConfig();
    ::openmso::pb::Config setConfig(const ::openmso::pb::Config &config);

    // Allocates and returns the capture id.
    quint64 acquireStart(::openmso::pb::AcquireMode mode);
    void acquireStop(quint64 captureId);
    void reset();

    // Stops the reader, asks the plugin to exit, reaps it. Idempotent.
    void shutdown();

    bool isRunning() const;

signals:
    void event(const ::openmso::pb::Event &event);
    void streamFailed(const QString &what);

private:
    explicit PluginClient(::openmso::CaptureClient client, QObject *parent);

    void startReader();

    std::unique_ptr<::openmso::CaptureClient> client_;
    QThread *thread_ = nullptr;
    EventReader *reader_ = nullptr;
};

} // namespace openmso::ocp
