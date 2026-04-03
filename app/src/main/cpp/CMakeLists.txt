cmake_minimum_required(VERSION 3.22.1)
project(silentdisco)

find_package(oboe REQUIRED CONFIG)

add_library(silentdisco SHARED native-lib.cpp)

target_link_libraries(
    silentdisco
    oboe::oboe
    android
    log
)
