#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SERVER_URL="${KALAMDB_URL:-http://127.0.0.1:8080}"
USER="${KALAMDB_USER:-admin}"
PASSWORD="${KALAMDB_PASSWORD:-kalamdb123}"
SQL_FILE="$SCRIPT_DIR/chat-app.sql"
ENV_FILE="$SCRIPT_DIR/.env.local"

fail() {
	echo "[setup][error] $*" >&2
	exit 1
}

require_cmd() {
	command -v "$1" >/dev/null 2>&1 || fail "Missing required command: $1"
}

run_kalam() {
	kalam \
		--url "$SERVER_URL" \
		--user "$USER" \
		--password "$PASSWORD" \
		--no-spinner \
		"$@"
}

drop_topic_if_present() {
	local topic="$1"
	if ! run_kalam --command "DROP TOPIC $topic" >/dev/null 2>&1; then
		:
	fi
}

echo "Building local SDK packages..."
bash scripts/ensure-sdk.sh

require_cmd kalam

echo "Clearing prior example topics if they exist..."
drop_topic_if_present "react_ai_chat.agent_actions"
drop_topic_if_present "react_ai_chat.agent_messages"

echo "Importing $(basename "$SQL_FILE") with kalam CLI..."
run_kalam --file "$SQL_FILE"

cat > "$ENV_FILE" <<ENV
VITE_KALAMDB_URL=$SERVER_URL
VITE_KALAMDB_USER=$USER
VITE_KALAMDB_PASSWORD=$PASSWORD
VITE_KALAMDB_DEMO_MODE=false
KALAMDB_URL=$SERVER_URL
KALAMDB_USER=$USER
KALAMDB_PASSWORD=$PASSWORD
ENV

echo "Wrote $ENV_FILE."
echo "Server-backed mode is enabled."
echo "Next: run 'npm run agent' in one terminal and 'npm run dev' in another."