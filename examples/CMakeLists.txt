cmake_minimum_required(VERSION 3.10)

project(RustCppIntegration)

# Set the C++ standard
set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED True)

# Add the Rust shared library
set(RUST_LIB_PATH "../target/debug")
set(RUST_LIB_NAME "mvs_p2p_rollback_server")

# Include the Rust library
link_directories(${RUST_LIB_PATH})

# Add the executable
add_executable(wrapper wrapper.cpp)

# Link the Rust shared library
set_target_properties(wrapper PROPERTIES LINK_FLAGS "-Wl,-rpath,${RUST_LIB_PATH}")
target_link_libraries(wrapper ${RUST_LIB_NAME})