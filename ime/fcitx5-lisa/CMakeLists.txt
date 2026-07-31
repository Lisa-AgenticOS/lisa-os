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

# Fcitx5::Module::DBus exports only the innermost directory
# (.../Fcitx5/Module/fcitx-module/dbus), so the canonical include that
# fcitx5's own addons use — <fcitx-module/dbus/dbus_public.h> — does not
# resolve from it. Add the Module root, derived from the imported target
# rather than hardcoded, because distributions disagree about the prefix.
#
# This is not hypothetical tidying: without it the build fails with a
# bare "fcitx-module/dbus/dbus_public.h: No such file or directory"
# against Arch's fcitx5 5.1.21. It went unnoticed because the CI job
# that compiles this addon is path-filtered to ime/**, so it does not
# run when only fcitx5 moves underneath us.
get_target_property(_lisa_dbus_inc Fcitx5::Module::DBus
                    INTERFACE_INCLUDE_DIRECTORIES)
if(_lisa_dbus_inc)
    get_filename_component(_lisa_module_root "${_lisa_dbus_inc}/../.." ABSOLUTE)
    target_include_directories(lisa PRIVATE "${_lisa_module_root}")
endif()

configure_file(lisa-addon.conf.in lisa-addon.conf @ONLY)

install(TARGETS lisa DESTINATION "${CMAKE_INSTALL_LIBDIR}/fcitx5")
install(FILES "${CMAKE_CURRENT_BINARY_DIR}/lisa-addon.conf"
        DESTINATION "${FCITX_INSTALL_PKGDATADIR}/addon"
        RENAME lisa.conf)

enable_testing()
add_subdirectory(tests)
