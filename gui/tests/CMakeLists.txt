# OPENMSO_PLUGINS_DIR is defined in the top-level CMakeLists.txt.

# --- tst_mainwindow: M0 chrome test -------------------------------------

add_executable(tst_mainwindow
    tst_mainwindow.cpp
    ../src/ui/MainWindow.cpp
    ../src/ui/MainWindow.h
    ../src/ui/Session.cpp
    ../src/ui/Session.h
)

target_include_directories(tst_mainwindow PRIVATE
    ${CMAKE_CURRENT_SOURCE_DIR}/../src
)

target_link_libraries(tst_mainwindow PRIVATE
    openmso-data
    openmso-view
    openmso-ocp
    Qt6::Core
    Qt6::Gui
    Qt6::Widgets
    Qt6::Network
    Qt6::Test
)

target_compile_definitions(tst_mainwindow PRIVATE
    OPENMSO_PLUGINS_DIR="${OPENMSO_PLUGINS_DIR}"
)

add_test(NAME tst_mainwindow COMMAND tst_mainwindow)
set_tests_properties(tst_mainwindow PROPERTIES
    ENVIRONMENT "QT_QPA_PLATFORM=offscreen"
)

# --- tst_pluginclient: M1 OCP client test -------------------------------

add_executable(tst_pluginclient
    tst_pluginclient.cpp
)

target_include_directories(tst_pluginclient PRIVATE
    ${CMAKE_CURRENT_SOURCE_DIR}/../src
)

target_compile_definitions(tst_pluginclient PRIVATE
    OPENMSO_PLUGINS_DIR="${OPENMSO_PLUGINS_DIR}"
)

target_link_libraries(tst_pluginclient PRIVATE
    openmso-ocp
    Qt6::Core
    Qt6::Network
    Qt6::Test
)

add_test(NAME tst_pluginclient COMMAND tst_pluginclient)
set_tests_properties(tst_pluginclient PROPERTIES
    ENVIRONMENT "QT_QPA_PLATFORM=offscreen"
)

# --- tst_capture: M2 data model test ------------------------------------

add_executable(tst_capture
    tst_capture.cpp
)

target_include_directories(tst_capture PRIVATE
    ${CMAKE_CURRENT_SOURCE_DIR}/../src
)

target_link_libraries(tst_capture PRIVATE
    openmso-data
    Qt6::Core
    Qt6::Test
)

add_test(NAME tst_capture COMMAND tst_capture)
set_tests_properties(tst_capture PROPERTIES
    ENVIRONMENT "QT_QPA_PLATFORM=offscreen"
)

# --- tst_view: M2 view paint test ---------------------------------------

add_executable(tst_view
    tst_view.cpp
)

target_include_directories(tst_view PRIVATE
    ${CMAKE_CURRENT_SOURCE_DIR}/../src
)

target_link_libraries(tst_view PRIVATE
    openmso-data
    openmso-view
    Qt6::Core
    Qt6::Gui
    Qt6::Widgets
    Qt6::Test
)

add_test(NAME tst_view COMMAND tst_view)
set_tests_properties(tst_view PROPERTIES
    ENVIRONMENT "QT_QPA_PLATFORM=offscreen"
)

# --- m2_smoke: end-to-end demo plugin → capture → render ---------------

qt_add_executable(m2_smoke
    m2_smoke.cpp
    ../src/ui/MainWindow.cpp
    ../src/ui/Session.cpp
)
target_include_directories(m2_smoke PRIVATE
    ${CMAKE_CURRENT_SOURCE_DIR}/../src
)
target_compile_definitions(m2_smoke PRIVATE
    OPENMSO_PLUGINS_DIR="${OPENMSO_PLUGINS_DIR}"
)
target_link_libraries(m2_smoke PRIVATE
    openmso-data
    openmso-view
    openmso-ocp
    Qt6::Core
    Qt6::Gui
    Qt6::Widgets
    Qt6::Network
)
add_test(NAME m2_smoke COMMAND m2_smoke)
set_tests_properties(m2_smoke PROPERTIES
    ENVIRONMENT "QT_QPA_PLATFORM=offscreen"
)
