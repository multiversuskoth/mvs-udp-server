cmake_minimum_required(VERSION 3.15)
set(CMAKE_TOOLCHAIN_FILE "$ENV{VCPKG_ROOT}/scripts/buildsystems/vcpkg.cmake" CACHE STRING "Vcpkg toolchain file")

project(rollback-server LANGUAGES C CXX)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

if(WIN32)
    message(STATUS "Configuring for Windows")
elseif(APPLE)
    message(STATUS "Configuring for macOS")
elseif(UNIX)
    message(STATUS "Configuring for Linux")
endif()

find_package(asio REQUIRED) # Standalone Asio
find_package(CURL REQUIRED)
find_package(ZLIB REQUIRED)

include_directories(${ASIO_INCLUDE_DIR} include)

file(GLOB SOURCES src/*.cpp)

add_executable(rollback-server ${SOURCES})

target_link_libraries(rollback-server
    PRIVATE
        CURL::libcurl
        ZLIB::ZLIB
)
