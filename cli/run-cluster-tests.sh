#!/usr/bin/env bash

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RUNNER="$SCRIPT_DIR/run-tests.sh"
FOLLOWER_LIST="$SCRIPT_DIR/test-lists/cluster-followers.txt"

RUN_FOLLOWERS=false
USER_SUPPLIED_TEST=false
USER_SUPPLIED_TEST_LIST=false
USER_SUPPLIED_URL=false
USER_SUPPLIED_CLUSTER_URLS=false
PASSTHROUGH_ARGS=()

show_help() {
    echo "Usage: $0 [--followers] [run-tests.sh options]"
    echo ""
    echo "Runs the CLI cluster integration tests in kalam-cli-e2e."
    echo "Defaults to the local 3-node cluster at http://127.0.0.1:2901-2903."
    echo ""
    echo "Options:"
    echo "  --followers              Run the curated follower/replication bundle"
    echo "  -h, --help               Show this help message"
    echo ""
    echo "Other options are forwarded to run-tests.sh, including:"
    echo "  --url, --cluster-urls, --password, --jobs, --nocapture, --test"
    echo ""
    echo "Examples:"
    echo "  $0"
    echo "  $0 --followers --nocapture"
    echo "  $0 --url http://127.0.0.1:2901 --followers --nocapture"
    echo "  $0 --test cluster_test_ws_follower_receives_leader_changes --nocapture"
}

while [[ $# -gt 0 ]]; do
    case $1 in
        --followers)
            RUN_FOLLOWERS=true
            shift
            ;;
        -t|--test)
            USER_SUPPLIED_TEST=true
            PASSTHROUGH_ARGS+=("$1")
            shift
            if [[ $# -eq 0 ]]; then
                echo "Error: $0 requires a value after --test"
                exit 1
            fi
            PASSTHROUGH_ARGS+=("$1")
            shift
            ;;
        -u|--url)
            USER_SUPPLIED_URL=true
            PASSTHROUGH_ARGS+=("$1")
            shift
            if [[ $# -eq 0 ]]; then
                echo "Error: $0 requires a value after --url"
                exit 1
            fi
            PASSTHROUGH_ARGS+=("$1")
            shift
            ;;
        --cluster-urls|--urls)
            USER_SUPPLIED_CLUSTER_URLS=true
            PASSTHROUGH_ARGS+=("$1")
            shift
            if [[ $# -eq 0 ]]; then
                echo "Error: $0 requires a value after --cluster-urls"
                exit 1
            fi
            PASSTHROUGH_ARGS+=("$1")
            shift
            ;;
        --test-list)
            USER_SUPPLIED_TEST_LIST=true
            PASSTHROUGH_ARGS+=("$1")
            shift
            if [[ $# -eq 0 ]]; then
                echo "Error: $0 requires a value after --test-list"
                exit 1
            fi
            PASSTHROUGH_ARGS+=("$1")
            shift
            ;;
        -h|--help)
            show_help
            exit 0
            ;;
        *)
            PASSTHROUGH_ARGS+=("$1")
            shift
            ;;
    esac
done

if [ ! -x "$RUNNER" ]; then
    echo "Error: runner not found or not executable: $RUNNER"
    exit 1
fi

if [ "$RUN_FOLLOWERS" = true ] && { [ "$USER_SUPPLIED_TEST" = true ] || [ "$USER_SUPPLIED_TEST_LIST" = true ]; }; then
    echo "Error: --followers cannot be combined with --test or --test-list"
    exit 1
fi

CMD=(
    "$RUNNER"
    --package kalam-cli-e2e
    --server-type cluster
    --test-target cluster
)

if [ "$RUN_FOLLOWERS" = true ]; then
    CMD+=(--test-list "$FOLLOWER_LIST")
elif [ "$USER_SUPPLIED_TEST" = false ] && [ "$USER_SUPPLIED_TEST_LIST" = false ]; then
    # The e2e binary also contains smoke, auth, and CLI tests. This runner
    # only executes the cluster suite unless the caller passes --test or --test-list.
    CMD+=(--test cluster)
fi

DEFAULT_CLUSTER_URLS="http://127.0.0.1:2901,http://127.0.0.1:2902,http://127.0.0.1:2903"
CLUSTER_URLS_VALUE="$DEFAULT_CLUSTER_URLS"
if [ "$USER_SUPPLIED_CLUSTER_URLS" = true ]; then
    for i in "${!PASSTHROUGH_ARGS[@]}"; do
        case "${PASSTHROUGH_ARGS[$i]}" in
            --cluster-urls|--urls)
                CLUSTER_URLS_VALUE="${PASSTHROUGH_ARGS[$((i + 1))]}"
                ;;
        esac
    done
else
    CMD+=(--cluster-urls "$DEFAULT_CLUSTER_URLS")
fi

if [ "$USER_SUPPLIED_URL" = false ]; then
    CMD+=(--url "${CLUSTER_URLS_VALUE%%,*}")
fi

CMD+=("${PASSTHROUGH_ARGS[@]}")

echo "Executing: ${CMD[*]}"
echo ""
exec "${CMD[@]}"