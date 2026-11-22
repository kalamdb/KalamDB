import { setTimeout as sleep } from "node:timers/promises";
import { connect } from "./adapters.mjs";

const [system, base] = process.argv.slice(2);
const timeoutMs = 15_000;
const drainMs = 250;

function setting(name, fallback, minimum, maximum) {
  const raw = process.env[name];
  const value = raw === undefined ? fallback : Number(raw);
  if ((raw !== undefined && !/^\d+$/.test(raw)) || !Number.isSafeInteger(value) ||
      value < minimum || value > maximum) {
    throw new Error(`${name} must be an integer in ${minimum}..${maximum}`);
  }
  return value;
}

function stats(samples) {
  if (samples.length === 0) return null;
  const sorted = [...samples].sort((a, b) => a - b);
  const percentile = p => sorted[Math.floor(sorted.length * p)];
  return { count: sorted.length, p50_ms: percentile(0.50), p95_ms: percentile(0.95) };
}

function within(promise, ms, reason) {
  return Promise.race([
    promise,
    sleep(ms, undefined, { ref: false }).then(() => { throw new Error(reason); }),
  ]);
}

async function run() {
  const history = setting("REALTIME_HISTORY", 20, 1, 499);
  const messages = setting("REALTIME_MESSAGES", 100, 1, 500 - history);
  const subscribers = setting("REALTIME_SUBSCRIBERS", 16, 1, 128);
  const intervalMs = setting("REALTIME_INTERVAL_MS", 50, 1, 60_000);
  const lifetime = new AbortController();
  const started = new Map();
  const historyLatencies = [];
  const deliveryLatencies = [];
  const writeLatencies = [];
  const stop = [];
  const states = Array.from({ length: subscribers }, () => new Map());
  let duplicates = 0;
  let fatal;
  let adapter;
  const failed = new Promise((_, reject) => { fatal = reject; });
  // The same failure signal races each stage. Handle errors arriving between stages.
  failed.catch(() => {});
  const monitored = promise => Promise.race([promise, failed]);
  const expected = messages * subscribers;

  function validate(row) {
    if (!Number.isInteger(row.sequence) || row.sequence < 0 || row.sequence >= history + messages ||
        row.owner !== "user1" || row.room !== "room0" ||
        !["number", "string"].includes(typeof row.key) || row.key === "") {
      throw new Error(`unexpected subscription row ${JSON.stringify(row)}`);
    }
  }

  function deliver(seen, row, receivedAt) {
    validate(row);
    const sentAt = started.get(row.sequence);
    if (sentAt === undefined) throw new Error(`unsent message ${row.sequence}`);
    if (seen.has(row.sequence)) {
      duplicates++;
      if (seen.get(row.sequence) !== row.key) throw new Error(`changed key for ${row.sequence}`);
      return;
    }
    seen.set(row.sequence, row.key);
    deliveryLatencies.push(receivedAt - sentAt);
  }

  function observe(index) {
    const openedAt = performance.now();
    const seen = states[index];
    let ready;
    const initialized = new Promise(resolve => { ready = resolve; });
    let first = true;
    const handleSnapshot = rows => {
      const receivedAt = performance.now();
      try {
        const snapshot = new Map();
        const keys = new Set();
        for (const row of rows) {
          validate(row);
          if (snapshot.has(row.sequence) || keys.has(row.key)) {
            duplicates++;
            throw new Error(`duplicate row in subscriber ${index} snapshot`);
          }
          snapshot.set(row.sequence, row.key);
          keys.add(row.key);
        }
        if (first) {
          if (snapshot.size !== history || rows.some(row => row.sequence >= history)) {
            throw new Error(`subscriber ${index} received incorrect initial history`);
          }
          first = false;
          for (const [sequence, key] of snapshot) seen.set(sequence, key);
          historyLatencies.push(receivedAt - openedAt);
          ready();
          return;
        }
        for (const [sequence, key] of seen) {
          if (snapshot.get(sequence) !== key) {
            throw new Error(`subscriber ${index} lost or replaced message ${sequence}`);
          }
        }
        for (const row of rows) {
          if (!seen.has(row.sequence)) deliver(seen, row, receivedAt);
        }
      } catch (error) {
        fatal(error);
      }
    };
    const handleInsert = row => {
      try {
        const receivedAt = performance.now();
        if (first) throw new Error(`subscriber ${index} received a change before its history`);
        deliver(seen, row, receivedAt);
      } catch (error) {
        fatal(error);
      }
    };
    return Promise.all([
      adapter.subscribe(handleSnapshot, handleInsert, fatal).then(async close => {
        if (lifetime.signal.aborted) await close();
        else stop.push(close);
      }),
      initialized,
    ]);
  }

  try {
    console.log(JSON.stringify({
      workload: "realtime-v1", system, subscribers, history, messages, interval_ms: intervalMs,
      delivery_timeout_ms: timeoutMs, drain_ms: drainMs,
    }));
    adapter = await within(connect(system, base, { history, messages, signal: lifetime.signal }),
      timeoutMs, "writer setup timed out");
    await monitored(within((async () => {
      for (let sequence = 0; sequence < history; sequence++) await adapter.write(sequence);
    })(), timeoutMs, "history seed timed out"));
    await monitored(within(Promise.all(states.map((_, index) => observe(index))),
      timeoutMs, "initial history timed out"));

    const writes = [];
    // Space sends from the preceding actual invocation so a stalled event loop cannot burst.
    for (let sequence = history; sequence < history + messages; sequence++) {
      if (sequence > history) await monitored(sleep(intervalMs));
      const sentAt = performance.now();
      started.set(sequence, sentAt);
      writes.push(adapter.write(sequence).then(
        () => writeLatencies.push(performance.now() - sentAt),
        error => { fatal(error); },
      ));
    }
    const delivered = (async () => {
      const deadline = performance.now() + timeoutMs;
      while (!lifetime.signal.aborted && deliveryLatencies.length < expected && performance.now() < deadline) {
        await sleep(10);
      }
    })();
    await monitored(Promise.all([delivered, within(Promise.all(writes), timeoutMs, "write acknowledgments timed out")]));
    if (deliveryLatencies.length === expected) await monitored(sleep(drainMs));
    if (deliveryLatencies.length !== expected || duplicates !== 0) {
      throw new Error("realtime delivery counts failed verification");
    }
  } finally {
    console.log(JSON.stringify({
      history_latency: stats(historyLatencies),
      write_ack_latency: stats(writeLatencies),
      delivery_latency: stats(deliveryLatencies),
      expected,
      received: deliveryLatencies.length,
      missed: expected - deliveryLatencies.length,
      duplicates,
    }));
    lifetime.abort();
    await Promise.allSettled([...stop.map(close => close()), adapter?.close?.()]);
  }
  console.log("verified=true");
}

await run();
