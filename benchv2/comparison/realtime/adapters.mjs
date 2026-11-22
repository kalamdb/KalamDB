import assert from "node:assert/strict";
import { createRequire } from "node:module";

const owner = "user1";
const room = "room0";

function message(sequence, ownerId = owner, roomId = room) {
  return { owner: ownerId, room: roomId, data: `a message ${sequence}` };
}

function textId(value) {
  assert.equal(typeof value, "string");
  assert.ok(value.length, "expected a non-empty ID");
  return value;
}

function generatedKeys() {
  const bySequence = new Map();
  const byKey = new Map();
  return (sequence, id) => {
    const key = textId(id);
    assert.equal(bySequence.get(sequence) ?? key, key, "write and subscriber keys disagree");
    assert.equal(byKey.get(key) ?? sequence, sequence, "messages share a generated key");
    bySequence.set(sequence, key);
    byKey.set(key, sequence);
    return key;
  };
}

function normalize(row, total, ownerId = owner, roomId = room, ownerField = "owner") {
  const match = /^a message (0|[1-9][0-9]*)$/.exec(row?.data);
  assert.ok(match, "invalid message data");
  const sequence = Number(match[1]);
  assert.ok(Number.isSafeInteger(sequence) && sequence < total, "unexpected message sequence");
  assert.equal(row.data, `a message ${sequence}`);
  assert.equal(row[ownerField], ownerId, `unexpected owner for message ${sequence}`);
  assert.equal(row.room, roomId, `unexpected room for message ${sequence}`);
  return { sequence, owner, room, key: row.id };
}

function one(rows) {
  assert.ok(Array.isArray(rows));
  assert.equal(rows.length, 1, "expected exactly one result");
  return rows[0];
}

function bearer(token) {
  return { authorization: `Bearer ${textId(token)}` };
}

function scoped(signal) {
  const controller = new AbortController();
  return {
    signal: AbortSignal.any([controller.signal, signal]),
    close: () => controller.abort(),
  };
}

function untilAborted(promise, signal) {
  if (signal.aborted) {
    return Promise.reject(signal.reason);
  }
  const { promise: abort, reject } = Promise.withResolvers();
  const onAbort = () => reject(signal.reason);
  signal.addEventListener("abort", onAbort, { once: true });
  return Promise.race([promise, abort])
    .finally(() => signal.removeEventListener("abort", onAbort));
}

async function request(base, path, { signal, headers, body, json = true }) {
  const response = await fetch(new URL(path, base), {
    method: body === undefined ? "GET" : "POST",
    headers: {
      accept: "application/json",
      ...(body === undefined ? {} : { "content-type": json ? "application/json" : "text/plain" }),
      ...headers,
    },
    signal,
    body: json && body !== undefined ? JSON.stringify(body) : body,
  });
  if (!response.ok) {
    await response.body?.cancel();
    throw new Error(`${response.status} ${response.statusText} from ${path}`);
  }
  return response.status === 204 ? undefined : response.json();
}

function jsonSocket(base, path, protocol, signal, receive, onError) {
  signal.throwIfAborted();
  const url = new URL(path, base);
  url.protocol = url.protocol === "https:" ? "wss:" : "ws:";
  const socket = new WebSocket(url, protocol);
  const ready = Promise.withResolvers();
  let closed = false;
  const close = () => {
    closed = true;
    signal.removeEventListener("abort", close);
    if (socket.readyState < WebSocket.CLOSING) {
      socket.close();
    }
  };
  function fail(error) {
    if (!closed) {
      ready.reject(error);
      close();
      onError(error);
    }
  }
  signal.addEventListener("abort", close, { once: true });
  socket.addEventListener("open", () => ready.resolve(), { once: true });
  socket.addEventListener("error", () => fail(new Error(`WebSocket error on ${url.pathname}`)));
  socket.addEventListener("close", ({ code }) => {
    fail(new Error(`WebSocket on ${url.pathname} closed with code ${code}`));
  });
  socket.addEventListener("message", ({ data }) => {
    if (closed) return;
    try {
      receive(JSON.parse(data));
    } catch (error) {
      fail(error);
    }
  });
  return {
    opened: untilAborted(ready.promise, signal),
    send: (message) => socket.send(JSON.stringify(message)),
    close,
  };
}

async function openSse(base, path, headers, signal, receive, onError) {
  const scope = scoped(signal);
  let response;
  try {
    response = await fetch(new URL(path, base), {
      signal: scope.signal,
      headers: { accept: "text/event-stream", ...headers },
    });
    assert.ok(response.ok, `SSE returned HTTP ${response.status} from ${path}`);
    assert.match(response.headers.get("content-type") ?? "", /^text\/event-stream\b/);
    assert.ok(response.body, "SSE response has no body");
  } catch (error) {
    scope.close();
    throw error;
  }

  const read = async () => {
    let buffer = "";
    try {
      for await (const chunk of response.body.pipeThrough(new TextDecoderStream())) {
        buffer += chunk;
        for (let boundary; (boundary = /\r?\n\r?\n/.exec(buffer)) !== null;) {
          const frame = buffer.slice(0, boundary.index);
          buffer = buffer.slice(boundary.index + boundary[0].length);
          let event = "message";
          const data = [];
          for (const line of frame.split(/\r?\n/)) {
            if (line.startsWith("event:")) event = line.slice(6).trimStart();
            if (line.startsWith("data:")) data.push(line.slice(5).replace(/^ /, ""));
          }
          if (data.length) {
            receive(event, JSON.parse(data.join("\n")));
          }
        }
      }
      throw new Error(`SSE ended on ${path}`);
    } catch (error) {
      if (!scope.signal.aborted) {
        onError(error);
      }
    }
  };
  const task = read();

  return async () => {
    scope.close();
    await task;
  };
}

async function trailbase(base, { history, total, signal }) {
  const path = "/api/records/v1/message_api";
  const ownerId = "AZH8mYedc1K7hrsTZgdHBA==";
  const roomId = "AZH8mYTFd5OexZn4K10jCA==";
  const { auth_token } = await request(base, "/api/auth/v1/login", {
    signal,
    body: { email: "user@localhost", password: "secret" },
  });
  const headers = bearer(auth_token);
  const checkKey = generatedKeys();
  const normalizeRow = (row) => {
    const normalized = normalize(row, total, ownerId, roomId, "_owner");
    checkKey(normalized.sequence, row.id);
    return normalized;
  };
  return {
    async write(sequence) {
      const { ids } = await request(base, path, {
        signal,
        headers,
        body: { _owner: ownerId, room: roomId, data: message(sequence).data },
      });
      checkKey(sequence, one(ids));
    },
    async subscribe(onSnapshot, onInsert, onError) {
      // The server sends a ping comment at sequence zero to flush the SSE headers.
      let nextSequence = 1;
      const close = await openSse(base, `${path}/subscribe/*`, headers, signal, (_, event) => {
        assert.equal(event.seq, nextSequence++, "TrailBase subscription sequence gap");
        assert.ok(event.Insert, "expected a TrailBase insert event");
        onInsert(normalizeRow(event.Insert));
      }, onError);
      try {
        const { records } = await request(base, `${path}?limit=${history + 1}`, { signal, headers });
        onSnapshot(records.map(normalizeRow));
        return close;
      } catch (error) {
        await close();
        throw error;
      }
    },
  };
}

async function pocketbase(base, { history, total, signal }) {
  const path = "/api/collections/message/records";
  const auth = await request(base, "/api/collections/users/auth-with-password", {
    signal,
    body: { identity: "user@bar.com", password: "1234567890" },
  });
  const headers = bearer(auth.token);
  const ownerId = textId(auth.record.id);
  const { items } = await request(base, "/api/collections/room/records?perPage=200", { signal, headers });
  const roomId = textId(one(items.filter(({ name }) => name === room)).id);
  const checkKey = generatedKeys();
  const normalizeRow = (row) => {
    const normalized = normalize(row, total, ownerId, roomId);
    checkKey(normalized.sequence, row.id);
    return normalized;
  };
  return {
    async write(sequence) {
      const row = await request(base, path, {
        signal,
        headers,
        body: message(sequence, ownerId, roomId),
      });
      assert.equal(normalizeRow(row).sequence, sequence);
    },
    async subscribe(onSnapshot, onInsert, onError) {
      const handshake = Promise.withResolvers();
      const close = await openSse(base, "/api/realtime", {}, signal, (event, data) => {
        if (event === "PB_CONNECT") {
          handshake.resolve(textId(data.clientId));
        } else {
          assert.equal(data.action, "create");
          onInsert(normalizeRow(data.record));
        }
      }, (error) => {
        handshake.reject(error);
        onError(error);
      });
      try {
        const clientId = await untilAborted(handshake.promise, signal);
        await request(base, "/api/realtime", {
          signal,
          headers,
          body: { clientId, subscriptions: ["message/*"] },
        });
        const { items: records } = await request(base, `${path}?perPage=${history + 1}`, { signal, headers });
        onSnapshot(records.map(normalizeRow));
        return close;
      } catch (error) {
        await close();
        throw error;
      }
    },
  };
}

async function convex(base, { total, signal }) {
  const require = createRequire(new URL("../setups/convex/package.json", import.meta.url));
  const { ConvexClient } = require("convex/browser");
  const { makeFunctionReference } = require("convex/server");
  const writer = new ConvexClient(base);
  const insert = makeFunctionReference("messages:insertRealtimeMessage");
  const list = makeFunctionReference("messages:roomMessages");
  const checkKey = generatedKeys();
  const normalizeRow = (row) => {
    const normalized = normalize(row, total);
    assert.equal(row.id, normalized.sequence);
    return { ...normalized, key: checkKey(normalized.sequence, row._id) };
  };
  return {
    async write(sequence) {
      const key = await untilAborted(writer.mutation(insert, { id: sequence, ...message(sequence) }), signal);
      checkKey(sequence, key);
    },
    async subscribe(onSnapshot, _onInsert, onError) {
      const client = new ConvexClient(base);
      const ready = Promise.withResolvers();
      const fail = (error) => {
        ready.reject(error);
        onError(error);
      };
      const unsubscribe = client.onUpdate(list, { room }, (rows) => {
        try {
          onSnapshot(rows.map(normalizeRow));
          ready.resolve();
        } catch (error) {
          fail(error);
        }
      }, fail);
      const close = async () => {
        unsubscribe();
        await client.close();
      };
      try {
        await untilAborted(ready.promise, signal);
        return close;
      } catch (error) {
        await close();
        throw error;
      }
    },
    close: () => writer.close(),
  };
}

function surrealResult(response) {
  const statement = one(response);
  assert.equal(statement.status, "OK", "SurrealDB statement failed");
  return statement.result;
}

async function surrealdb(base, { total, signal }) {
  const { token } = await request(base, "/signin", {
    signal,
    body: { user: "root", pass: "root" },
  });
  const headers = { ...bearer(token), "Surreal-NS": "bench", "Surreal-DB": "bench" };
  async function sql(body, routeHeaders = headers) {
    return surrealResult(await request(base, "/sql", {
      signal, headers: routeHeaders, body, json: false,
    }));
  }
  await sql("DEFINE NAMESPACE IF NOT EXISTS bench", bearer(token));
  await sql("DEFINE DATABASE IF NOT EXISTS bench", { ...bearer(token), "Surreal-NS": "bench" });
  await sql("DEFINE TABLE IF NOT EXISTS message SCHEMALESS PERMISSIONS FULL");

  const normalizeRow = (row) => {
    const normalized = normalize(row, total);
    assert.equal(row.id, `message:${normalized.sequence}`);
    return normalized;
  };
  return {
    async write(sequence) {
      const result = surrealResult(await request(base, `/key/message/${sequence}`, {
        signal, headers, body: message(sequence),
      }));
      assert.equal(normalizeRow(one(result)).sequence, sequence);
    },
    async subscribe(onSnapshot, onInsert, onError) {
      const scope = scoped(signal);
      const pending = new Map();
      let nextId = 0;
      let liveId;
      const socket = jsonSocket(base, "/rpc", "json", scope.signal, (frame) => {
        if (frame.id !== undefined) {
          const reply = pending.get(frame.id);
          assert.ok(reply, "unknown SurrealDB reply ID");
          pending.delete(frame.id);
          if (frame.error) {
            reply.reject(new Error(`SurrealDB RPC failed: ${frame.error.message}`));
          } else {
            reply.resolve(frame.result);
          }
          return;
        }
        assert.ok(liveId, "live event arrived before SurrealDB subscription was registered");
        assert.equal(frame.result.id, liveId);
        assert.equal(frame.result.action, "CREATE");
        onInsert(normalizeRow(frame.result.result));
      }, (error) => {
        for (const reply of pending.values()) reply.reject(error);
        pending.clear();
        onError(error);
      });
      async function rpc(method, params) {
        const id = ++nextId;
        const reply = Promise.withResolvers();
        pending.set(id, reply);
        socket.send({ id, method, params });
        return untilAborted(reply.promise, scope.signal);
      }
      const close = async () => {
        scope.close();
        socket.close();
      };
      try {
        await socket.opened;
        await rpc("authenticate", [token]);
        await rpc("use", ["bench", "bench"]);
        liveId = textId(surrealResult(await rpc("query", ["LIVE SELECT * FROM message"])));
        const rows = surrealResult(await rpc("query", ["SELECT * FROM message"]));
        onSnapshot(rows.map(normalizeRow));
        return close;
      } catch (error) {
        await close();
        throw error;
      }
    },
  };
}

async function kalamdb(base, { total, signal }) {
  const auth = { user: "admin", password: "kalamdb123" };
  const status = await request(base, "/v1/api/auth/status", { signal });
  if (status.needs_setup) {
    await request(base, "/v1/api/auth/setup", { signal, body: { ...auth, root_password: auth.password } });
  }
  const { access_token: token } = await request(base, "/v1/api/auth/login", { signal, body: auth });
  const headers = bearer(token);
  async function sql(statement, params) {
    const response = await request(base, "/v1/api/sql", {
      signal,
      headers,
      body: { sql: statement, params },
    });
    assert.equal(response.status, "success", `KalamDB SQL failed: ${response.error?.message ?? statement}`);
    return one(response.results);
  }
  await sql("CREATE SCHEMA IF NOT EXISTS bench");
  await sql("CREATE TABLE bench.message (id INT PRIMARY KEY, owner TEXT, room TEXT, data TEXT)");

  const normalizeRow = (row) => {
    const normalized = normalize(row, total);
    assert.equal(row.id, normalized.sequence);
    return normalized;
  };
  return {
    async write(sequence) {
      const { data } = message(sequence);
      const result = await sql("INSERT INTO bench.message (id, owner, room, data) VALUES ($1, $2, $3, $4)",
        [sequence, owner, room, data]);
      assert.equal(result.row_count, 1, "KalamDB insert did not affect exactly one row");
    },
    async subscribe(onSnapshot, onInsert, onError) {
      const scope = scoped(signal);
      const ready = Promise.withResolvers();
      const initialRows = [];
      const subscriptionId = "bench";
      const close = async () => {
        scope.close();
        socket.close();
      };
      const socket = jsonSocket(base, "/v1/ws?serialization=json&compression=none&compress=false",
        `kalamdb.jwt.${token}`, scope.signal, (frame) => {
          switch (frame.type) {
            case "auth_success":
              socket.send({
                type: "subscribe",
                subscription: {
                  id: subscriptionId,
                  sql: "SELECT id, owner, room, data FROM bench.message",
                  options: { batch_size: 100 },
                },
              });
              break;
            case "subscription_ack":
              assert.equal(frame.subscription_id, subscriptionId);
              break;
            case "initial_data_batch":
              assert.equal(frame.subscription_id, subscriptionId);
              initialRows.push(...frame.rows.map(normalizeRow));
              if (frame.batch_control.has_more) {
                socket.send({
                  type: "next_batch",
                  subscription_id: subscriptionId,
                  last_seq_id: frame.batch_control.last_seq_id,
                });
              } else {
                onSnapshot(initialRows);
                ready.resolve();
              }
              break;
            case "change":
              assert.equal(frame.subscription_id, subscriptionId);
              assert.equal(frame.change_type, "insert");
              frame.rows.map(normalizeRow).forEach(onInsert);
              break;
            case "pong":
              break;
            default:
              throw new Error(`unexpected KalamDB message: ${frame.type} ${frame.message ?? ""}`);
          }
        }, (error) => {
          ready.reject(error);
          onError(error);
        });
      try {
        await Promise.all([socket.opened, untilAborted(ready.promise, scope.signal)]);
        return close;
      } catch (error) {
        await close();
        throw error;
      }
    },
  };
}

const adapters = new Map([
  ["convex", convex],
  ["kalamdb", kalamdb],
  ["pocketbase", pocketbase],
  ["surrealdb", surrealdb],
  ["trailbase", trailbase],
]);

export async function connect(system, base, { history, messages, signal }) {
  const adapter = adapters.get(system);
  assert.ok(adapter, `unknown realtime adapter: ${system}`);
  const endpoint = new URL(base);
  assert.ok(["127.0.0.1", "localhost", "[::1]"].includes(endpoint.hostname),
    "realtime benchmark requires a local server");
  assert.ok(["http:", "https:"].includes(endpoint.protocol), "invalid benchmark URL");
  assert.ok(Number.isSafeInteger(history) && history > 0, "history must be a positive integer");
  assert.ok(Number.isSafeInteger(messages) && messages > 0, "messages must be a positive integer");
  const total = history + messages;
  assert.ok(total <= 500, "at most 500 history and measured messages per run");
  return adapter(base, { history, total, signal });
}
