#pragma once

#include <QMap>
#include <QObject>
#include <QPointer>

#include "data/Capture.h"
#include "ocp/PluginClient.h"

namespace openmso::data { class LogicSegment; class AnalogSegment; }

namespace openmso::ui {

// Bridges OCP events to the data model (Capture/Signal/Segment). Owns the
// PluginClient and the Capture.
//
// One Session = one plugin connection = one device, for the life of the
// plugin process: OCP v1 has no reopen, so a new device means a new Session.
class Session : public QObject {
    Q_OBJECT
public:
    explicit Session(QObject *parent = nullptr);

    data::Capture *capture() const { return capture_; }
    ocp::PluginClient *client() const { return client_; }

    // Launch `pluginName` from `pluginsDir` against `device`, then Hello and
    // Describe. An empty `device` tries the manifest's candidate URLs in turn
    // and keeps the first that answers. Returns false with deviceError()
    // emitted on failure.
    bool connectTo(const QString &pluginsDir, const QString &pluginName,
                   const QString &device = {});

    // Returns the capture id, or 0 on error.
    quint64 startCapture(bool continuous = false);
    void stopCapture();

    void disconnectFromPlugin();

signals:
    void deviceReady(const QString &summary);
    void deviceError(const QString &message);

private:
    // One launch + Hello + Describe. Reports failure through `error` rather
    // than deviceError(), so trying the next candidate stays quiet.
    bool tryConnect(const ocp::PluginManifest &manifest, const QString &device,
                    QString *error);

    void onEvent(const ::openmso::pb::Event &event);

    void onCaptureBegin(const ::openmso::pb::CaptureBegin &begin);
    void onAcquisitionBegin(const ::openmso::pb::AcquisitionBegin &begin);
    void onData(const ::openmso::pb::CaptureData &data);
    void onTrigger(const ::openmso::pb::CaptureTrigger &trigger);
    void onCaptureEnd(const ::openmso::pb::CaptureEnd &end);

    struct StreamInfo {
        QStringList channelIds;
        bool logic = false;
        data::AnalogDType dtype = data::AnalogDType::Int8;
        double scale = 1.0;
        double offset = 0.0;
        QString unit;
        int unitsize = 1;
    };

    QPointer<ocp::PluginClient> client_;
    data::Capture *capture_;   // owned (child)

    QMap<quint32, StreamInfo> streams_;
    quint64 captureId_ = 0;
    double samplerate_ = 0.0;
    bool segmentsReady_ = false;
    QString device_;
};

} // namespace openmso::ui
