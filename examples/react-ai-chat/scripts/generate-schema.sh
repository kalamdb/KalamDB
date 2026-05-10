#!/usr/bin/env bash
set -euo pipefail

echo "examples/react-ai-chat uses the checked-in src/app/schema.generated.ts for quick local runs."
echo "For a live server, run chat-app.sql, then regenerate with kalamdb-orm if you want schema drift checks."