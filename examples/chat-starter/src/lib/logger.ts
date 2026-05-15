import pino, { type LoggerOptions } from "pino";

// Shared structured logger for the agent, backend, and setup script.
// We deliberately do NOT use pino.transport(): transports run in a worker
// thread which keeps the Node event loop alive and breaks `node --test`
// process exit. Instead we attach pino-pretty inline (sync) for dev,
// and emit raw JSON to stdout in prod.

const isDev = process.env.NODE_ENV !== "production";
const isTest = process.env.NODE_ENV === "test" || process.env.VITEST === "true";

const options: LoggerOptions = {
  level: process.env.LOG_LEVEL ?? (isTest ? "silent" : isDev ? "debug" : "info"),
  base: { service: process.env.LOG_SERVICE },
  redact: {
    paths: [
      "password",
      "*.password",
      "authorization",
      "*.authorization",
      "token",
      "*.token",
      "apiKey",
      "*.apiKey",
    ],
    censor: "[REDACTED]",
  },
};

function buildStream(): NodeJS.WritableStream {
  if (!isDev || isTest) {
    return process.stdout;
  }
  // Lazily require pino-pretty so the agent image doesn't need it in prod.
  // eslint-disable-next-line @typescript-eslint/no-require-imports
  const pretty = require("pino-pretty") as (opts: Record<string, unknown>) => NodeJS.WritableStream;
  return pretty({
    colorize: true,
    translateTime: "HH:MM:ss.l",
    ignore: "pid,hostname,service",
    singleLine: true,
  });
}

export const logger = pino(options, buildStream());

export type Logger = typeof logger;
