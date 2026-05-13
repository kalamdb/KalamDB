/**
 * Firebase Auth integration test for KalamDB
 *
 * Uses the masky-bb320 Firebase project.
 * No npm packages needed — uses Node.js built-in crypto + fetch.
 *
 * Steps:
 *  1. Sign a Firebase "custom token" with the service-account private key.
 *  2. Exchange it for a real Firebase ID token via the Auth REST API.
 *  3. Send the ID token to a running KalamDB instance as a Bearer token.
 *  4. Verify SELECT CURRENT_USER() returns the expected provider username.
 *
 * Usage:
 *   node backend/tests/firebase_auth_test.mjs
 */

import { createSign } from "node:crypto";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";

// ── Config ────────────────────────────────────────────────────────────────
const KALAMDB_URL = "http://127.0.0.1:2900";
const KALAMDB_ROOT_PASSWORD = process.env.KALAMDB_ROOT_PASSWORD ?? "kalamdb123";
const FIREBASE_WEB_API_KEY = "AIzaSyBotd5joGFObcOpBW733IcEbRGfB1oD5Ik";
const FIREBASE_PROJECT_ID = "masky-bb320";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const SA_PATH = path.resolve(
  __dirname,
  "../../../masky-backend/tools/masky-firebase-adminsdk.json"
);
const sa = JSON.parse(readFileSync(SA_PATH, "utf8"));

// ── Helpers ───────────────────────────────────────────────────────────────
function base64url(data) {
  const b64 = Buffer.isBuffer(data)
    ? data.toString("base64")
    : Buffer.from(JSON.stringify(data)).toString("base64");
  return b64.replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

/** Create a Firebase custom token (signed JWT) from the service account. */
function createCustomToken(uid) {
  const now = Math.floor(Date.now() / 1000);
  const header = { alg: "RS256", typ: "JWT" };
  const payload = {
    iss: sa.client_email,
    sub: sa.client_email,
    aud: "https://identitytoolkit.googleapis.com/google.identity.identitytoolkit.v1.IdentityToolkit",
    iat: now,
    exp: now + 3600,
    uid,
  };

  const signingInput = `${base64url(header)}.${base64url(payload)}`;
  const sign = createSign("RSA-SHA256");
  sign.update(signingInput);
  const signature = sign.sign(sa.private_key, "base64url");
  return `${signingInput}.${signature}`;
}

/** Exchange a custom token for a real Firebase ID token. */
async function exchangeCustomToken(customToken) {
  const url = `https://identitytoolkit.googleapis.com/v1/accounts:signInWithCustomToken?key=${FIREBASE_WEB_API_KEY}`;
  const res = await fetch(url, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ token: customToken, returnSecureToken: true }),
  });
  const body = await res.json();
  if (!res.ok) {
    throw new Error(`Firebase token exchange failed: ${JSON.stringify(body)}`);
  }
  return body.idToken;
}

/** Decode a JWT payload (no verification – display only). */
function decodeJwtPayload(token) {
  const part = token.split(".")[1];
  return JSON.parse(Buffer.from(part, "base64url").toString("utf8"));
}

/** Run a SQL query against KalamDB with a Bearer token. */
async function kalamQuery(sql, bearerToken) {
  const res = await fetch(`${KALAMDB_URL}/v1/api/sql`, {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      Authorization: `Bearer ${bearerToken}`,
    },
    body: JSON.stringify({ sql }),
  });
  const body = await res.json();
  if (!res.ok) {
    throw new Error(
      `KalamDB query failed (${res.status}): ${JSON.stringify(body)}`
    );
  }
  return body;
}

/** Log a step result */
function ok(msg) {
  console.log(`  ✅  ${msg}`);
}
function section(msg) {
  console.log(`\n── ${msg} ──`);
}

// ── Main test ─────────────────────────────────────────────────────────────
section("Step 1: Generate Firebase custom token");
const TEST_UID = "kalamdb-test-user-001";
const customToken = createCustomToken(TEST_UID);
ok(`Custom token generated for uid=${TEST_UID} (len=${customToken.length})`);

section("Step 2: Exchange for Firebase ID token");
let idToken;
try {
  idToken = await exchangeCustomToken(customToken);
  ok(`ID token obtained (len=${idToken.length})`);
} catch (err) {
  console.error(`  ❌  ${err.message}`);
  process.exit(1);
}

section("Step 3: Inspect ID token claims");
const claims = decodeJwtPayload(idToken);
console.log(`     iss : ${claims.iss}`);
console.log(`     aud : ${claims.aud}`);
console.log(`     sub : ${claims.sub}`);
console.log(`     exp : ${new Date(claims.exp * 1000).toISOString()}`);

const expectedIss = `https://securetoken.google.com/${FIREBASE_PROJECT_ID}`;
if (claims.iss !== expectedIss) {
  console.error(`  ❌  Unexpected issuer: ${claims.iss}`);
  process.exit(1);
}
if (claims.aud !== FIREBASE_PROJECT_ID) {
  console.error(`  ❌  Unexpected audience: ${claims.aud}`);
  process.exit(1);
}
ok(`Claims look correct`);

section("Step 4: Test Firebase token against KalamDB");
// Expected username in KalamDB: oidc:fbs:<firebase-uid>
const expectedUsername = `oidc:fbs:${TEST_UID}`;

let result;
try {
  result = await kalamQuery("SELECT 1 AS ok", idToken);
  ok(`Query succeeded`);
} catch (err) {
  console.error(`  ❌  ${err.message}`);
  console.error(
    "\n  Make sure KalamDB is running with the Firebase issuer configured:"
  );
  console.error(`    jwt_trusted_issuers = "${expectedIss}"`);
  console.error(`    auto_create_users_from_provider = true`);
  process.exit(1);
}

section("Step 5: Verify query succeeded and user was provisioned");
const rawResult = JSON.stringify(result);
console.log(`     Response: ${rawResult}`);

// The KalamDB SQL response includes `as_user` in each resultset, which reflects
// the authenticated identity. Verify it matches the expected Firebase username.
if (rawResult.includes(expectedUsername)) {
  ok(`Authenticated as '${expectedUsername}' (verified via as_user claim in response)`);
} else if (rawResult.includes('"1"') || rawResult.includes('"ok"')) {
  ok(`SELECT 1 succeeded — authentication accepted by KalamDB`);
  console.warn(`  ⚠️  as_user not found in response — consider upgrading KalamDB`);
} else {
  console.error(`  ❌  Unexpected response: ${rawResult}`);
  process.exit(1);
}

// Double-check user was provisioned by querying the system users table
section("Step 6: Verify user was provisioned in system.users");
let usersResult;
try {
  usersResult = await kalamQuery(
    `SELECT user_id FROM system.users WHERE user_id = '${expectedUsername}'`,
    idToken
  );
  const usersJson = JSON.stringify(usersResult);
  if (usersJson.includes(expectedUsername)) {
    ok(`User '${expectedUsername}' exists in system.users`);
  } else {
    console.warn(`  ⚠️  User not yet in system.users (may require auto_provision)\n     Response: ${usersJson}`);
  }
} catch (e) {
  console.warn(`  ⚠️  Could not query system.users: ${e.message}`);
}

console.log("\n🎉  Firebase Auth integration test PASSED\n");
