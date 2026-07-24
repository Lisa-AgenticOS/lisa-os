# fcitx5-lisa — Writing Tools layer 2 (PLAN §5.7.3, ADR-0007).
# Linux-only (needs Fcitx5Core); the protocol half (http.cpp) also
# builds standalone for unit tests — see tests/CMakeLists.txt and
# `just ime-test`.
cmake_minimum_required(VERSION 3.16)
project(fcitx5-lisa VERSION 0.1.0 LANGUAGES CXX)

# fcitx5 headers use std::span / string_view::starts_with — C++20 floor.
set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

include(GNUInstallDirs)
find_package(Fcitx5Core REQUIRED)
find_package(Fcitx5Utils REQUIRED)
# Header-only interface target for the dbus addon's public API
# (IDBusModule::bus) — the double-shift gesture's Summon call rides
# fcitx5's own session-bus connection.
find_package(Fcitx5Module REQUIRED COMPONENTS DBus)
find_package(Threads REQUIRED)

add_library(lisa MODULE src/lisa.cpp src/http.cpp src/doubleshift.cpp)
target_link_libraries(lisa Fcitx5::Core Fcitx5::Utils Fcitx5::Module::DBus
                      Threads::Threads)
set_target_properties(lisa PROPERTIES PREFIX "")

configure_file(lisa-addon.conf.in lisa-addon.conf @ONLY)

install(TARGETS lisa DESTINATION "${CMAKE_INSTALL_LIBDIR}/fcitx5")
install(FILES "${CMAKE_CURRENT_BINARY_DIR}/lisa-addon.conf"
        DESTINATION "${FCITX_INSTALL_PKGDATADIR}/addon"
        RENAME lisa.conf)

enable_testing()
add_subdirectory(tests)
