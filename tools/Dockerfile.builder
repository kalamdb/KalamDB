# Multi-platform Rust builder for KalamDB
# Supports cross-compilation to Linux, macOS, and Windows

FROM rust:1.92-bookworm

# Install cross-compilation dependencies
RUN apt-get update && apt-get install -y \
    gcc \
    g++ \
    gcc-mingw-w64-x86-64 \
    gcc-aarch64-linux-gnu \
    g++-aarch64-linux-gnu \
    gcc-x86-64-linux-gnu \
    g++-x86-64-linux-gnu \
    clang \
    lld \
        llvm-dev \
        libclang-dev \
    cmake \
    libssl-dev \
    pkg-config \
    zip \
    && rm -rf /var/lib/apt/lists/*

# Install OSXCross for macOS cross-compilation
WORKDIR /opt
RUN git clone https://github.com/tpoechtrager/osxcross.git && \
    cd osxcross && \
    wget -nc https://github.com/joseluisq/macosx-sdks/releases/download/12.3/MacOSX12.3.sdk.tar.xz && \
    mv MacOSX12.3.sdk.tar.xz tarballs/ && \
    UNATTENDED=yes OSX_VERSION_MIN=10.9 ./build.sh

ENV PATH="/opt/osxcross/target/bin:$PATH"
ENV LD_LIBRARY_PATH="/opt/osxcross/target/lib:$LD_LIBRARY_PATH"
ENV RUSTUP_TOOLCHAIN=1.92.0
ENV CARGO_ENCODED_RUSTFLAGS=

# Add Rust targets
RUN rustup target add \
    x86_64-unknown-linux-gnu \
    aarch64-unknown-linux-gnu \
    x86_64-pc-windows-gnu \
    x86_64-apple-darwin \
    aarch64-apple-darwin

# Configure cargo for cross-compilation
RUN mkdir -p /root/.cargo
COPY cargo-config.toml /root/.cargo/config.toml

WORKDIR /workspace

# Default command
CMD ["/bin/bash"]
