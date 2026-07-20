#include "Session.h"

#include "data/AnalogSegment.h"
#include "data/LogicSegment.h"
#include "data/Signal.h"
#include "ocp/PluginManifest.h"

#include <QJsonArray>

namespace openmso::ui {

namespace {

data::AnalogDType parseDType(const QString &s)
{
    if (s == "int8")    return data::AnalogDType::Int8;
    if (s == "uint8")   return data::AnalogDType::UInt8;
    if (s == "int16")   return data::AnalogDType::Int16;
    if (s == "uint16")  return data::AnalogDType::UInt16;
    if (s == "float32") return data::AnalogDType::Float32;
    if (s == "float64") return data::AnalogDType::Float64;
    return data::AnalogDType::Int8;
}

} // namespace

Session::Session(QObject *parent)
    : QObject(parent), capture_(new data::Capture(this)) {}

bool Session::attachClient(ocp::PluginClient *client)
{
    if (client_)
        disconnectFromPlugin();
    client_ = client;
    if (client_) {
        client_->setParent(this);
        connect(client_, &ocp::PluginClient::notification,
                this, &Session::handleNotification);
        connect(client_, &ocp::PluginClient::disconnected,
                this, [this]{ emit deviceError(QStringLiteral("plugin disconnected")); });
    }
    return client_ != nullptr;
}

bool Session::connectDemo(const QString &pluginsDir)
{
    auto manifest = openmso::ocp::findPlugin(pluginsDir, QStringLiteral("demo"));
    if (manifest.name.isEmpty()) {
        emit deviceError(QStringLiteral("demo plugin not found under %1")
                             .arg(pluginsDir));
        return false;
    }
    auto *c = openmso::ocp::PluginClient::launch(manifest, this);
    if (!c) {
        emit deviceError(QStringLiteral("failed to launch demo plugin"));
        return false;
    }
    attachClient(c);

    try {
        c->initialize();
        const auto scan = c->request(QStringLiteral("scan"));
        const auto devices = scan.value("devices").toArray();
        if (devices.isEmpty()) {
            emit deviceError(QStringLiteral("demo returned no devices"));
            return false;
        }
        deviceId_ = devices.first().toObject().value("device_id").toString();
        c->request(QStringLiteral("open"),
                   QJsonObject{{"device_id", deviceId_}});

        // Pre-create the signal list from describe() so the view has
        // something to show before capture.begin. capture.begin will
        // create segments and attach them to these signals.
        const auto desc = c->request(QStringLiteral("describe"));
    QList<data::Capture::ChannelSpec> specs;
        const auto channels = desc.value("channels").toArray();
        for (const auto &v : channels) {
            const auto ch = v.toObject();
            const QString kind = ch.value("kind").toString();
            specs.append({ch.value("id").toString(),
                          ch.value("name").toString(),
                          kind == "analog" ? data::SignalKind::Analog
                                           : data::SignalKind::Logic});
        }
        capture_->beginCapture(0, 0, specs);  // placeholder sr/t0 until begin
        emit deviceReady(QStringLiteral("demo://0 Demo MSO"));
        return true;
    } catch (const openmso::ocp::PluginError &e) {
        emit deviceError(QString::fromStdString(e.what()));
        return false;
    }
}

qint64 Session::startCapture()
{
    if (!client_) return -1;
    try {
        const auto r = client_->request(
            QStringLiteral("acquire.start"),
            QJsonObject{{"mode", QStringLiteral("single")}});
        return r.value("capture_id").toVariant().toLongLong();
    } catch (const openmso::ocp::PluginError &e) {
        emit deviceError(QString::fromStdString(e.what()));
        return -1;
    }
}

void Session::stopCapture()
{
    if (!client_) return;
    try {
        client_->request(QStringLiteral("acquire.stop"));
    } catch (const openmso::ocp::PluginError &e) {
        emit deviceError(QString::fromStdString(e.what()));
    }
}

void Session::disconnectFromPlugin()
{
    streams_.clear();
    if (client_) {
        client_->shutdown();
        client_ = nullptr;
    }
}

// ---- notification dispatch --------------------------------------------

void Session::handleNotification(const QString &method,
                                 const QJsonObject &params,
                                 const QByteArray &payload)
{
    if (method == "capture.begin") onCaptureBegin(params);
    else if (method == "capture.data") onCaptureData(params, payload);
    else if (method == "capture.trigger") onCaptureTrigger(params);
    else if (method == "capture.end") onCaptureEnd(params);
    // Other notifications (event.status, log) ignored at v0.1.
}

void Session::onCaptureBegin(const QJsonObject &params)
{
    const double sr = params.value("samplerate").toDouble();
    const double t0 = params.value("t0").toDouble();
    streams_.clear();

    // Rebuild the channel list in stream order so segments attach to
    // the right Signal.
    QList<data::Capture::ChannelSpec> specs;
    const auto streams = params.value("streams").toArray();
    for (const auto &v : streams) {
        const auto s = v.toObject();
        StreamInfo info;
        info.stream = s.value("stream").toInt();
        info.kind = s.value("kind").toString();
        info.sampleCount = s.value("sample_count").toVariant().toLongLong();
        const auto chs = s.value("channels").toArray();
        for (const auto &c : chs)
            info.channelIds.append(c.toString());
        const auto enc = s.value("encoding").toObject();
        if (info.kind == "analog") {
            info.dtype = parseDType(enc.value("dtype").toString());
            info.scale = enc.value("scale").toDouble(1.0);
            info.offset = enc.value("offset").toDouble(0.0);
            info.unit = enc.value("unit").toString("V");
        } else {
            info.unitsize = enc.value("unitsize").toInt(1);
        }
        streams_.insert(info.stream, info);

        for (const auto &id : info.channelIds) {
            const auto kind = (info.kind == "analog")
                                  ? data::SignalKind::Analog
                                  : data::SignalKind::Logic;
            specs.append({id, id, kind});
        }
    }

    capture_->beginCapture(sr, t0, specs);

    // Now create segments on each signal. For a logic stream, every
    // channel signal gets its own LogicSegment that holds the same
    // bit-packed bytes (the painter reads only "its" bit out of each
    // byte). This is slightly wasteful in memory (8× the bytes for an
    // 8-channel stream) but trivial for the demo's 100k samples and
    // keeps the data model simple: one Signal → one Segment. A future
    // optimization can share the segment across sibling signals.
    for (auto it = streams_.begin(); it != streams_.end(); ++it) {
        const auto &info = it.value();
        for (const auto &id : info.channelIds) {
            auto *sig = capture_->signalById(id);
            if (!sig) continue;
            if (info.kind == "logic") {
                auto *seg = new data::LogicSegment(
                    info.unitsize, info.channelIds.size(), sig);
                seg->setSamplerate(sr);
                sig->appendSegment(seg);
            } else {
                auto *seg = new data::AnalogSegment(
                    info.dtype, info.scale, info.offset, info.unit, sig);
                seg->setSamplerate(sr);
                sig->appendSegment(seg);
            }
        }
    }
}

void Session::onCaptureData(const QJsonObject &params,
                            const QByteArray &payload)
{
    const int stream = params.value("stream").toInt();
    const qint64 firstSample =
        params.value("first_sample").toVariant().toLongLong();
    const qint64 nsamples =
        params.value("nsamples").toVariant().toLongLong();

    // Fan out to every signal in the stream. For logic streams each
    // signal has its own (duplicate) segment; for analog streams there
    // is exactly one signal per stream.
    auto it = streams_.find(stream);
    if (it == streams_.end()) return;
    const auto &info = it.value();
    for (const auto &id : info.channelIds) {
        auto *sig = capture_->signalById(id);
        if (!sig) continue;
        auto *seg = sig->primarySegment();
        if (info.kind == "logic") {
            if (auto *l = qobject_cast<data::LogicSegment *>(seg))
                l->appendChunk(payload, firstSample, nsamples);
        } else {
            if (auto *a = qobject_cast<data::AnalogSegment *>(seg))
                a->appendChunk(payload, firstSample, nsamples);
        }
    }
    capture_->notifyAppend(stream, firstSample, nsamples);
}

void Session::onCaptureTrigger(const QJsonObject &params)
{
    const qint64 sample = params.value("sample").toVariant().toLongLong();
    capture_->setTriggerSample(sample);
}

void Session::onCaptureEnd(const QJsonObject &params)
{
    const bool ok = params.value("ok").toBool(true);
    const QString err = params.value("error").toString();
    capture_->endCapture(ok, err);
}

Session::StreamTarget Session::resolveStream(int streamIndex) const
{
    StreamTarget t;
    auto it = streams_.find(streamIndex);
    if (it == streams_.end()) return t;
    const auto &info = it.value();
    for (const auto &id : info.channelIds) {
        auto *sig = capture_->signalById(id);
        if (!sig) continue;
        t.signal = sig;
        auto *seg = sig->primarySegment();
        if (info.kind == "logic")
            t.logic = qobject_cast<data::LogicSegment *>(seg);
        else
            t.analog = qobject_cast<data::AnalogSegment *>(seg);
        if (t.logic || t.analog) return t;
    }
    return t;
}

} // namespace openmso::ui
