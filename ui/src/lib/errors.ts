interface CustomQueryError {
  status?: string;
  error?: unknown;
  data?: unknown;
  message?: unknown;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function appendDetails(message: string, details: unknown): string {
  if (typeof details !== "string") {
    return message;
  }

  const trimmedDetails = details.trim();
  if (!trimmedDetails || message.includes(trimmedDetails)) {
    return message;
  }

  return `${message}\n${trimmedDetails}`;
}

function tryParseJson(value: string): unknown | undefined {
  const trimmed = value.trim();
  if (
    !(trimmed.startsWith("{") && trimmed.endsWith("}")) &&
    !(trimmed.startsWith("[") && trimmed.endsWith("]"))
  ) {
    return undefined;
  }

  try {
    return JSON.parse(trimmed);
  } catch {
    return undefined;
  }
}

function formatErrorFields(value: Record<string, unknown>): string | null {
  const code = typeof value.code === "string" ? value.code.trim() : "";
  const message = typeof value.message === "string" ? value.message.trim() : "";
  if (!message) {
    return null;
  }

  const withCode = code && !message.includes(code) ? `${code}: ${message}` : message;
  return appendDetails(withCode, value.details);
}

function formatStructuredQueryError(value: unknown): string | null {
  if (!isRecord(value)) {
    return null;
  }

  if (isRecord(value.error)) {
    const nested = formatErrorFields(value.error);
    if (nested) {
      return nested;
    }
  }

  return formatErrorFields(value);
}

function formatFromText(value: string): string {
  const parsed = tryParseJson(value);
  return formatStructuredQueryError(parsed) ?? value;
}

export function getErrorMessage(error: unknown, fallback: string): string {
  if (typeof error === "string") {
    const formatted = formatFromText(error).trim();
    return formatted || fallback;
  }

  if (error instanceof Error) {
    const formatted = formatFromText(error.message).trim();
    return formatted || fallback;
  }

  if (isRecord(error)) {
    const structured = formatStructuredQueryError(error);
    if (structured) {
      return structured;
    }

    const customError = error as CustomQueryError;

    if (typeof customError.error === "string") {
      return formatFromText(customError.error);
    }

    if (isRecord(customError.data)) {
      const data = customError.data as Record<string, unknown>;
      const fromData = formatStructuredQueryError(data);
      if (fromData) {
        return fromData;
      }
      if (typeof data.message === "string") {
        return appendDetails(formatFromText(data.message), data.details);
      }
    }

    if (typeof customError.message === "string") {
      return formatFromText(customError.message);
    }
  }

  return fallback;
}

export function toSerializableErrorPayload(error: unknown): unknown {
  if (error instanceof Error) {
    const parsed = tryParseJson(error.message);
    if (parsed !== undefined) {
      return parsed;
    }

    return {
      name: error.name,
      message: error.message,
    };
  }

  if (typeof error === "string") {
    return tryParseJson(error) ?? error;
  }

  return error;
}
