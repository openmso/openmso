#include <QCoreApplication>
#include <QDir>
#include <QElapsedTimer>
#include <QFileInfo>
#include <QSignalSpy>
#include <QTest>

#include "ocp/PluginClient.h"
#include "ocp/PluginManifest.h"

using openmso::ocp::PluginClient;
using openmso::ocp::PluginManifest;
using openmso::ocp::findPlugin;
namespace pb = openmso::pb;

// Skips rather than fails when the demo plugin is absent, so a CI matrix that
// only builds the GUI still runs the rest of the suite.
class TestPluginClient : public QObject {
    Q_OBJECT
private slots:
    void initTestCase();
    void helloAndDescribe();
    void singleCaptureDeliversEveryStream();
    void cleanup();

private:
    PluginClient *launch();

    QString pluginsDir_;
    PluginManifest demo_;
    PluginClient *client_ = nullptr;
};

void TestPluginClient::initTestCase()
{
    pluginsDir_ = QStringLiteral(OPENMSO_PLUGINS_DIR);
    if (!QDir(pluginsDir_).exists(QStringLiteral("demo")))
        QSKIP(qPrintable(QStringLiteral("demo plugin not found under %1")
                             .arg(pluginsDir_)));

    demo_ = findPlugin(pluginsDir_, QStringLiteral("demo"));
    QVERIFY2(!demo_.isNull(), "demo plugin.json could not be parsed");
    if (!QFileInfo::exists(demo_.argv.first()))
        QSKIP("demo plugin has not been built");
}

PluginClient *TestPluginClient::launch()
{
    QString error;
    client_ = PluginClient::launch(demo_, QStringLiteral("demo://0"), &error, this);
    if (!client_)
        QTest::qFail(qPrintable(error), __FILE__, __LINE__);
    return client_;
}

void TestPluginClient::cleanup()
{
    if (client_) {
        client_->shutdown();
        delete client_;
        client_ = nullptr;
    }
}

void TestPluginClient::helloAndDescribe()
{
    QVERIFY(launch());

    const auto hello = client_->hello(QStringLiteral("tst_pluginclient"),
                                      QStringLiteral("0.1"));
    QCOMPARE(hello.protocol(), openmso::PROTOCOL_VERSION);
    QCOMPARE(QString::fromStdString(hello.plugin().name()), QStringLiteral("demo"));
    QCOMPARE(QString::fromStdString(hello.device().model()), QStringLiteral("Demo MSO"));

    const auto description = client_->describe();
    int analog = 0, logic = 0;
    for (const auto &channel : description.channels()) {
        if (channel.kind() == pb::CHANNEL_ANALOG)
            ++analog;
        else if (channel.kind() == pb::CHANNEL_LOGIC)
            ++logic;
    }
    QCOMPARE(analog, 2);
    QCOMPARE(logic, 8);
}

void TestPluginClient::singleCaptureDeliversEveryStream()
{
    QVERIFY(launch());
    client_->hello(QStringLiteral("tst_pluginclient"), QStringLiteral("0.1"));

    pb::Config wanted;
    wanted.mutable_device()->set_sample_depth(2000);
    const auto settled = client_->setConfig(wanted);
    QCOMPARE(settled.device().sample_depth(), 2000u);

    QSignalSpy events(client_, &PluginClient::event);
    const quint64 captureId = client_->acquireStart(pb::ACQUIRE_SINGLE);
    QVERIFY(captureId > 0);

    // Events arrive queued from the reader thread, so the loop has to spin.
    QMap<quint32, quint64> samples;
    bool ended = false;
    QElapsedTimer timer;
    timer.start();
    while (!ended && timer.elapsed() < 20000) {
        QCoreApplication::processEvents(QEventLoop::WaitForMoreEvents, 100);
        while (!events.isEmpty()) {
            const auto event = events.takeFirst().at(0).value<pb::Event>();
            if (event.has_data()) {
                QCOMPARE(event.data().capture_id(), captureId);
                samples[event.data().stream()] += event.data().sample_count();
            } else if (event.has_capture_end()) {
                QVERIFY(!event.capture_end().has_error());
                ended = true;
            }
        }
    }

    QVERIFY2(ended, "capture never ended");
    QCOMPARE(samples.size(), 3);
    for (auto it = samples.begin(); it != samples.end(); ++it)
        QCOMPARE(it.value(), 2000u);
}

QTEST_MAIN(TestPluginClient)
#include "tst_pluginclient.moc"
