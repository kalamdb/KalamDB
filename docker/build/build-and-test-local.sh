#!/usr/bin/env bash
# Build and test KalamDB Docker image locally
# This mimics what the release workflow does

set -euo pipefail

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

log_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

log_info() {
    echo -e "${GREEN}[INFO]${NC} $1"
}

log_warn() {
    echo -e "${YELLOW}[WARN]${NC} $1"
}

# Configuration
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
IMAGE_TAG="${1:-kalamdb:local}"

detect_linux_target() {
    case "$(uname -m)" in
        x86_64|amd64)
            echo "x86_64-unknown-linux-gnu linux/amd64 binaries-amd64"
            ;;
        arm64|aarch64)
            echo "aarch64-unknown-linux-gnu linux/arm64 binaries-arm64"
            ;;
        *)
            log_error "Unsupported host architecture: $(uname -m)"
            exit 1
            ;;
    esac
}

main() {
    log_info "Building and testing KalamDB Docker image locally"
    log_info "Image tag: $IMAGE_TAG"
    log_info "Project root: $PROJECT_ROOT"
    
    cd "$PROJECT_ROOT"

    read -r LINUX_TARGET DOCKER_PLATFORM BINARIES_DIR <<< "$(detect_linux_target)"
    BINARY_OUTPUT_DIR="target/$LINUX_TARGET/release"
    BUILD_TOOL="cargo"
    if [[ "$(uname -s)" == "Darwin" ]]; then
        if ! command -v cross >/dev/null 2>&1; then
            log_error "cross is required on macOS to build Linux binaries for Docker images"
            exit 1
        fi
        BUILD_TOOL="cross"
    fi

    log_info "Local Docker platform: $DOCKER_PLATFORM"
    log_info "Linux target: $LINUX_TARGET"
    
    # Step 1: Build Linux release binaries
    log_info "Step 1/4: Building Linux release binaries..."
    rustup target add "$LINUX_TARGET" >/dev/null
    if [ ! -f "$BINARY_OUTPUT_DIR/kalamdb-server" ] || [ ! -f "$BINARY_OUTPUT_DIR/kalam" ]; then
        log_info "Building with $BUILD_TOOL (this may take a while)..."
        SKIP_UI_BUILD=1 "$BUILD_TOOL" build --release --target "$LINUX_TARGET" --no-default-features --features embedded-ui,mimalloc,traceability,cloud-aws -p kalamdb-server -p kalam-cli --bin kalamdb-server --bin kalam
    else
        log_warn "Using existing Linux binaries from $BINARY_OUTPUT_DIR"
    fi
    
    # Step 2: Prepare binaries directory
    log_info "Step 2/4: Preparing binaries directory..."
    rm -rf "$BINARIES_DIR"
    mkdir -p "$BINARIES_DIR"
    cp "$BINARY_OUTPUT_DIR/kalamdb-server" "$BINARIES_DIR/"
    cp "$BINARY_OUTPUT_DIR/kalam" "$BINARIES_DIR/"
    chmod +x "$BINARIES_DIR/kalamdb-server" "$BINARIES_DIR/kalam"
    log_info "Binaries ready:"
    ls -lh "$BINARIES_DIR/"
    
    # Step 3: Build Docker image
    log_info "Step 3/4: Building Docker image..."
    docker build \
        --platform "$DOCKER_PLATFORM" \
        --build-context binaries="$BINARIES_DIR" \
        -f docker/build/Dockerfile.prebuilt \
        -t "$IMAGE_TAG" \
        .
    
    log_info "Docker image built: $IMAGE_TAG"
    docker images "$IMAGE_TAG"
    
    # Step 4: Run smoke tests
    log_info "Step 4/4: Running smoke tests..."
    chmod +x docker/build/test-docker-image.sh
    DOCKER_PLATFORM="$DOCKER_PLATFORM" ./docker/build/test-docker-image.sh "$IMAGE_TAG"
    
    # Cleanup
    log_info "Cleaning up binaries directory..."
    rm -rf "$BINARIES_DIR"
    
    log_info ""
    log_info "========================================="
    log_info "✓ Build and test completed successfully!"
    log_info "========================================="
    log_info ""
    log_info "To run the image:"
    log_info "  docker run -p 2900:2900 $IMAGE_TAG"
    log_info ""
    log_info "To push to registry:"
    log_info "  docker tag $IMAGE_TAG your-registry/$IMAGE_TAG"
    log_info "  docker push your-registry/$IMAGE_TAG"
}

main "$@"
