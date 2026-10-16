import { QueryClient } from "@tanstack/react-query";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";

import { MockBackend } from "../api/mock";
import { App } from "./App";
import { useAppStore } from "./store";

function renderApp(backend = new MockBackend()) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(<App backend={backend} queryClient={client} />);
}

describe("App shell", () => {
  it("shows onboarding when there are no accounts", async () => {
    renderApp(new MockBackend({ empty: true }));
    expect(await screen.findByText("Welcome to Cumulas Mail")).toBeInTheDocument();
    expect(screen.queryByRole("navigation", { name: "Apps" })).not.toBeInTheDocument();
  });

  it("switches modules from the navigation rail and with Ctrl+number", async () => {
    useAppStore.setState({ module: "mail" });
    const user = userEvent.setup();
    renderApp();
    const nav = await screen.findByRole("navigation", { name: "Apps" });
    expect(nav).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: /calendar/i }));
    expect(useAppStore.getState().module).toBe("calendar");

    await user.keyboard("{Control>}3{/Control}");
    expect(useAppStore.getState().module).toBe("people");
  });
});
