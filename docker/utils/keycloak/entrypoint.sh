#!/bin/bash
# Note: no set -e so kcadm retry loop can handle failures gracefully

# Start Keycloak in the background
/opt/keycloak/bin/kc.sh start-dev --import-realm --http-enabled=true --hostname-strict=false &
KC_PID=$!

# Wait for Keycloak to be ready (image has no curl/wget; use sleep + kcadm retry)
echo "Waiting for Keycloak to start..."
sleep 20

# Disable SSL requirement on the master realm so /admin works over plain HTTP
echo "Disabling SSL requirement on master realm..."
for i in $(seq 1 10); do
  /opt/keycloak/bin/kcadm.sh config credentials \
    --server http://localhost:2900 \
    --realm master \
    --user "${KC_BOOTSTRAP_ADMIN_USERNAME:-${KEYCLOAK_ADMIN:-admin}}" \
    --password "${KC_BOOTSTRAP_ADMIN_PASSWORD:-${KEYCLOAK_ADMIN_PASSWORD:-admin}}" 2>&1 && break
  echo "kcadm not ready yet, retrying in 5s... ($i/10)"
  sleep 5
done

/opt/keycloak/bin/kcadm.sh update realms/master -s sslRequired=NONE
echo "SSL requirement disabled on master realm."

# Keep the Keycloak process running
wait $KC_PID
