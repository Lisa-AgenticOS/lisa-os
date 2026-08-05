# Protocol-half unit tests: no fcitx5 dependency, runs anywhere.
add_executable(http_test http_test.cpp ../src/http.cpp)
target_include_directories(http_test PRIVATE ../src)
target_link_libraries(http_test Threads::Threads)
add_test(NAME http_test COMMAND http_test)

# Double-tap-Shift state machine: same deal, pure standard C++.
add_executable(doubleshift_test doubleshift_test.cpp ../src/doubleshift.cpp)
target_include_directories(doubleshift_test PRIVATE ../src)
add_test(NAME doubleshift_test COMMAND doubleshift_test)

# The shipped notifications.conf keeps the fcitx5 Wayland-diagnose nag
# suppressed (#191/#201). No fcitx5 dependency: it reads the file the
# PKGBUILD installs and checks the marshalling form the suppression
# depends on. The path is passed explicitly so the test binary is not
# tied to the build directory's layout.
add_executable(notifications_test notifications_test.cpp)
target_compile_definitions(notifications_test PRIVATE
    SOURCE_DIR="${CMAKE_CURRENT_SOURCE_DIR}")
add_test(NAME notifications_test COMMAND notifications_test
    "${CMAKE_CURRENT_SOURCE_DIR}/../notifications.conf")
