// What the app store remembers on this device, read as the page loads.

import { afterEach, describe, expect, it, vi } from "vitest";

import { DISMISSED_UPDATE_KEY, storedDismissedUpdate } from "./store";

afterEach(() => {
  vi.restoreAllMocks();
  window.localStorage.clear();
});

describe("the update whose card was put off", () => {
  it("is read back", () => {
    expect(storedDismissedUpdate()).toBeNull();
    window.localStorage.setItem(DISMISSED_UPDATE_KEY, "demo-commit-2");
    expect(storedDismissedUpdate()).toBe("demo-commit-2");
  });

  it("is none when storage can't be read", () => {
    window.localStorage.setItem(DISMISSED_UPDATE_KEY, "demo-commit-2");
    vi.spyOn(Storage.prototype, "getItem").mockImplementation(() => {
      throw new DOMException("The operation is insecure.", "SecurityError");
    });
    expect(storedDismissedUpdate()).toBeNull();
  });

  it("is where the store starts", async () => {
    window.localStorage.setItem(DISMISSED_UPDATE_KEY, "demo-commit-2");
    // A page load: the store module evaluated afresh.
    vi.resetModules();
    const { useAppStore } = await import("./store");
    expect(useAppStore.getState().dismissedUpdate).toBe("demo-commit-2");
  });
});
