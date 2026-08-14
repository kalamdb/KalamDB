#!/usr/bin/env bash
# Bootstrap PocketBase admin, user, room, and room_members for the comparison bench.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PB_URL="${POCKETBASE_URL:-http://127.0.0.1:8090}"
ADMIN_EMAIL="admin@bar.com"
USER_EMAIL="user@bar.com"
PASSWORD="1234567890"
ROOM_NAME="room0"
BIN="${ROOT}/bin/pocketbase"
DATA="${ROOT}/setups/pocketbase/pb_data"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

if [[ ! -x "$BIN" ]]; then
  echo "Missing $BIN — run scripts/download-binaries.sh first" >&2
  exit 1
fi

mkdir -p "$DATA"
"$BIN" superuser upsert "$ADMIN_EMAIL" "$PASSWORD" --dir "$DATA" >/dev/null

curl -sf -X POST "$PB_URL/api/collections/_superusers/auth-with-password" \
  -H 'Content-Type: application/json' \
  -d "{\"identity\":\"$ADMIN_EMAIL\",\"password\":\"$PASSWORD\"}" >"$TMP/admin.json"
ADMIN_TOKEN=$(python3 -c 'import json; print(json.load(open("'"$TMP"'/admin.json"))["token"])')

auth_get() {
  curl -sf -H "Authorization: $ADMIN_TOKEN" "$1"
}
auth_post() {
  curl -sf -H "Authorization: $ADMIN_TOKEN" -H 'Content-Type: application/json' -X POST "$1" -d "$2"
}

auth_get "$PB_URL/api/collections/users/records?filter=email%3D%27${USER_EMAIL}%27" >"$TMP/users.json"
USER_ID=$(python3 -c 'import json; items=json.load(open("'"$TMP"'/users.json")).get("items") or []; print(items[0]["id"] if items else "")')
if [[ -z "$USER_ID" ]]; then
  auth_post "$PB_URL/api/collections/users/records" \
    "{\"email\":\"$USER_EMAIL\",\"password\":\"$PASSWORD\",\"passwordConfirm\":\"$PASSWORD\",\"verified\":true}" \
    >"$TMP/user.json"
  USER_ID=$(python3 -c 'import json; print(json.load(open("'"$TMP"'/user.json"))["id"])')
fi

auth_get "$PB_URL/api/collections/room/records?filter=name%3D%27${ROOM_NAME}%27" >"$TMP/rooms.json"
ROOM_ID=$(python3 -c 'import json; items=json.load(open("'"$TMP"'/rooms.json")).get("items") or []; print(items[0]["id"] if items else "")')
if [[ -z "$ROOM_ID" ]]; then
  auth_post "$PB_URL/api/collections/room/records" "{\"name\":\"$ROOM_NAME\"}" >"$TMP/room.json"
  ROOM_ID=$(python3 -c 'import json; print(json.load(open("'"$TMP"'/room.json"))["id"])')
fi

FILTER="user%3D%27${USER_ID}%27%26%26room%3D%27${ROOM_ID}%27"
auth_get "$PB_URL/api/collections/room_members/records?filter=${FILTER}" >"$TMP/members.json"
COUNT=$(python3 -c 'import json; print(len(json.load(open("'"$TMP"'/members.json")).get("items") or []))')
if [[ "$COUNT" -eq 0 ]]; then
  auth_post "$PB_URL/api/collections/room_members/records" \
    "{\"user\":\"$USER_ID\",\"room\":\"$ROOM_ID\"}" >/dev/null
fi

echo "PocketBase bootstrapped: user=$USER_ID room=$ROOM_ID"
