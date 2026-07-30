# Utility library. Pure, testable helpers (color palette, time
# formatting) with no view/data dependencies. Per 04-architecture.md.

add_library(openmso-util STATIC
    util/ChannelColors.cpp
    util/ChannelColors.h
    util/TimeFormat.cpp
    util/TimeFormat.h
)

target_include_directories(openmso-util PUBLIC
    ${CMAKE_CURRENT_SOURCE_DIR}
)

target_link_libraries(openmso-util PUBLIC
    Qt6::Core
    Qt6::Gui
)

set_target_properties(openmso-util PROPERTIES
    POSITION_INDEPENDENT_CODE ON
)

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

# Measurement library. Pure analysis over data/ segments — no view, no
# Qt Widgets. Feeds the measurement dock (and later derived channels).

add_library(openmso-measure STATIC
    measure/Measure.cpp
    measure/Measure.h
    measure/Schmitt.cpp
    measure/Schmitt.h
)

target_include_directories(openmso-measure PUBLIC
    ${CMAKE_CURRENT_SOURCE_DIR}
)

target_link_libraries(openmso-measure PUBLIC
    openmso-data
    Qt6::Core
)

set_target_properties(openmso-measure PROPERTIES
    POSITION_INDEPENDENT_CODE ON
)

# View library. Depends on data/ and Qt6 Gui/Widgets. Never calls ocp/
# or ui/ (layering rule 2 in 04-architecture.md).

add_library(openmso-view STATIC
    view/AnalogSignalTrace.cpp
    view/AnalogSignalTrace.h
    view/ChannelModel.cpp
    view/ChannelModel.h
    view/DerivedChannel.cpp
    view/DerivedChannel.h
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
    view/ViewState.cpp
    view/ViewState.h
)

target_include_directories(openmso-view PUBLIC
    ${CMAKE_CURRENT_SOURCE_DIR}
)

target_link_libraries(openmso-view PUBLIC
    openmso-data
    openmso-util
    openmso-measure
    Qt6::Core
    Qt6::Gui
    Qt6::Widgets
    Qt6::Concurrent
)

set_target_properties(openmso-view PROPERTIES
    POSITION_INDEPENDENT_CODE ON
)

# OCP client library: Qt bindings over the openmso-api C++ library.

add_library(openmso-ocp STATIC
    ocp/PluginClient.cpp
    ocp/PluginClient.h
    ocp/PluginManifest.cpp
    ocp/PluginManifest.h
)

target_include_directories(openmso-ocp PUBLIC
    ${CMAKE_CURRENT_SOURCE_DIR}
)

target_link_libraries(openmso-ocp PUBLIC
    openmso::openmso
    Qt6::Core
)

set_target_properties(openmso-ocp PROPERTIES
    POSITION_INDEPENDENT_CODE ON
)

# --- main executable -----------------------------------------------------

qt_add_executable(omso
    main.cpp
    ui/DeriveLogicDialog.cpp
    ui/DeriveLogicDialog.h
    ui/MainWindow.cpp
    ui/MainWindow.h
    ui/MeasurementsPanel.cpp
    ui/MeasurementsPanel.h
    ui/Session.cpp
    ui/Session.h
)

target_include_directories(omso PRIVATE
    ${CMAKE_CURRENT_SOURCE_DIR}
)

target_compile_definitions(omso PRIVATE
    OPENMSO_PLUGINS_DIR="${OPENMSO_PLUGINS_DIR}"
)

target_link_libraries(omso PRIVATE
    openmso-data
    openmso-view
    openmso-measure
    openmso-ocp
    openmso-util
    Qt6::Core
    Qt6::Gui
    Qt6::Widgets
)

set_target_properties(omso PROPERTIES
    WIN32_EXECUTABLE ON
    MACOSX_BUNDLE ON
)

install(TARGETS omso
    RUNTIME DESTINATION bin
    BUNDLE DESTINATION bin
)
