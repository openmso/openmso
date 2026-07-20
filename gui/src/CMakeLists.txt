# Data model library. Pure Qt6 Core — no Python, no libsigrok, no view
# layer. Per docs/gui-plan/04-architecture.md layering rule 3.

add_library(openmso-data STATIC
    data/Capture.cpp
    data/Capture.h
    data/EdgeIndex.cpp
    data/EdgeIndex.h
    data/Envelope.cpp
    data/Envelope.h
    data/LogicSegment.cpp
    data/LogicSegment.h
    data/AnalogSegment.cpp
    data/AnalogSegment.h
    data/Segment.cpp
    data/Segment.h
    data/Signal.cpp
    data/Signal.h
    data/Types.h
)

target_include_directories(openmso-data PUBLIC
    ${CMAKE_CURRENT_SOURCE_DIR}
)

target_link_libraries(openmso-data PUBLIC
    Qt6::Core
    Qt6::Gui
)

set_target_properties(openmso-data PROPERTIES
    POSITION_INDEPENDENT_CODE ON
)

# View library. Depends on data/ and Qt6 Gui/Widgets. Never calls ocp/
# or ui/ (layering rule 2 in 04-architecture.md).

add_library(openmso-view STATIC
    view/AnalogSignalTrace.cpp
    view/AnalogSignalTrace.h
    view/Header.cpp
    view/Header.h
    view/LogicSignalTrace.cpp
    view/LogicSignalTrace.h
    view/Ruler.cpp
    view/Ruler.h
    view/SignalTrace.cpp
    view/SignalTrace.h
    view/Trace.cpp
    view/Trace.h
    view/TraceView.cpp
    view/TraceView.h
    view/Viewport.cpp
    view/Viewport.h
    view/ViewState.h
)

target_include_directories(openmso-view PUBLIC
    ${CMAKE_CURRENT_SOURCE_DIR}
)

target_link_libraries(openmso-view PUBLIC
    openmso-data
    Qt6::Core
    Qt6::Gui
    Qt6::Widgets
)

set_target_properties(openmso-view PROPERTIES
    POSITION_INDEPENDENT_CODE ON
)

# OCP client library (added in M1).

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
    ui/Session.cpp
    ui/Session.h
)

target_include_directories(openmso-gui PRIVATE
    ${CMAKE_CURRENT_SOURCE_DIR}
)

target_compile_definitions(openmso-gui PRIVATE
    OPENMSO_PLUGINS_DIR="${OPENMSO_PLUGINS_DIR}"
)

target_link_libraries(openmso-gui PRIVATE
    openmso-data
    openmso-view
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
