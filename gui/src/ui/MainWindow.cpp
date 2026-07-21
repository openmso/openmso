#include "MainWindow.h"

#include "view/TraceView.h"
#include "data/Capture.h"
#include "ocp/PluginManifest.h"

#include <QAction>
#include <QApplication>
#include <QCloseEvent>
#include <QComboBox>
#include <QCoreApplication>
#include <QFileInfo>
#include <QIcon>
#include <QLabel>
#include <QMenu>
#include <QMenuBar>
#include <QMessageBox>
#include <QStatusBar>
#include <QToolBar>
#include <QWidget>

#include <cmath>

namespace openmso::ui {

MainWindow::MainWindow(QWidget *parent)
    : QMainWindow(parent)
{
    setWindowTitle(tr("OpenMSO"));
    resize(1280, 720);

    // Default plugins dir: <exe_dir>/../plugins (developer mode) or
    // env override. Tests override via setPluginsDir().
    if (qEnvironmentVariableIsSet("OPENMSO_PLUGINS_DIR")) {
        pluginsDir_ = qEnvironmentVariable("OPENMSO_PLUGINS_DIR");
    } else {
        const QString exeDir = QCoreApplication::applicationDirPath();
        const QString candidate = QFileInfo(exeDir + "/../plugins").absoluteFilePath();
        if (QFileInfo::exists(candidate + "/demo"))
            pluginsDir_ = candidate;
    }

    traceView_ = new view::TraceView(this);
    setCentralWidget(traceView_);

    buildActions();
    buildMenus();
    buildToolBar();
    buildStatusBar();
    updateToolbarState();
}

MainWindow::~MainWindow() = default;

void MainWindow::closeEvent(QCloseEvent *event)
{
    if (session_) {
        session_->disconnectFromPlugin();
    }
    event->accept();
}

void MainWindow::buildActions()
{
    // One QAction per command, shared between the menu and the toolbar
    // so their enabled-state and triggers never diverge.
    connectAction_ = new QAction(tr("Connect"), this);
    connect(connectAction_, &QAction::triggered, this, &MainWindow::onConnect);

    startAction_ = new QAction(tr("Start"), this);
    startAction_->setShortcut(QKeySequence(Qt::Key_Space));
    startAction_->setIcon(QIcon::fromTheme(QStringLiteral("media-playback-start")));
    connect(startAction_, &QAction::triggered, this, &MainWindow::onStart);

    stopAction_ = new QAction(tr("Stop"), this);
    stopAction_->setIcon(QIcon::fromTheme(QStringLiteral("media-playback-stop")));
    connect(stopAction_, &QAction::triggered, this, &MainWindow::onStop);

    saveAction_ = new QAction(tr("Save…"), this);
    saveAction_->setShortcut(QKeySequence::Save);
    saveAction_->setEnabled(false);   // wired up in a later milestone
}

void MainWindow::buildMenus()
{
    auto *bar = menuBar();

    auto *fileMenu = bar->addMenu(tr("&File"));
    fileMenu->addAction(saveAction_);
    fileMenu->addSeparator();
    fileMenu->addAction(tr("Quit"), QKeySequence::Quit, this, &QWidget::close);

    auto *captureMenu = bar->addMenu(tr("&Capture"));
    captureMenu->addAction(startAction_);
    captureMenu->addAction(stopAction_);
    captureMenu->addSeparator();
    captureMenu->addAction(tr("Configure device…"));

    auto *viewMenu = bar->addMenu(tr("&View"));
    viewMenu->addAction(tr("Zoom in"), QKeySequence(Qt::Key_Plus),
                        traceView_, &view::TraceView::zoomIn);
    viewMenu->addAction(tr("Zoom out"), QKeySequence(Qt::Key_Minus),
                        traceView_, &view::TraceView::zoomOut);
    viewMenu->addAction(tr("Fit to window"), QKeySequence(Qt::Key_0),
                        traceView_, &view::TraceView::fitToData);
    viewMenu->addAction(tr("Toggle cursors"), QKeySequence(Qt::Key_C),
                        traceView_, &view::TraceView::toggleCursors);

    auto *helpMenu = bar->addMenu(tr("&Help"));
    helpMenu->addAction(tr("About OpenMSO"), this, [this]{
        QMessageBox::about(this, tr("OpenMSO"),
            tr("OpenMSO GUI v0.1.0\nGPL-3.0-or-later"));
    });
    helpMenu->addAction(tr("About Qt"), qApp, &QApplication::aboutQt);
}

void MainWindow::buildToolBar()
{
    toolbar_ = addToolBar(tr("Main"));
    toolbar_->setObjectName("mainToolbar");
    toolbar_->setMovable(false);
    toolbar_->setToolButtonStyle(Qt::ToolButtonTextBesideIcon);

    // Plugin picker.
    pluginPicker_ = new QComboBox(toolbar_);
    pluginPicker_->setMinimumWidth(140);
    // Populate from findPlugins.
    QString dir = pluginsDir_;
    if (dir.isEmpty()) dir = QStringLiteral(OPENMSO_PLUGINS_DIR);
    const auto plugins = openmso::ocp::findPlugins(dir);
    for (const auto &p : plugins)
        pluginPicker_->addItem(p.name, p.name);
    if (pluginPicker_->count() == 0)
        pluginPicker_->addItem(tr("(no plugins found)"), QString());
    toolbar_->addWidget(pluginPicker_);

    toolbar_->addAction(connectAction_);
    toolbar_->addSeparator();
    toolbar_->addAction(startAction_);
    toolbar_->addAction(stopAction_);
    toolbar_->addSeparator();
    toolbar_->addAction(saveAction_);
}

void MainWindow::buildStatusBar()
{
    auto *bar = statusBar();
    statusState_ = new QLabel(tr("● idle"), this);
    statusDevice_ = new QLabel(tr("no device"), this);
    statusCursor_ = new QLabel(QString(), this);
    statusState_->setMinimumWidth(80);
    statusDevice_->setMinimumWidth(220);
    bar->addWidget(statusState_);
    bar->addWidget(statusDevice_);
    bar->addWidget(statusCursor_, 1);
}

void MainWindow::updateToolbarState()
{
    const bool connected = session_ && session_->client();
    const bool capturing = session_ && session_->capture()
        && session_->capture()->state() == data::Capture::State::Capturing;
    startAction_->setEnabled(connected && !capturing);
    stopAction_->setEnabled(capturing);
    connectAction_->setText(connected ? tr("Disconnect") : tr("Connect"));
}

void MainWindow::onConnect()
{
    if (session_ && session_->client()) {
        // Disconnect.
        session_->disconnectFromPlugin();
        delete session_;
        session_ = nullptr;
        traceView_->setCapture(nullptr);
        statusDevice_->setText(tr("no device"));
        updateToolbarState();
        return;
    }

    session_ = new Session(this);
    traceView_->setCapture(session_->capture());
    connect(session_, &Session::deviceReady,
            this, &MainWindow::onDeviceReady);
    connect(session_, &Session::deviceError,
            this, &MainWindow::onDeviceError);
    connect(session_->capture(), &data::Capture::stateChanged,
            this, &MainWindow::onCaptureStateChanged);
    connect(traceView_, &view::TraceView::cursorMoved,
            this, &MainWindow::onCursorMoved);

    QString dir = pluginsDir_;
    if (dir.isEmpty()) dir = QStringLiteral(OPENMSO_PLUGINS_DIR);
    const QString pluginName = pluginPicker_->currentData().toString();
    if (pluginName.isEmpty()) {
        onDeviceError(tr("No plugin selected"));
        return;
    }

    if (pluginName == "demo") {
        if (!session_->connectDemo(dir)) {
            // deviceError already emitted.
        }
    } else {
        onDeviceError(tr("Plugin '%1' not wired up yet; v0.1 ships demo only.")
                          .arg(pluginName));
    }
    updateToolbarState();
}

void MainWindow::onStart()
{
    if (!session_) return;
    session_->startCapture();
    updateToolbarState();
}

void MainWindow::onStop()
{
    if (!session_) return;
    session_->stopCapture();
    updateToolbarState();
}

void MainWindow::onCursorMoved(double a, double b)
{
    QString text;
    if (a >= 0 && b >= 0) {
        const double dt = std::abs(b - a);
        const double freq = dt > 0 ? 1.0 / dt : 0.0;
        text = tr("Δt=%1 s   f=%2 Hz").arg(dt, 0, 'g', 4).arg(freq, 0, 'g', 4);
    }
    statusCursor_->setText(text);
}

void MainWindow::onDeviceReady(const QString &summary)
{
    statusDevice_->setText(summary);
    updateToolbarState();
}

void MainWindow::onDeviceError(const QString &msg)
{
    statusState_->setText(tr("● error"));
    QMessageBox::warning(this, tr("OpenMSO"), msg);
    if (session_) {
        delete session_;
        session_ = nullptr;
        traceView_->setCapture(nullptr);
    }
    updateToolbarState();
}

void MainWindow::onCaptureStateChanged(data::Capture::State s)
{
    using S = data::Capture::State;
    switch (s) {
    case S::Idle:       statusState_->setText(tr("● idle")); break;
    case S::Arming:     statusState_->setText(tr("● arming")); break;
    case S::Capturing:  statusState_->setText(tr("● capturing")); break;
    case S::Complete:   statusState_->setText(tr("● complete")); break;
    case S::Error:      statusState_->setText(tr("● error")); break;
    }
    updateToolbarState();
}

} // namespace openmso::ui
