#include "PluginClient.h"

#include <QDebug>

namespace openmso::ocp {

namespace {

// Short enough that stop() is noticed promptly, long enough not to spin.
constexpr std::chrono::milliseconds READER_POLL{200};

std::vector<std::string> toArgv(const QStringList &list)
{
    std::vector<std::string> out;
    out.reserve(list.size());
    for (const QString &s : list)
        out.push_back(s.toStdString());
    return out;
}

} // namespace

EventReader::EventReader(::openmso::EventStream stream)
    : stream_(std::move(stream)) {}

void EventReader::run()
{
    try {
        stream_.setTimeout(READER_POLL);
        while (!stop_) {
            try {
                emit event(stream_.next());
            } catch (const ::openmso::Error &e) {
                if (e.kind() == ::openmso::ErrorKind::Nng && !stop_)
                    continue;  // poll timeout
                throw;
            }
        }
    } catch (const std::exception &e) {
        if (!stop_)
            emit failed(QString::fromUtf8(e.what()));
    }
    emit finished();
}

PluginClient::PluginClient(::openmso::CaptureClient client, QObject *parent)
    : QObject(parent),
      client_(std::make_unique<::openmso::CaptureClient>(std::move(client)))
{
    startReader();
}

PluginClient *PluginClient::launch(const PluginManifest &manifest,
                                   const QString &device, QString *error,
                                   QObject *parent)
{
    qRegisterMetaType<::openmso::pb::Event>();
    try {
        auto client = ::openmso::CaptureClient::launch(toArgv(manifest.argv),
                                                       device.toStdString());
        return new PluginClient(std::move(client), parent);
    } catch (const std::exception &e) {
        if (error)
            *error = QString::fromUtf8(e.what());
        return nullptr;
    }
}

void PluginClient::startReader()
{
    thread_ = new QThread(this);
    reader_ = new EventReader(client_->eventStream());
    reader_->moveToThread(thread_);

    connect(thread_, &QThread::started, reader_, &EventReader::run);
    connect(reader_, &EventReader::event, this, &PluginClient::event);
    connect(reader_, &EventReader::failed, this, &PluginClient::streamFailed);
    connect(reader_, &EventReader::finished, thread_, &QThread::quit);
    connect(thread_, &QThread::finished, reader_, &QObject::deleteLater);

    thread_->start();
}

PluginClient::~PluginClient()
{
    shutdown();
}

void PluginClient::shutdown()
{
    // The reader has to go first: it holds a borrowed handle on the event
    // socket, which CaptureClient closes as it dies.
    if (thread_) {
        if (reader_)
            reader_->stop();
        thread_->quit();
        thread_->wait();
        thread_ = nullptr;
        reader_ = nullptr;
    }

    if (client_) {
        try {
            client_->shutdown();
        } catch (const std::exception &e) {
            qWarning("plugin shutdown: %s", e.what());
        }
        client_.reset();
    }
}

bool PluginClient::isRunning() const
{
    return client_ && client_->isRunning();
}

::openmso::pb::HelloResult PluginClient::hello(const QString &clientName,
                                               const QString &clientVersion)
{
    return client_->hello(clientName.toStdString(), clientVersion.toStdString());
}

::openmso::pb::Description PluginClient::describe()
{
    return client_->describe();
}

::openmso::pb::Config PluginClient::getConfig()
{
    return client_->getConfig();
}

::openmso::pb::Config PluginClient::setConfig(const ::openmso::pb::Config &config)
{
    return client_->setConfig(config);
}

quint64 PluginClient::acquireStart(::openmso::pb::AcquireMode mode)
{
    const std::uint64_t id = client_->nextCaptureId();
    client_->acquireStart(id, mode);
    return id;
}

void PluginClient::acquireStop(quint64 captureId)
{
    client_->acquireStop(captureId);
}

void PluginClient::reset()
{
    client_->reset();
}

} // namespace openmso::ocp
