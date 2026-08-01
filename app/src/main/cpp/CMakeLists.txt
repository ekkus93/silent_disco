cmake_minimum_required(VERSION 3.22.1)
project(silentdisco)

find_package(oboe REQUIRED CONFIG)

add_library(silentdisco SHARED
    native-lib.cpp
    OboeOutputAdapter.cpp
    OboeJniBridge.cpp
)

# The narrow real-time C ABI's declarations only; its symbols are resolved at
# runtime via dlopen/dlsym against the already-packaged Rust cdylib (see
# OboeOutputAdapter.cpp), not linked at build time, so this include has no
# build-order dependency on cargo-ndk's own build task.
target_include_directories(silentdisco PRIVATE
    ${CMAKE_SOURCE_DIR}/../../../../rust/silent-disco-ffi/include
)

target_link_libraries(
    silentdisco
    oboe::oboe
    android
    log
)
