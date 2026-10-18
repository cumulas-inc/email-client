import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { MockBackend, mockAdminConsentUrl } from "../../api/mock";
import { renderWithBackend } from "../../test/render";
import { AdminConsent, ApprovalLink } from "./AdminConsent";

const URL = mockAdminConsentUrl("contoso.example");
const ADMIN_PAGE = "https://flatpak.cumulas.co/admin-consent/";
const UNREACHABLE = /After approving, your browser shows a page that can't be reached/;

const steps = () =>
  Array.from(document.querySelectorAll(".consent-steps li"), (li) => li.textContent);

describe("AdminConsent", () => {
  it("opens the approval page for administrators", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend();
    const openLink = vi.spyOn(backend, "openLink").mockResolvedValue(undefined);
    renderWithBackend(<AdminConsent adminConsentUrl={URL} adminPageUrl={ADMIN_PAGE} />, {
      backend,
    });

    const open = screen.getByRole("button", { name: "Open approval page" });
    expect(open).not.toHaveClass("btn--primary");
    expect(screen.getByRole("button", { name: "Email IT" })).toHaveClass("btn--primary");
    await user.click(open);
    expect(openLink).toHaveBeenCalledWith(URL);
    expect(screen.getByText(/Email IT opens your email app/)).toHaveTextContent(
      "Email IT opens your email app with a ready-to-send request; just add your IT team's address. If you're an administrator yourself, select Open approval page and sign in with your admin account.",
    );
    expect(screen.queryByText(UNREACHABLE)).not.toBeInTheDocument();
  });

  it("warns about the page that can't be reached without a page for administrators", () => {
    renderWithBackend(
      <AdminConsent adminConsentUrl={mockAdminConsentUrl("x", true)} adminPageUrl={null} />,
    );
    expect(screen.getByText(UNREACHABLE)).toBeInTheDocument();
  });

  it("walks through Request approval when Microsoft's page offers it", () => {
    renderWithBackend(
      <AdminConsent adminConsentUrl={URL} canRequestApproval retryLabel="Turn on" />,
    );
    expect(steps()).toEqual([
      "Select Request approval. Microsoft's sign-in page opens with a form to ask your IT team.",
      "Add a short reason, such as “I use Linux and need my work mail”, and send the request.",
      "Microsoft usually emails you when your IT team responds. Once they approve it, come back and select Turn on.",
    ]);
    expect(screen.getByText("Or send the approval link to IT yourself:")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Email IT" })).not.toHaveClass("btn--primary");
  });

  it("offers mail only", async () => {
    const user = userEvent.setup();
    const onMailOnly = vi.fn();
    renderWithBackend(<AdminConsent adminConsentUrl={URL} onMailOnly={onMailOnly} />);
    expect(steps()).toEqual([
      "Send the approval link below to your IT team.",
      "Once they've approved it, come back and select Try again.",
    ]);
    await user.click(screen.getByRole("button", { name: "Try with mail only" }));
    expect(onMailOnly).toHaveBeenCalledTimes(1);
  });

  it("speaks of the feature when mail works", () => {
    renderWithBackend(
      <AdminConsent adminConsentUrl={URL} tenant="contoso.example" feature="tasks" />,
    );
    expect(
      screen.getByRole("heading", {
        name: "Your organization needs to approve access to your tasks",
      }),
    ).toBeInTheDocument();
    expect(screen.getByText(/Your mail keeps working/)).toHaveTextContent(
      "Your mail keeps working. contoso.example only lets apps use your tasks after its IT team approves them.",
    );
    expect(steps()).toEqual([
      "Send the approval link below to your IT team.",
      "Once they've approved it, Cumulas Mail picks up the approval by itself within a couple of hours. To use it right away, select Turn on.",
    ]);
  });

  it("lists the push service's permission when the link covers it", () => {
    const { unmount } = renderWithBackend(<AdminConsent adminConsentUrl={URL} />);
    expect(
      screen.getByText("Receive notices that your mail or calendar changed (instant updates)"),
    ).toBeInTheDocument();
    unmount();
    renderWithBackend(<AdminConsent adminConsentUrl={mockAdminConsentUrl("x", true)} />);
    expect(screen.queryByText(/instant updates\)/)).not.toBeInTheDocument();
    // Grouped: what mail needs, then the rest.
    expect(screen.getByRole("list", { name: "For mail" })).toBeInTheDocument();
    expect(
      screen.getByRole("list", { name: "For calendar, contacts, tasks and time zone (optional)" }),
    ).toBeInTheDocument();
  });
});

describe("ApprovalLink", () => {
  it("keeps Email IT small in a settings row, and sends its own email when given one", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend();
    const openLink = vi.spyOn(backend, "openLink").mockResolvedValue(undefined);
    renderWithBackend(
      <ApprovalLink adminConsentUrl={URL} emphasis="compact" mailto="mailto:?x" />,
      {
        backend,
      },
    );
    expect(screen.getByRole("button", { name: "Email IT" })).toHaveClass("btn", "btn--sm");
    expect(screen.getByRole("button", { name: "Open approval page" })).toHaveClass("btn--sm");
    await user.click(screen.getByRole("button", { name: "Email IT" }));
    expect(openLink).toHaveBeenCalledWith("mailto:?x");
  });
});
