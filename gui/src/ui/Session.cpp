#include "Session.h"

#include <openmso/encoding.h>

#include "data/AnalogSegment.h"
#include "data/LogicSegment.h"
#include "data/Signal.h"
#include "ocp/PluginManifest.h"

namespace openmso::ui {

namespace {

namespace pb = ::openmso::pb;

data::AnalogDType dtypeOf(pb::SampleType type)
{
    switch (type) {
    case pb::SAMPLE_UINT8:   return data::AnalogDType::UInt8;
    case pb::SAMPLE_INT16:   return data::AnalogDType::Int16;
    case pb::SAMPLE_UINT16:  return data::AnalogDType::UInt16;
    case pb::SAMPLE_FLOAT32: return data::AnalogDType::Float32;
    case pb::SAMPLE_FLOAT64: return data::AnalogDType::Float64;
    default:                 return data::AnalogDType::Int8;
    }
}

QString summarise(const pb::HelloResult &hello, const QString &device)
{
    const auto &d = hello.device();
    QStringList parts;
    if (!d.vendor().empty())
        parts << QString::fromStdString(d.vendor());
    if (!d.model().empty())
        parts << QString::fromStdString(d.model());
    if (parts.isEmpty())
        parts << QString::fromStdString(hello.plugin().name());
    return device + " " + parts.join(' ');
}

} // namespace

Session::Session(QObject *parent)
    : QObject(parent), capture_(new data::Capture(this)) {}

bool Session::connectTo(const QString &pluginsDir, const QString &pluginName,
                        const QString &device)
{
    const auto manifest = ocp::findPlugin(pluginsDir, pluginName);
    if (manifest.isNull()) {
        emit deviceError(tr("plugin '%1' not found under %2")
                             .arg(pluginName, pluginsDir));
        return false;
    }

    QStringList candidates;
    if (device.isEmpty())
        candidates = ocp::candidateDeviceUrls(manifest);
    else
        candidates << device;

    if (candidates.isEmpty()) {
        emit deviceError(tr("%1 declares no device this frontend can address")
                             .arg(pluginName));
        return false;
    }

    QStringList failures;
    for (const QString &candidate : candidates) {
        QString error;
        if (tryConnect(manifest, candidate, &error))
            return true;
        failures << tr("%1: %2").arg(candidate, error);
    }

    emit deviceError(failures.join(QStringLiteral("\n")));
    return false;
}

bool Session::tryConnect(const ocp::PluginManifest &manifest,
                         const QString &device, QString *error)
{
    auto *client = ocp::PluginClient::launch(manifest, device, error, this);
    if (!client)
        return false;

    client_ = client;
    device_ = device;
    connect(client_, &ocp::PluginClient::event, this, &Session::onEvent);
    connect(client_, &ocp::PluginClient::streamFailed, this,
            [this](const QString &what) { emit deviceError(what); });

    try {
        const auto hello = client_->hello(QStringLiteral("omso"),
                                          QStringLiteral("0.1.0"));
        const auto description = client_->describe();

        QList<data::Capture::ChannelSpec> specs;
        int analogOrd = 0, logicOrd = 0;
        for (const auto &channel : description.channels()) {
            const bool analog = channel.kind() == pb::CHANNEL_ANALOG;
            specs.append({QString::fromStdString(channel.id()),
                          QString::fromStdString(channel.name()),
                          analog ? data::SignalKind::Analog
                                 : data::SignalKind::Logic,
                          analog ? analogOrd++ : logicOrd++});
        }
        capture_->declareChannels(specs);

        emit deviceReady(summarise(hello, device));
        return true;
    } catch (const ::openmso::Error &e) {
        *error = QString::fromUtf8(e.what());
        disconnectFromPlugin();
        return false;
    }
}

quint64 Session::startCapture(bool continuous)
{
    if (!client_)
        return 0;
    try {
        segmentsReady_ = false;
        captureId_ = client_->acquireStart(continuous ? pb::ACQUIRE_CONTINUOUS
                                                      : pb::ACQUIRE_SINGLE);
        return captureId_;
    } catch (const ::openmso::Error &e) {
        emit deviceError(QString::fromUtf8(e.what()));
        return 0;
    }
}

void Session::stopCapture()
{
    if (!client_ || captureId_ == 0)
        return;
    try {
        client_->acquireStop(captureId_);
    } catch (const ::openmso::Error &e) {
        emit deviceError(QString::fromUtf8(e.what()));
    }
}

void Session::disconnectFromPlugin()
{
    streams_.clear();
    captureId_ = 0;
    if (client_) {
        // Detach first: shutdown() can emit, and a deviceError() from here
        // would let MainWindow delete this Session while it is on the stack.
        disconnect(client_, nullptr, this, nullptr);
        client_->shutdown();
        client_->deleteLater();
        client_ = nullptr;
    }
}

void Session::onEvent(const pb::Event &event)
{
    switch (event.event_case()) {
    case pb::Event::kCaptureBegin:     onCaptureBegin(event.capture_begin()); break;
    case pb::Event::kAcquisitionBegin: onAcquisitionBegin(event.acquisition_begin()); break;
    case pb::Event::kData:             onData(event.data()); break;
    case pb::Event::kTrigger:          onTrigger(event.trigger()); break;
    case pb::Event::kCaptureEnd:       onCaptureEnd(event.capture_end()); break;
    case pb::Event::kDeviceLost:
        emit deviceError(tr("device lost: %1")
                             .arg(QString::fromStdString(event.device_lost().reason())));
        break;
    default:
        break;
    }
}

void Session::onCaptureBegin(const pb::CaptureBegin &begin)
{
    if (begin.capture_id() != captureId_)
        return;

    samplerate_ = begin.samplerate();
    segmentsReady_ = false;
    streams_.clear();

    for (const auto &stream : begin.streams()) {
        StreamInfo info;
        info.logic = stream.has_logic();
        for (const auto &id : stream.channels())
            info.channelIds.append(QString::fromStdString(id));

        if (info.logic) {
            info.unitsize = static_cast<int>(stream.logic().unitsize());
        } else {
            const auto &format = stream.analog();
            info.dtype = dtypeOf(format.type());
            info.scale = format.scale();
            info.offset = format.offset();
            info.unit = QString::fromStdString(format.unit());
        }
        streams_.insert(stream.id(), info);
    }
}

void Session::onAcquisitionBegin(const pb::AcquisitionBegin &begin)
{
    if (begin.capture_id() != captureId_)
        return;

    // Each acquisition replaces the display: a re-arming scope produces one
    // per trigger, a streaming device exactly one.
    QList<data::Capture::ChannelSpec> specs;
    int analogOrd = 0;
    for (const auto &info : streams_) {
        int bit = 0;
        for (const QString &id : info.channelIds) {
            if (info.logic)
                specs.append({id, id, data::SignalKind::Logic, bit++});
            else
                specs.append({id, id, data::SignalKind::Analog, analogOrd++});
        }
    }

    capture_->beginCapture(samplerate_, begin.t0(), specs);

    // One Segment per Signal: sibling logic channels each hold the same
    // packed bytes and read only their own bit out of them.
    for (const auto &info : streams_) {
        for (const QString &id : info.channelIds) {
            auto *signal = capture_->signalById(id);
            if (!signal)
                continue;
            data::Segment *segment = nullptr;
            if (info.logic) {
                segment = new data::LogicSegment(info.unitsize,
                                                 info.channelIds.size(), signal);
            } else {
                segment = new data::AnalogSegment(info.dtype, info.scale,
                                                  info.offset, info.unit, signal);
            }
            segment->setSamplerate(samplerate_);
            signal->appendSegment(segment);
        }
    }

    segmentsReady_ = true;
    capture_->markCapturing();
}

void Session::onData(const pb::CaptureData &data)
{
    if (data.capture_id() != captureId_ || !segmentsReady_)
        return;

    auto it = streams_.find(data.stream());
    if (it == streams_.end())
        return;
    const StreamInfo &info = *it;

    const std::size_t unitsize =
        info.logic ? static_cast<std::size_t>(info.unitsize)
                   : static_cast<std::size_t>(data::bytesPerSample(info.dtype));

    QByteArray packed;
    try {
        std::string scratch;
        const auto view = ::openmso::encoding::decodePayload(data, unitsize, scratch);
        packed = QByteArray(view.data, static_cast<qsizetype>(view.size));
    } catch (const ::openmso::Error &e) {
        emit deviceError(QString::fromUtf8(e.what()));
        return;
    }

    const auto firstSample = static_cast<qint64>(data.first_sample());
    const auto nsamples = static_cast<qint64>(data.sample_count());

    for (const QString &id : info.channelIds) {
        auto *signal = capture_->signalById(id);
        if (!signal)
            continue;
        auto *segment = signal->primarySegment();
        if (info.logic) {
            if (auto *l = qobject_cast<data::LogicSegment *>(segment))
                l->appendChunk(packed, firstSample, nsamples);
        } else {
            if (auto *a = qobject_cast<data::AnalogSegment *>(segment))
                a->appendChunk(packed, firstSample, nsamples);
        }
    }
    capture_->notifyAppend(data.stream(), firstSample, nsamples);
}

void Session::onTrigger(const pb::CaptureTrigger &trigger)
{
    if (trigger.capture_id() == captureId_)
        capture_->setTriggerSample(static_cast<qint64>(trigger.sample()));
}

void Session::onCaptureEnd(const pb::CaptureEnd &end)
{
    if (end.capture_id() != captureId_)
        return;
    if (end.has_error())
        capture_->endCapture(false, QString::fromStdString(end.error().message()));
    else
        capture_->endCapture(true);
}

} // namespace openmso::ui
