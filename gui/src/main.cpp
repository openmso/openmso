#include "ui/MainWindow.h"

#include <QApplication>

int main(int argc, char *argv[])
{
    QApplication app(argc, argv);
    QApplication::setApplicationName("openmso-gui");
    QApplication::setApplicationVersion("0.1.0");
    QApplication::setOrganizationName("OpenMSO");

    openmso::ui::MainWindow window;
    window.show();

    return app.exec();
}
