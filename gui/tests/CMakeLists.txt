add_executable(tst_mainwindow
    tst_mainwindow.cpp
    ../src/ui/MainWindow.cpp
    ../src/ui/MainWindow.h
)

target_include_directories(tst_mainwindow PRIVATE
    ${CMAKE_CURRENT_SOURCE_DIR}/../src
)

target_link_libraries(tst_mainwindow PRIVATE
    Qt6::Core
    Qt6::Gui
    Qt6::Widgets
    Qt6::Network
    Qt6::Test
)

# QTEST_MAIN() generates the test entry point; CTest picks it up via
# add_test. qt_add_test would do the same but is not exposed by every
# Qt6 packaging (e.g. Debian's split), so this form is more portable.
add_test(NAME tst_mainwindow COMMAND tst_mainwindow)

# Headless on CI / dev boxes without a display server. The offscreen
# QPA plugin ships with qt6-base.
set_tests_properties(tst_mainwindow PROPERTIES
    ENVIRONMENT "QT_QPA_PLATFORM=offscreen"
)
