# OCP client library. Linked into openmso-gui and the tests.
# Pure Qt6 — no Python, no libsigrok (per docs/gui-plan/10-build-system.md).

add_library(openmso-ocp STATIC
    ocp/MessageStream.cpp
    ocp/MessageStream.h
    ocp/PluginClient.cpp
    ocp/PluginClient.h
    ocp/PluginError.cpp
    ocp/PluginError.h
    ocp/PluginManifest.cpp
    ocp/PluginManifest.h
)

target_include_directories(openmso-ocp PUBLIC
    ${CMAKE_CURRENT_SOURCE_DIR}
)

target_link_libraries(openmso-ocp PUBLIC
    Qt6::Core
    Qt6::Network
)

set_target_properties(openmso-ocp PROPERTIES
    POSITION_INDEPENDENT_CODE ON
)

# --- main executable -----------------------------------------------------

qt_add_executable(openmso-gui
    main.cpp
    ui/MainWindow.cpp
    ui/MainWindow.h
)

target_include_directories(openmso-gui PRIVATE
    ${CMAKE_CURRENT_SOURCE_DIR}
)

target_link_libraries(openmso-gui PRIVATE
    openmso-ocp
    Qt6::Core
    Qt6::Gui
    Qt6::Widgets
)

set_target_properties(openmso-gui PROPERTIES
    WIN32_EXECUTABLE ON
    MACOSX_BUNDLE ON
)

install(TARGETS openmso-gui
    RUNTIME DESTINATION bin
    BUNDLE DESTINATION bin
)
