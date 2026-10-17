// Runs the inline script of the page for IT administrators
// (admin-consent.html) for each answer Microsoft can bring an administrator
// back with, against stand-ins for the few browser objects the page may use:
// document.getElementById (elements with hidden and textContent only, and
// the error code's link its href), document.title and location.search.
// Anything else the script touches fails the check. It must never show the
// text of the address, which anyone can write: only an AADSTS code and an
// OAuth error token. Called by check-admin-consent.sh; needs no
// dependencies.
//
// Usage: node check-admin-consent.mjs PAGE

import { readFileSync } from "node:fs";

const page = process.argv[2];
if (!page) {
  console.error("usage: node check-admin-consent.mjs PAGE");
  process.exit(2);
}
const html = readFileSync(page, "utf8");

/** The ids the script may show, hide or fill in. */
const IDS = [
  "page-title",
  "admin",
  "approved",
  "failed",
  "failed-description",
  "failed-error-note",
  "failed-error",
  "failed-lookup",
  "failed-declined",
  "failed-other",
];
const APPROVED = "Cumulas Mail is approved";
const FAILED = "Cumulas Mail wasn't approved";
const NO_REASON = "Microsoft didn't say why";
const LOOKUP = "https://login.microsoftonline.com/error";

let failures = 0;
const ok = (what) => console.error(`ok - ${what}`);
const fail = (what, why) => {
  console.error(`FAIL - ${what}${why ? `: ${why}` : ""}`);
  failures += 1;
};

const scripts = [...html.matchAll(/<script\b([^>]*)>([\s\S]*?)<\/script>/gi)];
if (scripts.length !== 1 || scripts[0][1].trim() !== "") {
  console.error(`FAIL - ${page} has one inline <script> without attributes`);
  process.exit(1);
}
const script = scripts[0][2];

/** The page's element with this id as it loads: whether it's hidden, its text and link. */
function initialElement(id) {
  const tag = new RegExp(`<([a-z0-9]+)\\b[^>]*\\bid="${id}"[^>]*>`, "i").exec(html);
  if (!tag) return null;
  const hidden = /\shidden(\s|=|\/?>)/i.test(tag[0]);
  const href = /\shref="([^"]*)"/i.exec(tag[0])?.[1];
  const rest = html.slice(tag.index + tag[0].length);
  const close = rest.search(new RegExp(`</${tag[1]}\\s*>`, "i"));
  const textContent = rest
    .slice(0, close)
    .replace(/<[^>]*>/g, "")
    .replace(/\s+/g, " ")
    .trim();
  return href === undefined ? { hidden, textContent } : { hidden, textContent, href };
}

const initial = new Map(IDS.map((id) => [id, initialElement(id)]));
for (const [id, element] of initial) {
  if (element === null) {
    console.error(`FAIL - ${page} has an element with id="${id}"`);
    process.exit(1);
  }
}
const initialTitle = /<title>([^<]*)<\/title>/i.exec(html)?.[1].trim();

/** Only these properties can be read or written; anything else throws. */
function stub(name, target, allowed) {
  const check = (property) => {
    if (!allowed.includes(property)) throw new Error(`the script used ${name}.${String(property)}`);
  };
  return new Proxy(target, {
    get(object, property) {
      check(property);
      return object[property];
    },
    set(object, property, value) {
      check(property);
      object[property] = value;
      return true;
    },
    has(object, property) {
      check(property);
      return property in object;
    },
    defineProperty(_object, property) {
      throw new Error(`the script defined ${name}.${String(property)}`);
    },
    deleteProperty(_object, property) {
      throw new Error(`the script deleted ${name}.${String(property)}`);
    },
  });
}

/** Runs the script as the page at `?search` would, and returns what it left. */
function run(search) {
  const elements = new Map();
  for (const [id, element] of initial) elements.set(id, { ...element });
  const proxies = new Map();
  for (const [id, element] of elements) {
    const allowed =
      id === "failed-lookup" ? ["hidden", "textContent", "href"] : ["hidden", "textContent"];
    proxies.set(id, stub(`#${id}`, element, allowed));
  }
  const documentState = {
    title: initialTitle,
    getElementById: (id) => {
      if (!proxies.has(id)) throw new Error(`the script asked for #${id}`);
      return proxies.get(id);
    },
  };
  const document = stub("document", documentState, ["getElementById", "title"]);
  const location = stub("location", { search }, ["search"]);
  // The browser globals a page could reach the network or the page's HTML
  // with are undefined here, so a script that uses them fails.
  const blocked = [
    "window",
    "self",
    "globalThis",
    "fetch",
    "XMLHttpRequest",
    "navigator",
    "WebSocket",
    "EventSource",
    "Image",
    "localStorage",
    "sessionStorage",
    "indexedDB",
    "history",
    "setTimeout",
    "setInterval",
    "require",
    "process",
  ];
  // The check runs the page's script itself, on purpose: the page never does this.
  const body = new Function("document", "location", "URLSearchParams", ...blocked, script);
  body(document, location, URLSearchParams);
  const state = (id) => elements.get(id);
  return { state, title: documentState.title };
}

/** Runs a case and its expectations, each [description, actual, expected]. */
function expect(what, search, expectations) {
  let result;
  try {
    result = run(search);
  } catch (e) {
    fail(what, e.message);
    return;
  }
  const wrong = expectations(result)
    .filter(([, actual, expected]) => actual !== expected)
    .map(
      ([name, actual, expected]) =>
        `${name} is ${JSON.stringify(actual)}, not ${JSON.stringify(expected)}`,
    );
  if (wrong.length === 0) ok(what);
  else fail(what, wrong.join("; "));
}

const shown = ({ state }, id) => [`#${id} shown`, state(id).hidden, false];
const hidden = ({ state }, id) => [`#${id} hidden`, state(id).hidden, true];
const text = ({ state }, id, expected) => [`#${id}'s text`, state(id).textContent, expected];
const title = (r, expected) => ["the title", r.title, expected];
const lookup = ({ state }, expected) => [
  "#failed-lookup's link",
  state("failed-lookup").href,
  expected,
];
/** That no element shows `text`, which only the address had. */
const nowhere = ({ state }, text) => [
  `the text "${text}" shown`,
  IDS.some((id) => state(id).textContent.includes(text)),
  false,
];

expect("no answer: the page for administrators", "", (r) => [
  shown(r, "admin"),
  hidden(r, "approved"),
  hidden(r, "failed"),
  hidden(r, "failed-error-note"),
  hidden(r, "failed-lookup"),
  hidden(r, "failed-declined"),
  hidden(r, "failed-other"),
  title(r, initialTitle),
  text(r, "page-title", initial.get("page-title").textContent),
]);

for (const search of ["?admin_consent=True&tenant=x&scope=y", "?admin_consent=true"]) {
  expect(`approved (${search})`, search, (r) => [
    shown(r, "approved"),
    hidden(r, "admin"),
    hidden(r, "failed"),
    text(r, "page-title", APPROVED),
    title(r, APPROVED),
  ]);
}

expect(
  "Microsoft's failure, which says admin_consent=True too",
  "?admin_consent=True&error=consent_required&error_description=AADSTS65004%3a+The+resource+owner+or+authorization+server+denied+the+request.%0d%0aTrace+ID%3a+x",
  (r) => [
    shown(r, "failed"),
    hidden(r, "approved"),
    shown(r, "admin"),
    shown(r, "failed-error-note"),
    text(r, "failed-error", "consent_required"),
    text(r, "failed-description", "Microsoft error code AADSTS65004"),
    shown(r, "failed-lookup"),
    lookup(r, `${LOOKUP}?code=65004`),
    nowhere(r, "resource owner"),
    shown(r, "failed-declined"),
    hidden(r, "failed-other"),
    title(r, FAILED),
    text(r, "page-title", initial.get("page-title").textContent),
  ],
);

expect(
  "markup in the description isn't shown",
  "?error=access_denied&error_description=%3Cimg%20src%3Dx%20onerror%3Dalert(1)%3E%0D%0ATrace%20ID%3A%20y",
  (r) => [
    shown(r, "failed"),
    text(r, "failed-description", NO_REASON),
    nowhere(r, "img"),
    hidden(r, "failed-lookup"),
    text(r, "failed-error", "access_denied"),
    shown(r, "failed-declined"),
    hidden(r, "failed-other"),
  ],
);

expect("an error without a description", "?error=invalid_request", (r) => [
  shown(r, "failed"),
  text(r, "failed-description", NO_REASON),
  hidden(r, "failed-lookup"),
  shown(r, "failed-error-note"),
  text(r, "failed-error", "invalid_request"),
  hidden(r, "failed-declined"),
  shown(r, "failed-other"),
  hidden(r, "approved"),
  shown(r, "admin"),
  title(r, FAILED),
]);

expect("an error wins over admin_consent", "?admin_consent=False&error=x", (r) => [
  shown(r, "failed"),
  hidden(r, "approved"),
  shown(r, "admin"),
  shown(r, "failed-other"),
  title(r, FAILED),
]);

expect(
  "a description that isn't Microsoft's code shows nothing of it",
  "?error=x&error_description=Your+approval+expired.+Re-approve+at+https%3A%2F%2Fconsent.example+within+24h",
  (r) => [
    shown(r, "failed"),
    text(r, "failed-description", NO_REASON),
    nowhere(r, "consent.example"),
    hidden(r, "failed-lookup"),
    shown(r, "failed-other"),
  ],
);

expect(
  "only the code of a description starting with one",
  `?error=server_error&error_description=AADSTS90094%3A+Re-approve+at+https%3A%2F%2Fconsent.example${"a".repeat(600)}`,
  (r) => [
    text(r, "failed-description", "Microsoft error code AADSTS90094"),
    lookup(r, `${LOOKUP}?code=90094`),
    nowhere(r, "consent.example"),
    text(r, "failed-error", "server_error"),
    shown(r, "failed-other"),
  ],
);

expect(
  "an error that isn't an OAuth error token isn't shown",
  "?error=Re-approve+at+https%3A%2F%2Fconsent.example&error_description=AADSTS65004xyz",
  (r) => [
    shown(r, "failed"),
    hidden(r, "failed-error-note"),
    text(r, "failed-error", ""),
    text(r, "failed-description", NO_REASON),
    hidden(r, "failed-lookup"),
    nowhere(r, "consent.example"),
    shown(r, "failed-other"),
    title(r, FAILED),
  ],
);

if (failures > 0) {
  console.error(`${failures} check${failures === 1 ? "" : "s"} of ${page} failed.`);
  process.exit(1);
}
