# ---- Build stage (ARM64) ----
FROM --platform=linux/arm64 arm64v8/ubuntu:24.04 AS builder

RUN apt-get update && apt-get install -y \
    build-essential \
    gcc \
    g++ \
    cmake \
    git \
    curl \
    pkg-config \
    ninja-build \
    zip \
    unzip

RUN git clone --depth=1 https://github.com/microsoft/vcpkg /opt/vcpkg \
 && /opt/vcpkg/bootstrap-vcpkg.sh -disableMetrics

ENV VCPKG_ROOT=/opt/vcpkg

WORKDIR /src
COPY . /src

# (Optional) Install system libs if you need them
RUN apt-get install -y libssl-dev

# Install vcpkg deps (manifest mode)
RUN /opt/vcpkg/vcpkg install --triplet arm64-linux

# Build
RUN cmake -B build \
    -S . \
    -DCMAKE_BUILD_TYPE=Release \
    -DCMAKE_TOOLCHAIN_FILE=/opt/vcpkg/scripts/buildsystems/vcpkg.cmake \
	-DVCPKG_BUILD_TYPE=release \
    -DVCPKG_TARGET_TRIPLET=arm64-linux \
    -G Ninja \
 && cmake --build build

RUN strip /src/build/rollback-server

# ---- Final stage (artifact only) ----
FROM scratch AS artifact
# If you need a minimal runtime image, change FROM to e.g. FROM arm64v8/ubuntu:24.04

# Copy only the built binary
COPY --from=builder /src/build/rollback-server /rollback-server

# If you want to run it as a container:
ENTRYPOINT ["/rollback-server"]
