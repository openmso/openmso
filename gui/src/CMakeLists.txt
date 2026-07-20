qt_add_executable(openmso-gui
    main.cpp
    ui/MainWindow.cpp
    ui/MainWindow.h
)

target_include_directories(openmso-gui PRIVATE
    ${CMAKE_CURRENT_SOURCE_DIR}
)

target_link_libraries(openmso-gui PRIVATE
    Qt6::Core
    Qt6::Gui
    Qt6::Widgets
    Qt6::Network
)

set_target_properties(openmso-gui PROPERTIES
    WIN32_EXECUTABLE ON
    MACOSX_BUNDLE ON
)

install(TARGETS openmso-gui
    RUNTIME DESTINATION bin
    BUNDLE DESTINATION bin
)
