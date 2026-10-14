import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, type RenderOptions } from "@testing-library/react";
import type { ReactElement } from "react";

import { BackendProvider } from "../api/BackendProvider";
import type { Backend } from "../api/backend";
import { MockBackend } from "../api/mock";

/** Renders UI with a fresh demo backend and query client. */
export function renderWithBackend(
  ui: ReactElement,
  { backend = new MockBackend(), ...options }: { backend?: Backend } & RenderOptions = {},
) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false, staleTime: Infinity }, mutations: { retry: 0 } },
  });
  const result = render(
    <BackendProvider backend={backend}>
      <QueryClientProvider client={client}>{ui}</QueryClientProvider>
    </BackendProvider>,
    options,
  );
  return { ...result, backend, client };
}
