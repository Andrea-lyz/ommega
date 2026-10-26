cmake_minimum_required(VERSION 3.22)
project(MaskCPU VERSION 0.3.0 LANGUAGES CXX)

if(NOT ANDROID OR NOT ANDROID_ABI MATCHES "^(arm64-v8a|x86_64)$")
    message(FATAL_ERROR "Build with the Android NDK for a 64-bit Android ABI.")
endif()

add_library(maskcpu SHARED src/main.cpp src/property_view.cpp src/property_storage.cpp src/runtime_profile.cpp)
target_include_directories(maskcpu PRIVATE third_party)
target_compile_features(maskcpu PRIVATE cxx_std_17)
target_compile_options(maskcpu PRIVATE
    -Wall -Wextra -Werror
    -fno-exceptions -fno-rtti -fno-threadsafe-statics
    -fvisibility=hidden -fvisibility-inlines-hidden
    -ffunction-sections -fdata-sections)
target_link_options(maskcpu PRIVATE
    -Wl,--no-undefined -Wl,--gc-sections -Wl,--exclude-libs,ALL
    -Wl,-z,max-page-size=16384)
target_link_libraries(maskcpu PRIVATE log)
set_target_properties(maskcpu PROPERTIES
    PREFIX ""
    OUTPUT_NAME "arm64-v8a"
    LIBRARY_OUTPUT_DIRECTORY "${CMAKE_BINARY_DIR}/zygisk")
