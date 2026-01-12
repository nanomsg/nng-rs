#!/bin/bash
# Build and install NNG from the submodule source.
# This script handles OS-specific differences between Linux and macOS.
set -ex

cd nng-sys/nng
mkdir -p build && cd build

# OS-specific settings
if [[ "$OSTYPE" == "darwin"* ]]; then
    PREFIX="/usr/local"
    JOBS=$(sysctl -n hw.ncpu)
else
    PREFIX="/usr"
    JOBS=$(nproc)
fi

cmake -DCMAKE_INSTALL_PREFIX="$PREFIX" -DBUILD_SHARED_LIBS=ON \
      -DNNG_TESTS=OFF -DNNG_TOOLS=OFF ..
make -j"$JOBS"
sudo make install

# Linux needs ldconfig to update the shared library cache
if [[ "$OSTYPE" != "darwin"* ]]; then
    sudo ldconfig
fi
