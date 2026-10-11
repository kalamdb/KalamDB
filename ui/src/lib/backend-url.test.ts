// @vitest-environment jsdom

import { afterEach, describe, expect, it } from "vitest";
import { getBackendOrigin, sameLoopbackOrigin } from "./backend-url";

describe("sameLoopbackOrigin", () => {
  it("treats localhost, 127.0.0.1, and ::1 as the same server when the port and scheme match", () => {
    expect(sameLoopbackOrigin("http://127.0.0.1:2900", "http://localhost:2900")).toBe(true);
    expect(sameLoopbackOrigin("http://[::1]:2900", "http://127.0.0.1:2900")).toBe(true);
    expect(sameLoopbackOrigin("http://localhost:2900", "http://127.0.0.1:2901")).toBe(false);
    expect(sameLoopbackOrigin("https://localhost:2900", "http://127.0.0.1:2900")).toBe(false);
    expect(sameLoopbackOrigin("http://db.example:2900", "http://127.0.0.1:2900")).toBe(false);
    expect(sameLoopbackOrigin("not a url", "http://localhost:2900")).toBe(false);
  });
});

describe("getBackendOrigin", () => {
  afterEach(() => {
    delete window.__KALAMDB_RUNTIME_CONFIG__;
  });

  it("keeps the page origin when a loopback admin UI is opened under another loopback name", () => {
    const page = new URL(window.location.origin);
    const port = page.port ? `:${page.port}` : "";
    window.__KALAMDB_RUNTIME_CONFIG__ = {
      backendOrigin: `${page.protocol}//127.0.0.1${port}`,
    };
    expect(getBackendOrigin()).toBe(window.location.origin);

    window.__KALAMDB_RUNTIME_CONFIG__ = { backendOrigin: "https://db.example" };
    expect(getBackendOrigin()).toBe("https://db.example");
  });
});
