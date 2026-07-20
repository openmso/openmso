#include <QCoreApplication>
#include <QDir>
#include <QJsonObject>
#include <QJsonArray>
#include <QTest>

#include "ocp/PluginClient.h"
#include "ocp/PluginError.h"
#include "ocp/PluginManifest.h"

using openmso::ocp::PluginClient;
using openmso::ocp::PluginError;
using openmso::ocp::PluginManifest;
using openmso::ocp::findPlugin;

// M1 acceptance test per docs/gui-plan/11-milestones.md:
// "launch demo, call initialize + scan + describe, assert the demo's
//  two analog + eight logic channels come back."
//
// The plugin path is injected at configure time via the
// OPENMSO_PLUGINS_DIR macro (see CMakeLists.txt). The test skips
// (rather than fails) if the demo plugin is absent — this lets the
// test run on CI matrices that only build the GUI.
class TestPluginClient : public QObject {
    Q_OBJECT
private slots:
    void initTestCase();
    void initializeScanDescribe();
    void cleanupTestCase();

private:
    QString pluginsDir_;
    PluginManifest demo_;
    PluginClient *client_ = nullptr;
};

void TestPluginClient::initTestCase()
{
    pluginsDir_ = QStringLiteral(OPENMSO_PLUGINS_DIR);
    if (!QDir(pluginsDir_).exists(QStringLiteral("demo"))) {
        QSKIP(qPrintable(QStringLiteral(
            "demo plugin not found under %1 — skipping.").arg(pluginsDir_)));
    }
    demo_ = findPlugin(pluginsDir_, QStringLiteral("demo"));
    QVERIFY2(!demo_.name.isEmpty(), "demo plugin.json could not be parsed");
    QVERIFY2(!demo_.argv.isEmpty(), "demo plugin has no argv");
}

void TestPluginClient::initializeScanDescribe()
{
    client_ = PluginClient::launch(demo_, this);
    QVERIFY2(client_, "failed to launch demo plugin");

    // initialize
    QJsonObject initResult = client_->initialize(
        QStringLiteral("tst_pluginclient"), QStringLiteral("0.1"));
    QCOMPARE(initResult.value("protocol_version").toInt(), 0);
    const auto plugin = initResult.value("plugin").toObject();
    QCOMPARE(plugin.value("name").toString(), QStringLiteral("demo"));

    // scan
    QJsonObject scanResult =
        client_->request(QStringLiteral("scan"));
    const auto devices = scanResult.value("devices").toArray();
    QVERIFY2(!devices.isEmpty(), "demo returned no devices");
    const QString deviceId =
        devices.first().toObject().value("device_id").toString();
    QCOMPARE(deviceId, QStringLiteral("demo0"));

    // open
    client_->request(QStringLiteral("open"),
                     QJsonObject{{"device_id", deviceId}});

    // describe
    QJsonObject desc = client_->request(QStringLiteral("describe"));
    const auto channels = desc.value("channels").toArray();
    int analog = 0, logic = 0;
    for (const auto &v : channels) {
        const QString kind = v.toObject().value("kind").toString();
        if (kind == "analog") ++analog;
        else if (kind == "logic") ++logic;
    }
    QCOMPARE(analog, 2);
    QCOMPARE(logic, 8);
}

void TestPluginClient::cleanupTestCase()
{
    if (client_) {
        client_->shutdown();
        delete client_;
        client_ = nullptr;
    }
}

QTEST_MAIN(TestPluginClient)
#include "tst_pluginclient.moc"
