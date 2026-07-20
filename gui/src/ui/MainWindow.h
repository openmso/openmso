#pragma once

#include <QMainWindow>

#include "ui/Session.h"

class QToolBar;
class QComboBox;
class QAction;
class QLabel;

namespace openmso::view { class TraceView; }

namespace openmso::ui {

// Main window. Owns the Session (plugin client + capture) and the
// TraceView. Per docs/gui-plan/09-ui-layout.md.
class MainWindow : public QMainWindow {
    Q_OBJECT
public:
    explicit MainWindow(QWidget *parent = nullptr);
    ~MainWindow() override;

    // For tests: set the plugins directory used for demo discovery.
    void setPluginsDir(const QString &dir) { pluginsDir_ = dir; }

protected:
    void closeEvent(QCloseEvent *event) override;

private slots:
    void onConnect();
    void onStart();
    void onStop();
    void onCursorMoved(double a, double b);
    void onDeviceReady(const QString &summary);
    void onDeviceError(const QString &msg);
    void onCaptureStateChanged(openmso::data::Capture::State s);

private:
    void buildMenus();
    void buildToolBar();
    void buildStatusBar();
    void updateToolbarState();

    // Chrome.
    QToolBar *toolbar_ = nullptr;
    QComboBox *pluginPicker_ = nullptr;
    QAction *connectAction_ = nullptr;
    QAction *startAction_ = nullptr;
    QAction *stopAction_ = nullptr;
    QLabel *statusState_ = nullptr;
    QLabel *statusDevice_ = nullptr;
    QLabel *statusCursor_ = nullptr;

    // Core.
    view::TraceView *traceView_ = nullptr;
    Session *session_ = nullptr;
    QString pluginsDir_;
};

} // namespace openmso::ui
