# Protocol-half unit tests: no fcitx5 dependency, runs anywhere.
add_executable(http_test http_test.cpp ../src/http.cpp)
target_include_directories(http_test PRIVATE ../src)
target_link_libraries(http_test Threads::Threads)
add_test(NAME http_test COMMAND http_test)
