interface CustomQueryError {
  status?: string;
  error?: unknown;
  data?: unknown;
  message?: unknown;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
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

export function getErrorMessage(error: unknown, fallback: string): string {
  if (typeof error === "string") {
    return error;
  }

  if (error instanceof Error) {
    return error.message || fallback;
  }

  if (isRecord(error)) {
    const customError = error as CustomQueryError;

    if (typeof customError.error === "string") {
      return customError.error;
    }

    if (isRecord(customError.error)) {
      const nestedError = customError.error as Record<string, unknown>;
      if (typeof nestedError.message === "string") {
        return appendDetails(nestedError.message, nestedError.details);
      }
    }

    if (isRecord(customError.data)) {
      const data = customError.data as Record<string, unknown>;
      if (typeof data.message === "string") {
        return appendDetails(data.message, data.details);
      }
      if (isRecord(data.error) && typeof data.error.message === "string") {
        const nestedError = data.error as Record<string, unknown>;
        return appendDetails(nestedError.message as string, nestedError.details);
      }
    }

    if (typeof customError.message === "string") {
      return customError.message;
    }
  }

  return fallback;
}