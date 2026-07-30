cmake_minimum_required(VERSION 3.21)
project(omso VERSION 0.1.0 LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)

set(CMAKE_AUTOMOC ON)
set(CMAKE_AUTORCC ON)
set(CMAKE_AUTOUIC ON)

find_package(Qt6 6.4 REQUIRED COMPONENTS Core Gui Widgets Test Concurrent)

qt_standard_project_setup()

include(FetchContent)

# Tracks main until openmso-api tags a v1 release; -DOPENMSO_API_REF=<sha> to
# pin. To build against a local checkout instead, configure with
# -DFETCHCONTENT_SOURCE_DIR_OPENMSO_API=<path to the openmso-api checkout>.
set(OPENMSO_API_REF "main" CACHE STRING "openmso-api git ref to build against")

# Not GIT_SHALLOW: a shallow fetch cannot always resolve a raw commit SHA, and
# pinning one is the whole point of OPENMSO_API_REF.
FetchContent_Declare(openmso-api
    GIT_REPOSITORY https://github.com/openmso/openmso-api.git
    GIT_TAG ${OPENMSO_API_REF}
    SOURCE_SUBDIR cpp
)
FetchContent_MakeAvailable(openmso-api)

# Plugins directory: defaults to <repo>/plugins (developer mode).
# Overridable via -DOPENMSO_PLUGINS_DIR=... at configure time.
if(NOT DEFINED OPENMSO_PLUGINS_DIR)
    set(OPENMSO_PLUGINS_DIR "${CMAKE_SOURCE_DIR}/../plugins")
endif()
get_filename_component(OPENMSO_PLUGINS_DIR
    "${OPENMSO_PLUGINS_DIR}" ABSOLUTE)

enable_testing()

add_subdirectory(src)
add_subdirectory(tests)
