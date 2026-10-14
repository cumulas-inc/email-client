import "@testing-library/jest-dom/vitest";
import { cleanup, configure } from "@testing-library/react";
import { afterEach } from "vitest";

// Full-module tests render a lot; give async queries headroom on busy CI
// machines instead of failing on the 1 s default.
configure({ asyncUtilTimeout: 4000 });

afterEach(() => {
  cleanup();
});
