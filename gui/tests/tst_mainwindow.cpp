#include <QLabel>
#include <QMainWindow>
#include <QStatusBar>
#include <QToolBar>
#include <QTest>

#include "ui/MainWindow.h"
using openmso::ui::MainWindow;

class TestMainWindow : public QObject {
    Q_OBJECT
private slots:
    void windowAppears();
    void hasPlaceholderToolbar();
    void statusBarShowsIdle();
};

// M0 user story: "I run openmso-gui and a window appears."
// Constructs the window, shows it, and verifies the chrome
// (central widget, toolbar, status bar) is in place.
void TestMainWindow::windowAppears()
{
    MainWindow w;
    QVERIFY(!w.isVisible());
    w.show();
    QVERIFY(QTest::qWaitForWindowExposed(&w));
    QVERIFY(w.isVisible());
    QVERIFY(w.centralWidget() != nullptr);
    QCOMPARE(w.windowTitle(), QStringLiteral("OpenMSO"));
}

void TestMainWindow::hasPlaceholderToolbar()
{
    MainWindow w;
    auto *tb = w.findChild<QToolBar *>("mainToolbar");
    QVERIFY(tb != nullptr);
    QVERIFY(!tb->actions().isEmpty());
}

void TestMainWindow::statusBarShowsIdle()
{
    MainWindow w;
    auto *sb = w.statusBar();
    QVERIFY(sb != nullptr);
    // The first label in the status bar is the state indicator.
    auto labels = sb->findChildren<QLabel *>();
    QVERIFY(!labels.isEmpty());
    QVERIFY(labels.first()->text().contains(QStringLiteral("idle"),
                                             Qt::CaseInsensitive));
}

QTEST_MAIN(TestMainWindow)
#include "tst_mainwindow.moc"
