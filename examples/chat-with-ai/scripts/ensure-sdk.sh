#!/bin/bash
# Ensures the local TypeScript SDK packages are compiled before running the example.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
TYPESCRIPT_SDK_DIR="$(cd "$PROJECT_DIR/../../link/sdks/typescript" && pwd)"
CLIENT_DIR="$TYPESCRIPT_SDK_DIR/client"
CONSUMER_DIR="$TYPESCRIPT_SDK_DIR/consumer"
ORM_DIR="$TYPESCRIPT_SDK_DIR/orm"
CLIENT_CORE_DIR="$(cd "$PROJECT_DIR/../../link/kalam-client" && pwd)"
CLIENT_ENTRY="$CLIENT_DIR/dist/src/index.js"
CLIENT_WASM="$CLIENT_DIR/dist/wasm/kalam_client_bg.wasm"
CONSUMER_ENTRY="$CONSUMER_DIR/dist/src/index.js"
CONSUMER_WASM="$CONSUMER_DIR/dist/wasm/kalam_consumer_bg.wasm"
ORM_ENTRY="$ORM_DIR/dist/index.js"

client_needs_build=false
orm_needs_build=false

if [ ! -f "$CLIENT_ENTRY" ] || [ ! -f "$CLIENT_WASM" ]; then
    client_needs_build=true
elif find "$CLIENT_DIR/src" "$CLIENT_CORE_DIR/src" -type f \( -name '*.ts' -o -name '*.rs' \) -newer "$CLIENT_ENTRY" | grep -q .; then
    client_needs_build=true
elif find "$CLIENT_DIR/src" "$CLIENT_CORE_DIR/src" -type f \( -name '*.ts' -o -name '*.rs' \) -newer "$CLIENT_WASM" | grep -q .; then
    client_needs_build=true
fi

if [ ! -f "$ORM_ENTRY" ]; then
    orm_needs_build=true
elif find "$ORM_DIR/src" -type f -newer "$ORM_ENTRY" | grep -q .; then
    orm_needs_build=true
fi

if [ "$client_needs_build" = true ]; then
    echo "@kalamdb/client needs a rebuild. Building now..."
    echo ""
    cd "$CLIENT_DIR"
    bash build.sh
    echo ""
    echo "@kalamdb/client compiled successfully"
else
    echo "@kalamdb/client is ready"
fi

if [ ! -f "$CONSUMER_ENTRY" ] || [ ! -f "$CONSUMER_WASM" ]; then
    echo "@kalamdb/consumer is not compiled. Building now..."
    echo ""
    npm --prefix "$CONSUMER_DIR" run build
    echo ""
    echo "@kalamdb/consumer compiled successfully"
else
    echo "@kalamdb/consumer is ready"
fi

if [ "$orm_needs_build" = true ]; then
    echo "@kalamdb/orm needs a rebuild. Building now..."
    echo ""
    npm --prefix "$ORM_DIR" run build
    echo ""
    echo "@kalamdb/orm compiled successfully"
else
    echo "@kalamdb/orm is ready"
fi

# Copy the WASM file to public/ so Vite can serve it.
PUBLIC_WASM_DIR="$PROJECT_DIR/public/wasm"
mkdir -p "$PUBLIC_WASM_DIR"

if [ ! -f "$PUBLIC_WASM_DIR/kalam_client_bg.wasm" ] || \
   [ "$CLIENT_WASM" -nt "$PUBLIC_WASM_DIR/kalam_client_bg.wasm" ]; then
    echo "Copying WASM to public/wasm/ for browser access..."
    cp "$CLIENT_WASM" "$PUBLIC_WASM_DIR/"
    echo "WASM file ready at /wasm/kalam_client_bg.wasm"
else
    echo "WASM file in public/ is up to date"
fi
