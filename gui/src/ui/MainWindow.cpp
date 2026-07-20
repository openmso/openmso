#include "MainWindow.h"

#include <QAction>
#include <QCloseEvent>
#include <QLabel>
#include <QMenu>
#include <QMenuBar>
#include <QStatusBar>
#include <QToolBar>
#include <QWidget>

namespace openmso::ui {

MainWindow::MainWindow(QWidget *parent)
    : QMainWindow(parent)
{
    setWindowTitle(tr("OpenMSO"));
    resize(1280, 720);

    // Placeholder central widget. The real TraceView lands in M2.
    auto *placeholder = new QWidget(this);
    placeholder->setObjectName("tracePlaceholder");
    setCentralWidget(placeholder);

    buildMenus();
    buildToolBar();
    buildStatusBar();
}

MainWindow::~MainWindow() = default;

void MainWindow::closeEvent(QCloseEvent *event)
{
    event->accept();
}

void MainWindow::buildMenus()
{
    // Menu skeleton per docs/gui-plan/09-ui-layout.md. Actions are
    // created without handlers at M0; they are wired up in later
    // milestones.
    auto *bar = menuBar();

    auto *fileMenu = bar->addMenu(tr("&File"));
    fileMenu->addAction(tr("New session"));
    fileMenu->addAction(tr("Open .sr…"));
    fileMenu->addAction(tr("Save As…"));
    fileMenu->addSeparator();
    fileMenu->addAction(tr("Quit"), QKeySequence::Quit, this, &QWidget::close);

    auto *captureMenu = bar->addMenu(tr("&Capture"));
    captureMenu->addAction(tr("Start"));
    captureMenu->addAction(tr("Stop"));
    captureMenu->addAction(tr("Snapshot"));
    captureMenu->addSeparator();
    captureMenu->addAction(tr("Configure device…"));
    captureMenu->addAction(tr("Add/Remove plugin path…"));

    auto *viewMenu = bar->addMenu(tr("&View"));
    viewMenu->addAction(tr("Zoom in"));
    viewMenu->addAction(tr("Zoom out"));
    viewMenu->addAction(tr("Fit"));
    viewMenu->addAction(tr("Go to trigger"));
    viewMenu->addSeparator();
    viewMenu->addAction(tr("Toggle cursors"));
    viewMenu->addAction(tr("Toggle ruler"));
    viewMenu->addAction(tr("Light/dark theme"));

    auto *decodeMenu = bar->addMenu(tr("&Decode"));
    decodeMenu->addAction(tr("Add decoder…"));
    decodeMenu->addAction(tr("Manage decoders"));
    decodeMenu->addAction(tr("Clear annotations"));

    auto *helpMenu = bar->addMenu(tr("&Help"));
    helpMenu->addAction(tr("About OpenMSO"));
    helpMenu->addAction(tr("About Qt"));
    helpMenu->addSeparator();
    helpMenu->addAction(tr("View logs"));
    helpMenu->addAction(tr("Report issue"));
}

void MainWindow::buildToolBar()
{
    toolbar_ = addToolBar(tr("Main"));
    toolbar_->setObjectName("mainToolbar");
    toolbar_->setMovable(false);

    // Placeholder actions matching the toolbar layout in
    // 09-ui-layout.md. They are not connected at M0.
    toolbar_->addAction(tr("Plugin"));
    toolbar_->addAction(tr("Connect…"));
    toolbar_->addAction(tr("Configure"));
    toolbar_->addSeparator();
    toolbar_->addAction(tr("Start"));
    toolbar_->addAction(tr("Stop"));
    toolbar_->addSeparator();
    toolbar_->addAction(tr("Mode"));
    toolbar_->addAction(tr("Trigger"));
    toolbar_->addSeparator();
    toolbar_->addAction(tr("Save…"));
    toolbar_->addAction(tr("Load .sr"));
    toolbar_->addAction(tr("Decoders"));
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
    bar->addWidget(statusCursor_, /*stretch=*/1);
}

} // namespace openmso::ui
