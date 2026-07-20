#pragma once

#include <QMainWindow>

class QToolBar;
class QLabel;

namespace openmso::ui {

// Skeleton main window for the M0 milestone.
//
// Per docs/gui-plan/09-ui-layout.md the final window has a menu bar,
// a main toolbar, a central TraceView, two docks (log + decoder picker),
// and a three-field status bar. At M0 only the chrome exists: an empty
// central widget, placeholder toolbar actions, the menu skeleton, and
// an idle status indicator. Real widgets land in M1 (TraceView in M2,
// docks in M5/M6).
class MainWindow : public QMainWindow {
    Q_OBJECT
public:
    explicit MainWindow(QWidget *parent = nullptr);
    ~MainWindow() override;

protected:
    void closeEvent(QCloseEvent *event) override;

private:
    void buildMenus();
    void buildToolBar();
    void buildStatusBar();

    // Owned by Qt's child-object tree; raw pointers are fine here.
    QToolBar *toolbar_ = nullptr;
    QLabel *statusState_ = nullptr;
    QLabel *statusDevice_ = nullptr;
    QLabel *statusCursor_ = nullptr;
};

} // namespace openmso::ui
