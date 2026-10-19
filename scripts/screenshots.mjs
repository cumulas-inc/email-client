// Regenerates the documentation screenshots in docs/screenshots/ from the
// in-browser demo: demo data, a fixed clock, 1440×900 at 1×, light and dark.
//
//   npm run dev                                  # the demo, in another terminal
//   node scripts/screenshots.mjs                 # every screenshot
//   node scripts/screenshots.mjs calendar tasks  # only file names containing these
//   node scripts/screenshots.mjs dark            # only the dark ones
//
// Playwright isn't one of the app's dependencies. Install it without saving
// (`npm install --no-save playwright && npx playwright install chromium`), or
// point PLAYWRIGHT at an existing install. Environment variables:
//
//   SCREENSHOTS_URL  the demo (default http://localhost:1420/, the `npm run dev` port)
//   SCREENSHOTS_OUT  output folder (default docs/screenshots)
//   PLAYWRIGHT       path to the `playwright` or `@playwright/test` package
//   CHROMIUM         a Chromium or headless-shell binary (default: Playwright's own)
//
// Use the dev server, not a build: one scene patches the demo backend's module.
// Every scene starts from a fresh page, so the demo data is the same each run.
// When you add, rename or drop a screenshot, update README.md and
// packaging/flatpak/co.cumulas.Mail.metainfo.xml to match.

import { mkdirSync } from "node:fs";
import { createRequire } from "node:module";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const BASE_URL = process.env.SCREENSHOTS_URL ?? "http://localhost:1420/";
const OUT_DIR = path.resolve(root, process.env.SCREENSHOTS_OUT ?? "docs/screenshots");
const FILTERS = process.argv.slice(2);

/** A Wednesday morning in Seattle, where the demo's Acme office is. */
const TIME_ZONE = "America/Los_Angeles";
const NOW = new Date("2026-10-14T10:24:00-07:00");

/** Whether `<name>-<theme>.png` was asked for on the command line. */
const wanted = (name, theme) =>
  FILTERS.length === 0 || FILTERS.some((f) => `${name}-${theme}`.includes(f));

function loadPlaywright() {
  const require = createRequire(import.meta.url);
  const candidates = [process.env.PLAYWRIGHT, "playwright", "@playwright/test"].filter(Boolean);
  for (const id of candidates) {
    try {
      return require(id);
    } catch {
      // Try the next one.
    }
  }
  console.error(
    "Playwright not found. Run `npm install --no-save playwright && npx playwright install chromium`,\n" +
      "or set PLAYWRIGHT to the path of an existing playwright or @playwright/test package.",
  );
  process.exit(1);
}

// --- helpers ---------------------------------------------------------------------------

const apps = (page) => page.getByRole("navigation", { name: "Apps" });

async function openModule(page, label) {
  await apps(page).getByRole("button", { name: label, exact: true }).click();
}

async function openMessage(page, subject) {
  await page.locator(".mail-row", { hasText: subject }).first().click();
  await page.getByRole("article", { name: subject }).waitFor();
}

async function openSettings(page, tab) {
  await page.getByRole("button", { name: "Main menu", exact: true }).click();
  await page.getByRole("menuitem", { name: /^Settings/ }).click();
  await page.getByRole("tab", { name: tab }).click();
}

const LAUNCH_CHECKLIST = "Atlas launch checklist — final review today";

// --- scenes ----------------------------------------------------------------------------
// Each scene runs on a fresh page, once per theme, and calls `shot(name)` for
// each of its `shots`, in order; the file is `<name>-<theme>.png`.

// The compose scenes write from the work account as most people see it, with
// sensitivity labels never turned on: the demo's account has them on, and
// its organization requires a label, which the composer would point out.
const LABELS_OFF = "?missing=sensitivityLabels";

const scenes = [
  {
    shots: ["mail", "mail-compose"],
    query: LABELS_OFF,
    async run(page, shot) {
      await openMessage(page, LAUNCH_CHECKLIST);
      await shot("mail");

      await page
        .getByRole("group", { name: "Respond" })
        .getByRole("button", { name: "Reply all" })
        .click();
      // Type once the app has put the caret in the body, or the first keys are lost.
      await page.waitForFunction(
        () => document.activeElement?.getAttribute("aria-label") === "Message body",
      );
      await page.keyboard.type(
        "Thanks Priya — the rollback plan looks good to me. Two small notes:",
      );
      await page.keyboard.press("Enter");
      await page.getByRole("button", { name: "Bulleted list" }).click();
      await page.keyboard.type("Let's link the on-call rota from the checklist");
      await page.keyboard.press("Enter");
      await page.keyboard.type("Security sign-off is due Thursday");
      await shot("mail-compose");
    },
  },
  {
    shots: ["compose-signature"],
    query: LABELS_OFF,
    async run(page, shot) {
      // A new message from the work account, which Acme signs with Cumulas Signatures.
      await page.getByRole("button", { name: "New message", exact: true }).first().click();
      await page.locator(".mail-signature iframe").waitFor();
      await page.keyboard.type("Priya");
      await page
        .getByRole("listbox", { name: "Suggested contacts" })
        .getByRole("option", { name: /Priya Raman/ })
        .waitFor();
      await page.keyboard.press("Enter");
      await page.getByRole("textbox", { name: "Subject" }).fill("Atlas launch: go for Thursday");
      await page.getByRole("textbox", { name: "Message body" }).click();
      await page.keyboard.type("Hi Priya,");
      await page.keyboard.press("Enter");
      await page.keyboard.press("Enter");
      await page.keyboard.type(
        "Security signed off this morning, so we're go for Thursday. I'll send the customer note once Mei has had a last look.",
      );
      await page.keyboard.press("Enter");
      await page.keyboard.press("Enter");
      await page.keyboard.type("Thanks for pulling the checklist together.");
      await shot("compose-signature");
    },
  },
  {
    shots: ["mail-search"],
    themes: ["light"],
    async run(page, shot) {
      await page.locator("#global-search").fill("atlas");
      // The box's suggestions close with the keyboard elsewhere; the
      // filters above the results stay, and the rows mark the word.
      await page.locator("#global-search").blur();
      await page.getByRole("listbox", { name: "Search results" }).waitFor();
      await page.getByRole("group", { name: "Search filters" }).waitFor();
      await page.locator(".mail-row mark.search-hit").first().waitFor();
      // Once the servers have answered too: the list announces the count
      // only then.
      await page
        .locator('section[aria-label="Message list"] > p[role=status]')
        .filter({ hasText: /results?\.$/ })
        .waitFor({ state: "attached" });
      await openMessage(page, "Customer announcement draft for Atlas");
      await shot("mail-search");
    },
  },
  {
    shots: ["mail-reading-pane-bottom"],
    themes: ["light"],
    async run(page, shot) {
      await openSettings(page, "Mail");
      await page.getByRole("radio", { name: "Bottom" }).check();
      await page.keyboard.press("Escape");
      await page.locator(".mail--bottom").waitFor();
      await openMessage(page, LAUNCH_CHECKLIST);
      await shot("mail-reading-pane-bottom");
    },
  },
  {
    shots: ["calendar-workweek", "calendar-details", "calendar-month", "calendar-editor"],
    async run(page, shot) {
      await openModule(page, "Calendar");
      await page.getByRole("button", { name: "Work week", exact: true }).click();
      await page.locator(".cal-ev").first().waitFor();
      await shot("calendar-workweek");

      // An invitation Alex hasn't answered yet.
      await page.locator(".cal-ev", { hasText: "Customer escalation" }).first().click();
      await page.locator(".cal-details").waitFor();
      await shot("calendar-details");
      await page.keyboard.press("Escape");

      await page.getByRole("button", { name: "Month", exact: true }).click();
      await page.locator(".cal-month").waitFor();
      await shot("calendar-month");

      await page.getByRole("button", { name: "Work week", exact: true }).click();
      await page.getByRole("button", { name: /New event/ }).click();
      await page.getByRole("dialog").waitFor();
      await page.getByPlaceholder("Add a title").fill("Roadmap review");
      await page.getByPlaceholder("Add a location").fill("Room 4.12");
      const attendees = page.getByRole("combobox", { name: "Invite attendees" });
      for (const name of ["Priya Raman", "Jonas Becker", "Mei Tanaka"]) {
        await attendees.fill(name.split(" ")[0]);
        await page.getByRole("option", { name: new RegExp(name) }).click();
      }
      await page.getByText("Teams meeting").click();
      await page
        .getByPlaceholder("Add a description")
        .fill(
          "Walk through the Q1 roadmap and agree on the top three bets.\n\nPre-read: planning doc.",
        );
      await page.getByPlaceholder("Add a title").focus();
      await shot("calendar-editor");
    },
  },
  {
    shots: ["contacts-contact", "contacts-editor"],
    async run(page, shot) {
      await openModule(page, "Contacts");
      await page.getByRole("option", { name: /Priya Raman/ }).click();
      await page.getByRole("heading", { name: "Priya Raman" }).waitFor();
      await shot("contacts-contact");

      await page.getByRole("button", { name: "Edit", exact: true }).click();
      await page.getByRole("dialog").waitFor();
      await shot("contacts-editor");
    },
  },
  {
    shots: ["tasks-scheduled", "tasks-details"],
    async run(page, shot) {
      await openModule(page, "Tasks");
      await page.getByRole("option", { name: /^Scheduled/ }).click();
      await page.getByRole("heading", { name: "Scheduled" }).waitFor();
      await shot("tasks-scheduled");

      await page.getByRole("option", { name: /^Send Q3 metrics to Dana/ }).click();
      await page.getByRole("complementary", { name: "Task details" }).waitFor();
      await shot("tasks-details");
    },
  },
  {
    shots: [
      "onboarding-welcome",
      "onboarding-microsoft",
      "onboarding-admin-consent",
      "onboarding-imap",
      "onboarding-imap-advanced",
    ],
    query: "?empty",
    async run(page, shot) {
      await page.getByRole("heading", { name: "Welcome to Cumulas Mail" }).waitFor();
      await shot("onboarding-welcome");

      await page.getByRole("button", { name: "Microsoft 365 or Outlook.com" }).click();
      await page.getByRole("button", { name: "Sign in with Microsoft" }).waitFor();
      await shot("onboarding-microsoft");

      // The demo treats "admin@…" as an organization that blocks user consent.
      await page.getByLabel("Email address (optional)").fill("admin@contoso.example");
      await page.getByRole("button", { name: "Sign in with Microsoft" }).click();
      await page.getByText("Your organization needs to approve Cumulas Mail").waitFor();
      await shot("onboarding-admin-consent");

      await page.getByRole("button", { name: "Close" }).last().click();
      await page.getByRole("button", { name: "Other email (IMAP)" }).click();
      await page.getByLabel("Email address").fill("alex.morgan@gmail.com");
      await page.getByLabel("Your name").fill("Alex Morgan");
      await page.getByLabel("Password", { exact: true }).fill("abcd efgh ijkl mnop");
      await shot("onboarding-imap");

      await page.getByRole("button", { name: /Advanced settings/ }).click();
      await page.getByLabel("User name").scrollIntoViewIfNeeded();
      await shot("onboarding-imap-advanced");
    },
  },
  {
    shots: [
      "settings-accounts",
      "settings-general",
      "settings-mail",
      "settings-signatures",
      "settings-microsoft",
      "settings-about",
    ],
    async run(page, shot) {
      await openMessage(page, LAUNCH_CHECKLIST);
      await openSettings(page, "Accounts");
      await page.getByRole("button", { name: "Manage Alex (Personal)" }).click();
      await page.getByRole("button", { name: "Change password or servers" }).waitFor();
      await shot("settings-accounts");
      for (const [tab, name] of [
        ["General", "settings-general"],
        ["Mail", "settings-mail"],
        ["Signatures", "settings-signatures"],
        ["Microsoft app registration", "settings-microsoft"],
        ["About", "settings-about"],
      ]) {
        await page.getByRole("tab", { name: tab }).click();
        if (tab === "Signatures") await page.locator(".signature-field iframe").waitFor();
        if (tab === "About") {
          // The privacy statements, whole: below the logo, they don't all fit.
          const privacy = page.getByRole("region", { name: "Privacy" });
          await privacy.waitFor();
          await page.locator(".settings__content").evaluate(
            (content, section) => {
              const top = parseFloat(getComputedStyle(content).paddingTop);
              content.scrollTop +=
                section.getBoundingClientRect().top - content.getBoundingClientRect().top - top;
            },
            await privacy.elementHandle(),
          );
        }
        await shot(name);
      }
    },
  },
  {
    shots: ["settings-org-setup", "settings-org-setup-changes"],
    // The 0.4.0 guide's objects: rules to repair, a policy in simulation, a leftover rule.
    query: "?orgsetup=partial",
    async run(page, shot) {
      await openMessage(page, LAUNCH_CHECKLIST);
      await openSettings(page, "Accounts");
      await page.getByRole("button", { name: "Manage Alex Morgan" }).click();
      await page.getByRole("button", { name: "Set up or check for your organization…" }).click();
      await page.getByRole("button", { name: "Sign in as an administrator" }).click();
      await page.getByRole("button", { name: "Sign out" }).waitFor();
      await shot("settings-org-setup");

      await page.getByRole("button", { name: "Repair", exact: true }).click();
      await page.getByRole("heading", { name: "Make these changes in Microsoft 365?" }).waitFor();
      await shot("settings-org-setup-changes");
    },
  },
  {
    shots: ["settings-account-fix"],
    async run(page, shot, { moduleUrl }) {
      // Make the demo's IMAP account fail its next sync, as if its password
      // changed. Patch the demo backend through the module URL the app loaded
      // (Vite adds a timestamp after hot updates), so it reaches the running one.
      await page.evaluate(async (url) => {
        const { MockBackend } = await import(url);
        const proto = MockBackend.prototype;
        const original = proto.getSyncStatuses;
        proto.getSyncStatuses = function () {
          const home = this.state.accounts.find((a) => a.id === "acct-home");
          if (home) home.syncError = "The server rejected the user name or password.";
          proto.getSyncStatuses = original;
          return original.call(this);
        };
      }, moduleUrl("/src/api/mock/index.ts"));
      await page.getByRole("button", { name: "Sync all accounts" }).click();
      await page.locator(".sync-status--error").waitFor();

      await openMessage(page, LAUNCH_CHECKLIST);
      await openSettings(page, "Accounts");
      await page.getByRole("button", { name: "Fix Alex (Personal)" }).click();
      await page.getByLabel("New password").fill("wrong");
      await page.getByRole("button", { name: "Save changes", exact: true }).click();
      await page.getByText(/didn't accept this email address and password/).waitFor();
      await page.locator(".settings__content").evaluate((el) => (el.scrollTop = 120));
      await shot("settings-account-fix");
    },
  },
];

// --- run -------------------------------------------------------------------------------

const { chromium } = loadPlaywright();
mkdirSync(OUT_DIR, { recursive: true });
const browser = await chromium.launch({ executablePath: process.env.CHROMIUM || undefined });
let failed = false;

for (const scene of scenes) {
  for (const theme of scene.themes ?? ["light", "dark"]) {
    if (!scene.shots.some((name) => wanted(name, theme))) continue;

    const context = await browser.newContext({
      viewport: { width: 1440, height: 900 },
      deviceScaleFactor: 1,
      colorScheme: theme,
      reducedMotion: "reduce",
      locale: "en-US",
      timezoneId: TIME_ZONE,
    });
    context.setDefaultTimeout(10_000);
    await context.clock.setFixedTime(NOW);
    const page = await context.newPage();
    const errors = [];
    page.on("pageerror", (e) => errors.push(e.message));
    const requested = [];
    page.on("request", (r) => requested.push(r.url()));
    const moduleUrl = (pathname) =>
      requested.find((url) => new URL(url).pathname === pathname) ?? pathname;

    let last = "start";
    const shot = async (name) => {
      if (!scene.shots.includes(name)) throw new Error(`"${name}" is missing from shots`);
      last = name;
      if (!wanted(name, theme)) return;
      // Let fonts and transitions settle, with the pointer out of the way.
      await page.mouse.move(1439, 899);
      await page.evaluate(() => document.fonts.ready);
      await page.waitForTimeout(400);
      const file = `${name}-${theme}.png`;
      await page.screenshot({ path: path.join(OUT_DIR, file) });
      console.log(file);
    };

    try {
      await page.goto(new URL(scene.query ?? "", BASE_URL).href);
      await page.locator(".module-switcher, .welcome").first().waitFor();
      await scene.run(page, shot, { moduleUrl });
    } catch (e) {
      failed = true;
      console.error(`${scene.shots[0]} (${theme}) failed after ${last}: ${e.message}`);
    }
    for (const e of errors) console.error(`  page error: ${e}`);
    await context.close();
  }
}

await browser.close();
process.exit(failed ? 1 : 0);
