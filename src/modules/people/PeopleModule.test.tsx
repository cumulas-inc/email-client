import { act, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { MockBackend } from "../../api/mock";
import { useAppStore } from "../../app/store";
import { useBackendEvents } from "../../app/useBackendEvents";
import { Toaster } from "../../components/Toaster";
import { useToasts } from "../../components/toast";
import { renderWithBackend } from "../../test/render";
import { PeopleModule } from "./PeopleModule";
import { usePeopleStore } from "./store";

function renderPeople(backend = new MockBackend()) {
  return renderWithBackend(
    <>
      <PeopleModule />
      <Toaster />
    </>,
    { backend },
  );
}

const optionNames = (list: HTMLElement) =>
  within(list)
    .queryAllByRole("option")
    .map((o) => o.getAttribute("aria-label")?.split(",")[0]);

const detailsHeading = () =>
  within(screen.getByRole("region", { name: "Contact details" })).getByRole("heading", {
    level: 2,
  });

beforeEach(() => {
  useAppStore.setState({ module: "people", search: "", composeRequest: null });
  usePeopleStore.setState({ selectedId: null, accountId: null });
  useToasts.setState({ toasts: [] });
});

describe("Contact list", () => {
  it("groups contacts under letter headers, sorted by name, with # last", async () => {
    renderPeople();
    const list = await screen.findByRole("listbox", { name: "Contacts" });

    // Each group is labeled by its letter header.
    const letters = within(list)
      .getAllByRole("group")
      .map((g) => document.getElementById(g.getAttribute("aria-labelledby")!)?.textContent);
    expect(letters.join(" ")).toBe("A B C D E H J L M O P R S Z #");
    expect(optionNames(within(list).getByRole("group", { name: "A" }))).toEqual([
      "Acme IT Service Desk",
      "Aisha Khan",
    ]);
    // Accents fold into their base letter; digits go under "#".
    expect(optionNames(within(list).getByRole("group", { name: "L" }))).toEqual([
      "Lucía Fernández",
    ]);
    expect(optionNames(within(list).getByRole("group", { name: "Z" }))).toEqual(["Zoë Adeyemi"]);
    expect(optionNames(within(list).getByRole("group", { name: "#" }))).toEqual([
      "24/7 Emergency Plumbing",
    ]);
    // Rows show the company, else the email address.
    expect(
      within(list).getByRole("option", { name: "Aisha Khan, Contoso Partners" }),
    ).toBeVisible();
    expect(
      within(list).getByRole("option", { name: "Ben Morgan, ben.morgan@mailbox.example" }),
    ).toBeVisible();
    // A company that only repeats the name gives way to the address.
    expect(
      within(list).getByRole("option", {
        name: "24/7 Emergency Plumbing, help@247plumbing.example",
      }),
    ).toBeVisible();
  });

  it("shows the first contact until one is picked, then follows clicks and the keyboard", async () => {
    const user = userEvent.setup();
    renderPeople();
    const list = await screen.findByRole("listbox", { name: "Contacts" });
    expect(detailsHeading()).toHaveTextContent("Acme IT Service Desk");

    await user.click(within(list).getByRole("option", { name: /^Lucía Fernández/ }));
    expect(detailsHeading()).toHaveTextContent("Lucía Fernández");
    expect(within(list).getByRole("option", { name: /^Lucía/ })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    expect(list).toHaveFocus();

    await user.keyboard("{ArrowDown}");
    expect(detailsHeading()).toHaveTextContent("Mei Tanaka");
    await user.keyboard("{ArrowUp}{ArrowUp}");
    expect(detailsHeading()).toHaveTextContent("Jonas Becker");
    await user.keyboard("{End}");
    expect(detailsHeading()).toHaveTextContent("24/7 Emergency Plumbing");
    await user.keyboard("{Home}");
    expect(detailsHeading()).toHaveTextContent("Acme IT Service Desk");
    // Type-ahead jumps by name.
    await user.keyboard("z");
    expect(detailsHeading()).toHaveTextContent("Zoë Adeyemi");
    expect(list).toHaveAttribute(
      "aria-activedescendant",
      within(list).getByRole("option", { name: /^Zoë/ }).id,
    );
  });

  it("filters live by the search box, debounced, through listContacts", async () => {
    const backend = new MockBackend();
    const spy = vi.spyOn(backend, "listContacts");
    renderPeople(backend);
    const list = await screen.findByRole("listbox", { name: "Contacts" });
    expect(optionNames(list)).toHaveLength(16);

    act(() => useAppStore.setState({ search: "f" }));
    act(() => useAppStore.setState({ search: "fab" }));
    act(() => useAppStore.setState({ search: "fabrikam" }));
    await waitFor(() => expect(optionNames(list)).toEqual(["Carlos Mendes", "Elena Petrova"]));
    // Only the settled text reaches the backend.
    expect(spy.mock.calls.map((c) => c[1])).toEqual([undefined, "fabrikam"]);
    expect(screen.getByRole("status")).toHaveTextContent("Results for “fabrikam”");
    expect(detailsHeading()).toHaveTextContent("Carlos Mendes");

    act(() => useAppStore.setState({ search: "nobody-like-this" }));
    expect(await screen.findByText("No matching contacts")).toBeInTheDocument();
    await userEvent.setup().click(screen.getByRole("button", { name: "Clear search" }));
    expect(useAppStore.getState().search).toBe("");
    await waitFor(() => expect(optionNames(screen.getByRole("listbox"))).toHaveLength(16));
  });

  it("ignores the search box while another module owns it", async () => {
    useAppStore.setState({ module: "mail", search: "fabrikam" });
    renderPeople();
    const list = await screen.findByRole("listbox", { name: "Contacts" });
    expect(optionNames(list)).toHaveLength(16);
  });

  it("filters by address book from the sidebar, with counts", async () => {
    const user = userEvent.setup();
    renderPeople();
    await screen.findByRole("listbox", { name: "Contacts" });
    const books = screen.getByRole("list", { name: "Address books" });
    const all = await within(books).findByRole("button", { name: "All contacts, 16 contacts" });
    const work = within(books).getByRole("button", {
      name: "Alex Morgan (alex.morgan@acme.example), 11 contacts",
    });
    const home = within(books).getByRole("button", {
      name: "Alex (Personal) (alex@morgan-family.example), 5 contacts",
    });
    expect(all).toHaveAttribute("aria-current", "true");
    expect(work).not.toHaveAttribute("aria-current");

    await user.click(home);
    await waitFor(() =>
      expect(optionNames(screen.getByRole("listbox", { name: "Contacts" }))).toEqual([
        "Ben Morgan",
        "Helen Morgan (Mom)",
        "Riverside Climbing Club",
        "Zoë Adeyemi",
        "24/7 Emergency Plumbing",
      ]),
    );
    expect(usePeopleStore.getState().accountId).toBe("acct-home");
    expect(home).toHaveAttribute("aria-current", "true");
    expect(all).not.toHaveAttribute("aria-current");
    // The list's title names the address book on show.
    const listPane = screen.getByRole("region", { name: "Contact list" });
    expect(within(listPane).getByText("Alex (Personal)")).toBeInTheDocument();

    await user.click(all);
    await waitFor(() =>
      expect(optionNames(screen.getByRole("listbox", { name: "Contacts" }))).toHaveLength(16),
    );
    expect(usePeopleStore.getState().accountId).toBeNull();
  });

  it("files names from other alphabets under their own letters, laid out in their direction", async () => {
    const backend = new MockBackend();
    for (const name of ["ليلى حسن", "Łukasz Nowak", "Дмитрий Иванов"]) {
      await backend.createContact({
        accountId: "acct-home",
        displayName: name,
        emails: [],
        phones: [],
      });
    }
    renderPeople(backend);
    const list = await screen.findByRole("listbox", { name: "Contacts" });
    expect(optionNames(within(list).getByRole("group", { name: "L" }))).toEqual([
      "Lucía Fernández",
      "Łukasz Nowak",
    ]);
    expect(optionNames(within(list).getByRole("group", { name: "Д" }))).toEqual(["Дмитрий Иванов"]);
    expect(optionNames(within(list).getByRole("group", { name: "ل" }))).toEqual(["ليلى حسن"]);

    const user = userEvent.setup();
    await user.click(within(list).getByRole("option", { name: "ليلى حسن" }));
    expect(within(list).getByText("ليلى حسن")).toHaveAttribute("dir", "auto");
    expect(detailsHeading()).toHaveAttribute("dir", "auto");
  });

  it("offers every contact when an address book is empty", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend();
    for (const c of await backend.listContacts("acct-home")) await backend.deleteContact(c.id);
    renderPeople(backend);
    await screen.findByRole("listbox", { name: "Contacts" });
    await user.click(screen.getByRole("button", { name: /^Alex \(Personal\)/ }));
    expect(await screen.findByText("No contacts in this account")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Show all contacts" }));
    await waitFor(() =>
      expect(optionNames(screen.getByRole("listbox", { name: "Contacts" }))).toHaveLength(11),
    );
  });

  it("shows an empty state when there are no contacts", async () => {
    renderPeople(new MockBackend({ empty: true }));
    expect(await screen.findByText("No contacts yet")).toBeInTheDocument();
    expect(screen.getByText("No contact selected")).toBeInTheDocument();
  });
});

describe("Contact details", () => {
  it("shows the contact's information", async () => {
    const user = userEvent.setup();
    renderPeople();
    await user.click(await screen.findByRole("option", { name: /^Priya Raman/ }));
    const details = screen.getByRole("region", { name: "Contact details" });
    expect(within(details).getByText("Program Manager · Atlas · Acme")).toBeInTheDocument();
    expect(within(details).getByText("alex.morgan@acme.example")).toBeInTheDocument();
    expect(within(details).getByText("Mobile")).toBeInTheDocument();
    expect(within(details).getByText("+1 (206) 555-0123")).toBeInTheDocument();
    expect(within(details).getByText("Work")).toBeInTheDocument();
    expect(within(details).getByText("November 6, 1989")).toBeInTheDocument();
    expect(within(details).getByText(/Runs the Atlas launch/)).toHaveClass("selectable");
    expect(within(details).getByText(/1 Canal Street/)).toBeInTheDocument();

    await user.click(within(details).getByRole("button", { name: "Copy mobile number" }));
    expect(await navigator.clipboard.readText()).toBe("+1 (206) 555-0123");
    expect(await screen.findByText("Phone number copied")).toBeInTheDocument();
  });

  it("starts an email from the Email button and from an address", async () => {
    const user = userEvent.setup();
    renderPeople();
    await user.click(await screen.findByRole("option", { name: /^Aisha Khan/ }));

    await user.click(screen.getByRole("button", { name: "Email" }));
    expect(useAppStore.getState().composeRequest).toEqual({
      to: [{ name: "Aisha Khan", address: "aisha.khan@contoso-partners.example" }],
      accountId: "acct-work",
    });
    expect(useAppStore.getState().module).toBe("mail");

    useAppStore.setState({ module: "people", composeRequest: null });
    await user.click(screen.getByRole("button", { name: "aisha@khan-advisory.example" }));
    expect(useAppStore.getState().composeRequest).toEqual({
      to: [{ name: "Aisha Khan", address: "aisha@khan-advisory.example" }],
      accountId: "acct-work",
    });
  });

  it("disables Email for a contact without an address", async () => {
    const backend = new MockBackend();
    await backend.createContact({
      accountId: "acct-home",
      displayName: "Aardvark Phone Only",
      emails: [],
      phones: [{ kind: "home", number: "01234" }],
    });
    renderPeople(backend);
    await screen.findByRole("listbox", { name: "Contacts" });
    expect(detailsHeading()).toHaveTextContent("Aardvark Phone Only");
    expect(screen.getByRole("button", { name: "Email" })).toBeDisabled();
  });
});

describe("Contact editor", () => {
  it("creates a contact, validating the name and email addresses", async () => {
    const user = userEvent.setup();
    const { backend } = renderPeople();
    await screen.findByRole("listbox", { name: "Contacts" });

    await user.click(screen.getByRole("button", { name: "New contact" }));
    const dialog = screen.getByRole("dialog", { name: "New contact" });
    const accountChoice = within(dialog).getByRole("radiogroup", { name: "Save to account" });
    expect(within(accountChoice).getByRole("radio", { name: /^Alex Morgan/ })).toBeChecked();

    // Nothing entered: the name is required.
    await user.click(within(dialog).getByRole("button", { name: "Add contact" }));
    expect(within(dialog).getByText("Enter a name for the contact.")).toBeInTheDocument();
    expect(within(dialog).getByLabelText("Display name")).toHaveAttribute("aria-invalid", "true");

    await user.type(within(dialog).getByLabelText("First name"), "Grace");
    await user.type(within(dialog).getByLabelText("Last name"), "Liu");
    // The display name follows first + last name.
    expect(within(dialog).getByLabelText("Display name")).toHaveValue("Grace Liu");
    expect(within(dialog).queryByText("Enter a name for the contact.")).not.toBeInTheDocument();

    await user.type(within(dialog).getByLabelText("Email address 1"), "grace@");
    await user.click(within(dialog).getByRole("button", { name: "Add contact" }));
    expect(within(dialog).getByText(/Enter a valid email address/)).toBeInTheDocument();
    await user.type(within(dialog).getByLabelText("Email address 1"), "liu.example");

    await user.click(within(dialog).getByRole("button", { name: "Add email address" }));
    await user.type(within(dialog).getByLabelText("Email address 2"), "grace@home.example");
    await user.click(within(dialog).getByRole("button", { name: "Add phone number" }));
    await user.selectOptions(within(dialog).getByLabelText("Phone 1 type"), "Work");
    await user.type(within(dialog).getByLabelText("Phone number 1"), "+1 (206) 555-0199");
    await user.type(within(dialog).getByLabelText("Company"), "Tailspin Toys");
    await user.type(within(dialog).getByLabelText("Job title"), "Producer");
    await user.type(within(dialog).getByLabelText("Department"), "Games");
    await user.type(within(dialog).getByLabelText("Birthday"), "1990-05-04");
    await user.type(within(dialog).getByLabelText("Address"), "1 Main Street");
    await user.type(within(dialog).getByLabelText("Notes"), "Met at the conference.");
    await user.click(within(dialog).getByRole("button", { name: "Add contact" }));

    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
    expect(detailsHeading()).toHaveTextContent("Grace Liu");
    expect(screen.getByRole("option", { name: "Grace Liu, Tailspin Toys" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    expect(await screen.findByText("Grace Liu was added to your contacts")).toBeInTheDocument();

    const saved = (await backend.listContacts(undefined, "grace"))[0]!;
    expect(saved).toMatchObject({
      accountId: "acct-work",
      displayName: "Grace Liu",
      givenName: "Grace",
      surname: "Liu",
      emails: [
        { name: null, address: "grace@liu.example" },
        { name: null, address: "grace@home.example" },
      ],
      phones: [{ kind: "business", number: "+1 (206) 555-0199" }],
      company: "Tailspin Toys",
      jobTitle: "Producer",
      department: "Games",
      birthday: "1990-05-04",
      address: "1 Main Street",
      notes: "Met at the conference.",
    });
  });

  it("creates in the chosen account and clears filters hiding the new contact", async () => {
    const user = userEvent.setup();
    renderPeople();
    const list = await screen.findByRole("listbox", { name: "Contacts" });
    act(() => useAppStore.setState({ search: "fabrikam" }));
    await waitFor(() => expect(optionNames(list)).toHaveLength(2));

    await user.click(screen.getByRole("button", { name: "New contact" }));
    const dialog = screen.getByRole("dialog");
    await user.click(within(dialog).getByRole("radio", { name: /^Alex \(Personal\)/ }));
    await user.type(within(dialog).getByLabelText("Display name"), "Uncle Rob");
    await user.click(within(dialog).getByRole("button", { name: "Add contact" }));

    await waitFor(() => expect(detailsHeading()).toHaveTextContent("Uncle Rob"));
    expect(useAppStore.getState().search).toBe("");
    const details = screen.getByRole("region", { name: "Contact details" });
    expect(within(details).getByText("alex@morgan-family.example")).toBeInTheDocument();
  });

  it("edits a contact and keeps a custom display name", async () => {
    const user = userEvent.setup();
    const { backend } = renderPeople();
    await user.click(await screen.findByRole("option", { name: /^Helen Morgan/ }));
    await user.click(screen.getByRole("button", { name: "Edit" }));

    const dialog = screen.getByRole("dialog", { name: "Edit contact" });
    expect(within(dialog).queryByLabelText("Save to account")).not.toBeInTheDocument();
    expect(within(dialog).getByText("alex@morgan-family.example")).toBeInTheDocument();
    const first = within(dialog).getByLabelText("First name");
    await user.clear(first);
    await user.type(first, "Helena");
    // "Helen Morgan (Mom)" was chosen by hand, so it is not overwritten.
    expect(within(dialog).getByLabelText("Display name")).toHaveValue("Helen Morgan (Mom)");
    await user.type(within(dialog).getByLabelText("Job title"), "Retired teacher");
    await user.click(within(dialog).getByRole("button", { name: "Remove phone number 1" }));
    await user.click(within(dialog).getByRole("button", { name: "Save changes" }));

    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
    expect(screen.getByText("Retired teacher")).toBeInTheDocument();
    expect(await screen.findByText("Contact saved")).toBeInTheDocument();
    const [helen] = await backend.listContacts(undefined, "helen");
    expect(helen).toMatchObject({
      givenName: "Helena",
      displayName: "Helen Morgan (Mom)",
      jobTitle: "Retired teacher",
      phones: [],
    });
  });

  it("shows backend errors inside the dialog", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend();
    vi.spyOn(backend, "updateContact").mockRejectedValueOnce({
      code: "network",
      message: "You're offline.",
    });
    renderPeople(backend);
    await user.click(await screen.findByRole("option", { name: /^Ben Morgan/ }));
    await user.click(screen.getByRole("button", { name: "Edit" }));
    await user.click(screen.getByRole("button", { name: "Save changes" }));
    expect(await within(screen.getByRole("dialog")).findByRole("alert")).toHaveTextContent(
      "You're offline.",
    );
  });

  it("cancels without saving", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend();
    const spy = vi.spyOn(backend, "createContact");
    renderPeople(backend);
    await screen.findByRole("listbox", { name: "Contacts" });
    await user.click(screen.getByRole("button", { name: "New contact" }));
    await user.type(screen.getByLabelText("First name"), "Nobody");
    await user.click(screen.getByRole("button", { name: "Cancel" }));
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(spy).not.toHaveBeenCalled();
  });

  it("opens with Ctrl+N, and the avatar follows the name", async () => {
    const user = userEvent.setup();
    renderPeople();
    await screen.findByRole("listbox", { name: "Contacts" });
    await user.keyboard("{Control>}n{/Control}");
    const dialog = screen.getByRole("dialog", { name: "New contact" });
    await user.type(within(dialog).getByLabelText("First name"), "Grace");
    await user.type(within(dialog).getByLabelText("Last name"), "Liu");
    expect(within(dialog).getByText("GL")).toBeInTheDocument();
    // Ctrl+N inside the dialog doesn't open another one.
    await user.keyboard("{Control>}n{/Control}");
    expect(screen.getAllByRole("dialog")).toHaveLength(1);
  });
});

describe("Deleting contacts", () => {
  it("asks for confirmation, deletes and selects the next contact", async () => {
    const user = userEvent.setup();
    const { backend } = renderPeople();
    await user.click(await screen.findByRole("option", { name: /^Ben Morgan/ }));
    await user.click(screen.getByRole("button", { name: "Delete" }));

    const dialog = screen.getByRole("dialog", { name: "Delete contact?" });
    expect(dialog).toHaveTextContent("Ben Morgan will be permanently deleted");
    await user.click(within(dialog).getByRole("button", { name: "Delete contact" }));

    await waitFor(() =>
      expect(screen.queryByRole("option", { name: /^Ben Morgan/ })).not.toBeInTheDocument(),
    );
    expect(detailsHeading()).toHaveTextContent("Carlos Mendes");
    expect(await backend.listContacts(undefined, "ben")).toEqual([]);
    expect(await screen.findByText("Ben Morgan was deleted")).toBeInTheDocument();
  });

  it("keeps the contact when canceled, and supports the Delete key", async () => {
    const user = userEvent.setup();
    const { backend } = renderPeople();
    await user.click(await screen.findByRole("option", { name: /^Elena Petrova/ }));
    await user.keyboard("{Delete}");
    const dialog = screen.getByRole("dialog", { name: "Delete contact?" });
    await user.click(within(dialog).getByRole("button", { name: "Cancel" }));
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(await backend.listContacts(undefined, "elena")).toHaveLength(1);
  });
});

describe("contacts permission", () => {
  const EMAIL = "alex.morgan@acme.example";

  /** Applies the backend's data changes, as the app does. */
  function Events() {
    useBackendEvents();
    return null;
  }

  function renderWithEvents(backend: MockBackend) {
    return renderWithBackend(
      <>
        <PeopleModule />
        <Events />
      </>,
      { backend },
    );
  }

  it("leaves an account without contacts permission out of new contacts", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend({ missing: ["contacts"] });
    renderWithEvents(backend);

    const notice = await screen.findByRole("region", { name: "Contacts need permission" });
    expect(
      within(notice).getByRole("button", { name: `Turn on contacts for ${EMAIL}` }),
    ).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "New contact" }));
    const dialog = screen.getByRole("dialog", { name: "New contact" });
    const choice = within(dialog).getByRole("radiogroup", { name: "Save to account" });
    const radios = within(choice).getAllByRole("radio");
    expect(radios).toHaveLength(1);
    expect(radios[0]!.closest("label")).toHaveTextContent("alex@morgan-family.example");
    expect(radios[0]).toBeChecked();
  });

  it("explains an address book it can't read, and offers no new contact for it", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend({ missing: ["contacts"] });
    renderWithEvents(backend);

    await screen.findByRole("region", { name: "Contacts need permission" });
    await user.click(screen.getByRole("button", { name: /^Alex Morgan \(/ }));
    const listPane = screen.getByRole("region", { name: "Contact list" });
    expect(
      await within(listPane).findByText("This account's contacts need permission"),
    ).toBeInTheDocument();
    expect(
      within(listPane).getByText(
        `Turn them on, or ask your IT team to approve them, under Address books to see the contacts of ${EMAIL} here.`,
      ),
    ).toBeInTheDocument();
    expect(within(listPane).queryByText("No contacts in this account")).toBeNull();
    // New contacts would go to another account.
    expect(within(listPane).queryByRole("button", { name: "New contact" })).toBeNull();

    await user.click(within(listPane).getByRole("button", { name: "Show all contacts" }));
    expect(await within(listPane).findByRole("listbox", { name: "Contacts" })).toBeInTheDocument();
  });

  it("says when Microsoft 365 refused an address book", async () => {
    const user = userEvent.setup();
    renderWithEvents(new MockBackend({ refused: ["contacts"] }));

    await screen.findByRole("region", { name: "Microsoft 365 refused access to your contacts" });
    await user.click(screen.getByRole("button", { name: /^Alex Morgan \(/ }));
    const listPane = screen.getByRole("region", { name: "Contact list" });
    expect(
      await within(listPane).findByText("Microsoft 365 refused access to your contacts"),
    ).toBeInTheDocument();
    expect(within(listPane).queryByRole("button", { name: "New contact" })).toBeNull();
  });

  it("disables New contact while no account can take contacts, until one can", async () => {
    const backend = new MockBackend({ refused: ["contacts"] });
    await backend.removeAccount("acct-home");
    renderWithEvents(backend);

    await screen.findByRole("region", { name: "Microsoft 365 refused access to your contacts" });
    const create = screen.getByRole("button", { name: "New contact" });
    expect(create).toBeDisabled();
    expect(create).toHaveAccessibleDescription("Microsoft 365 refused access to your contacts");

    act(() => backend.grantFeature("acct-work", "contacts"));
    const sidebar = screen.getByRole("complementary", { name: "Address books" });
    await waitFor(() =>
      expect(within(sidebar).getByRole("button", { name: "New contact" })).toBeEnabled(),
    );
    expect(screen.queryByRole("region", { name: /refused access to your contacts/ })).toBeNull();
  });
});
