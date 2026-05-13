#!/usr/bin/env bash
# Run the opt-in realtime chat workload benchmark.
#
# Usage:
#   ./run-chat-realtime.sh
#   ./run-chat-realtime.sh --url http://127.0.0.1:2900 --minutes 10 --users 1000 --realtime-convs 150
#   ./run-chat-realtime.sh --messages-per-minute 20
#   ./run-chat-realtime.sh --messages-per-minute 0

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$SCRIPT_DIR"

URL="${KALAMDB_URL:-http://127.0.0.1:2900}"
MINUTES="${KALAMDB_BENCH_CHAT_MINUTES:-5}"
USER_COUNT="${KALAMDB_BENCH_CHAT_USERS:-1000}"
REALTIME_CONVS="${KALAMDB_BENCH_CHAT_REALTIME_CONVS:-100}"
MESSAGES_PER_MINUTE="${KALAMDB_BENCH_CHAT_MESSAGES_PER_MINUTE:-20}"
BENCH_USER="${KALAMDB_USER:-}"
BENCH_PASSWORD="${KALAMDB_PASSWORD:-}"
EXTRA_ARGS=()

while [[ $# -gt 0 ]]; do
    case "$1" in
        --url)
            URL="$2"
            shift 2
            ;;
        --minutes)
            MINUTES="$2"
            shift 2
            ;;
        --users)
            USER_COUNT="$2"
            shift 2
            ;;
        --realtime-convs)
            REALTIME_CONVS="$2"
            shift 2
            ;;
        --messages-per-minute)
            MESSAGES_PER_MINUTE="$2"
            shift 2
            ;;
        --user)
            BENCH_USER="$2"
            shift 2
            ;;
        --password)
            BENCH_PASSWORD="$2"
            shift 2
            ;;
        *)
            EXTRA_ARGS+=("$1")
            shift
            ;;
    esac
done

export KALAMDB_BENCH_CHAT_MINUTES="$MINUTES"
export KALAMDB_BENCH_CHAT_USERS="$USER_COUNT"
export KALAMDB_BENCH_CHAT_REALTIME_CONVS="$REALTIME_CONVS"
export KALAMDB_BENCH_CHAT_MESSAGES_PER_MINUTE="$MESSAGES_PER_MINUTE"

echo "▸ Running chat_realtime benchmark"
echo "▸ URL: $URL"
echo "▸ Minutes: $MINUTES"
echo "▸ Seeded users: $USER_COUNT"
echo "▸ Active conversations: $REALTIME_CONVS"
if [[ "$MESSAGES_PER_MINUTE" == "0" ]]; then
    echo "▸ Conversation message rate: idle subscriptions only"
else
    echo "▸ Conversation message rate: $MESSAGES_PER_MINUTE messages/min"
fi

CMD=(./run-benchmarks.sh --urls "$URL" --suite chat-runtime --output-dir results/chat-runtime --bench chat_realtime --iterations 1 --warmup 0)

if [[ -n "$BENCH_USER" ]]; then
    CMD+=(--user "$BENCH_USER")
fi

if [[ -n "$BENCH_PASSWORD" ]]; then
    CMD+=(--password "$BENCH_PASSWORD")
fi

if (( ${#EXTRA_ARGS[@]} > 0 )); then
    CMD+=("${EXTRA_ARGS[@]}")
fi

exec "${CMD[@]}"
