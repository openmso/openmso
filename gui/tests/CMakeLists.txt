# Inject the repo's plugins/ directory so tests can find the demo
# plugin regardless of where the build tree lives. The path is
# absolute and points at the source tree; CI dev boxes and source
# builds both work. Out-of-tree packagers can override
# -DOPENMSO_PLUGINS_DIR=... at configure time.
if(NOT DEFINED OPENMSO_PLUGINS_DIR)
    set(OPENMSO_PLUGINS_DIR "${CMAKE_SOURCE_DIR}/../plugins")
endif()
get_filename_component(OPENMSO_PLUGINS_DIR
    "${OPENMSO_PLUGINS_DIR}" ABSOLUTE)

# --- tst_mainwindow: M0 chrome test -------------------------------------

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
