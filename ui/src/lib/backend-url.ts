function trimTrailingSlashes(value: string): string {
  return value.replace(/\/+$/, "");
}

function isLoopbackHost(hostname: string): boolean {
  const host = hostname.toLowerCase().replace(/^\[|\]$/g, "");
  return host === "localhost" || host === "127.0.0.1" || host === "::1";
}

/** Same loopback server opened by a different host name, such as 127.0.0.1 vs localhost. */
export function sameLoopbackOrigin(pageOrigin: string, configuredOrigin: string): boolean {
  try {
    const page = new URL(pageOrigin);
    const configured = new URL(configuredOrigin);
    return page.protocol === configured.protocol
      && page.port === configured.port
      && isLoopbackHost(page.hostname)
      && isLoopbackHost(configured.hostname);
  } catch {
    return false;
  }
}

function resolveConfiguredOrigin(): string | null {
  if (typeof window !== "undefined") {
    const runtimeConfigured = window.__KALAMDB_RUNTIME_CONFIG__?.backendOrigin?.trim();
    if (runtimeConfigured) {
      return trimTrailingSlashes(runtimeConfigured);
    }
  }

  if (!import.meta.env.DEV) {
    return null;
  }

  const configured = import.meta.env.VITE_API_URL?.trim();
  if (!configured) {
    return null;
  }

  return trimTrailingSlashes(configured);
}

function resolveDefaultOrigin(): string {
  if (import.meta.env.DEV) {
    return "http://localhost:2900";
  }

  if (typeof window !== "undefined") {
    return trimTrailingSlashes(window.location.origin);
  }

  return "http://localhost:2900";
}

export function getBackendOrigin(): string {
  const configuredOrigin = resolveConfiguredOrigin();
  if (
    configuredOrigin
    && typeof window !== "undefined"
    && sameLoopbackOrigin(window.location.origin, configuredOrigin)
  ) {
    return trimTrailingSlashes(window.location.origin);
  }
  if (configuredOrigin) {
    return configuredOrigin;
  }

  return resolveDefaultOrigin();
}

export function getApiBaseUrl(): string {
  return `${getBackendOrigin()}/v1/api`;
}
