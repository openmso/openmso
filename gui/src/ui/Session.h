#pragma once

#include <QJsonObject>
#include <QObject>
#include <QPointer>

#include "data/Capture.h"
#include "ocp/PluginClient.h"

namespace openmso::data { class LogicSegment; class AnalogSegment; }

namespace openmso::ui {

// Bridges OCP notifications (from PluginClient) to the data model
// (Capture/Signal/Segment). Owns the PluginClient and the Capture.
// Per docs/gui-plan/07-ocp-client.md "Mapping OCP → data model".
//
// One Session = one plugin connection + one capture. The GUI creates
// a Session on Connect, tears it down on Disconnect.
class Session : public QObject {
    Q_OBJECT
public:
    explicit Session(QObject *parent = nullptr);

    data::Capture *capture() const { return capture_; }
    ocp::PluginClient *client() const { return client_; }

    // Take ownership of an already-launched client (transferred from
    // the caller). The session connects notification handling and
    // drives the initialize → scan → open → describe handshake.
    bool attachClient(ocp::PluginClient *client);

    // Drive the full Connect flow: launch the plugin, initialize,
    // scan, open device 0, describe. Returns true on success.
    // `pluginsDir` is the path searched for plugin manifests.
    bool connectDemo(const QString &pluginsDir);

    // Start an acquisition. Returns the capture_id or -1 on error.
    qint64 startCapture();
    void stopCapture();

    void disconnectFromPlugin();

signals:
    // Emitted after describe() returns, with the device summary
    // suitable for the status bar.
    void deviceReady(const QString &summary);
    void deviceError(const QString &message);

private:
    void handleNotification(const QString &method,
                            const QJsonObject &params,
                            const QByteArray &payload);

    void onCaptureBegin(const QJsonObject &params);
    void onCaptureData(const QJsonObject &params, const QByteArray &payload);
    void onCaptureTrigger(const QJsonObject &params);
    void onCaptureEnd(const QJsonObject &params);

    // Find the signal + segment for a stream index, or nullptr.
    struct StreamTarget {
        data::Signal *signal = nullptr;
        data::LogicSegment *logic = nullptr;
        data::AnalogSegment *analog = nullptr;
    };
    StreamTarget resolveStream(int streamIndex) const;

    QPointer<ocp::PluginClient> client_;
    data::Capture *capture_;       // owned (child)

    // Stream index → (signal id list, kind, unitsize/dtype).
    struct StreamInfo {
        int stream = -1;
        QStringList channelIds;   // for logic: 8 ids; for analog: 1 id
        QString kind;             // "logic" | "analog"
        // analog:
        data::AnalogDType dtype = data::AnalogDType::Int8;
        double scale = 1.0;
        double offset = 0.0;
        QString unit;
        // logic:
        int unitsize = 1;
        qint64 sampleCount = 0;   // expected, from capture.begin
    };
    QMap<int, StreamInfo> streams_;
    QString deviceId_;
};

} // namespace openmso::ui
