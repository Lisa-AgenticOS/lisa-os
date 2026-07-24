# Protocol-half unit tests: no fcitx5 dependency, runs anywhere.
add_executable(http_test http_test.cpp ../src/http.cpp)
target_include_directories(http_test PRIVATE ../src)
target_link_libraries(http_test Threads::Threads)
add_test(NAME http_test COMMAND http_test)

# Double-tap-Shift state machine: same deal, pure standard C++.
add_executable(doubleshift_test doubleshift_test.cpp ../src/doubleshift.cpp)
target_include_directories(doubleshift_test PRIVATE ../src)
add_test(NAME doubleshift_test COMMAND doubleshift_test)
