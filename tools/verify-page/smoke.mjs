// Smoke test of the verify page (docs/m1-store-spec.md section 9): every
// export case under test-vectors/store/exports through the page's folder input
// in headless Chromium, then case 004 through the files input in a shuffled
// order and case 102 dropped as a folder, each compared with the case's
// expected report. It also fails on any console or page error (a CSP
// violation included), on a request for anything but the page's own five
// files, and on a markup or code sink in page.mjs.
//
//   sh tools/build-wasm.sh
//   node tools/verify-page/smoke.mjs
//
// Playwright is loaded with require, so a global install on NODE_PATH or a
// node_modules folder above this file works. The script serves the repository
// root itself on 127.0.0.1. It never skips: a missing Playwright, browser or
// module is a failure. Exit 0 when every run matches, 1 otherwise.

import { createReadStream, existsSync, readFileSync, readdirSync, realpathSync, statSync } from "node:fs";
import { createServer } from "node:http";
import { createRequire } from "node:module";
import { once } from "node:events";
import { dirname, extname, join, sep } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const ROOT = realpathSync(join(dirname(fileURLToPath(import.meta.url)), "..", ".."));
const PAGE_DIR = join(ROOT, "tools", "verify-page");
const PKG = join(ROOT, "crates", "evidence-wasm", "pkg");
const CASES = join(ROOT, "test-vectors", "store", "exports");
const PAGE_PATH = "/tools/verify-page/";
const CSP =
  "default-src 'self'; script-src 'self' 'wasm-unsafe-eval'; object-src 'none'; base-uri 'none'; form-action 'none'";
// The only requests the page may make, all to its own origin.
const PAGE_FILES = [
  PAGE_PATH,
  `${PAGE_PATH}page.css`,
  `${PAGE_PATH}page.mjs`,
  "/crates/evidence-wasm/pkg/evidence.mjs",
  "/crates/evidence-wasm/pkg/evidence_wasm.wasm",
];
// Ways to turn text into markup or code, none of which page.mjs may use.
const SINKS = /\b(innerHTML|outerHTML|insertAdjacentHTML|eval|Function)\b|document\s*\.\s*write/g;
const SEGMENT_NAME = /^[0-9]{6}\.jsonl$/;
const DEFAULT_MAX_FINDINGS = 1000;
const TIMEOUT_MS = 30_000;
// Case 004 through the files input, in this order rather than the manifest's.
const SHUFFLED_004 = ["000003.jsonl", "manifest.json", "000001.jsonl", "000004.jsonl", "000002.jsonl"];
// The case dropped as a folder: four segment files and findings.
const DROPPED_CASE = "102-entry-removed";

const failures = [];
const fail = (message) => failures.push(message);

function stop(message) {
  console.error(`page smoke: ${message}`);
  process.exit(1);
}

// --- Prerequisites -----------------------------------------------------------------

const require = createRequire(import.meta.url);
let chromium;
try {
  ({ chromium } = require("playwright"));
} catch (error) {
  stop(`Playwright is not installed (${error.code ?? error.message}); install it, or set NODE_PATH to its node_modules`);
}
for (const file of ["evidence.mjs", "evidence_wasm.wasm"]) {
  if (!existsSync(join(PKG, file))) stop(`crates/evidence-wasm/pkg/${file} is missing; run sh tools/build-wasm.sh`);
}
const { load } = await import(pathToFileURL(join(PKG, "evidence.mjs")).href);
const ev = await load();
const canonical = (value) => ev.canonicalize(JSON.stringify(value));

// --- Static checks of the page's files -------------------------------------------------

const pageScript = readFileSync(join(PAGE_DIR, "page.mjs"), "utf8");
for (const match of pageScript.matchAll(SINKS)) fail(`page.mjs uses ${match[0]}`);

const html = readFileSync(join(PAGE_DIR, "index.html"), "utf8");
const cspTags = [...html.matchAll(/<meta\s+http-equiv="Content-Security-Policy"\s+content="([^"]*)"\s*>/gi)];
if (cspTags.length !== 1 || cspTags[0][1] !== CSP) fail("index.html must have exactly the one CSP meta tag of section 9");
for (const tag of html.match(/<script\b[^>]*>/gi) ?? []) {
  if (!/\ssrc="[^"]+"/.test(tag)) fail(`index.html has an inline script: ${tag}`);
}
if (/<style\b/i.test(html)) fail("index.html has a <style> element");
if (/\sstyle\s*=/i.test(html)) fail("index.html has a style attribute");
if (/\son[a-z]+\s*=/i.test(html)) fail("index.html has an event handler attribute");

// --- A static server for the repository root ----------------------------------------------

const TYPES = {
  ".html": "text/html; charset=utf-8",
  ".mjs": "text/javascript; charset=utf-8",
  ".css": "text/css; charset=utf-8",
  ".json": "application/json",
  ".wasm": "application/wasm",
};

function serve(request, response) {
  const refuse = (status) => {
    response.writeHead(status, { "Content-Type": "text/plain" });
    response.end(`${status}\n`);
  };
  if (request.method !== "GET") {
    response.setHeader("Allow", "GET");
    refuse(405);
    return;
  }
  let file;
  try {
    const path = decodeURIComponent(new URL(request.url, "http://127.0.0.1").pathname);
    file = realpathSync(join(ROOT, path));
    if (statSync(file).isDirectory()) file = realpathSync(join(file, "index.html"));
  } catch {
    refuse(404);
    return;
  }
  if (!file.startsWith(ROOT + sep)) {
    refuse(403);
    return;
  }
  if (!statSync(file).isFile()) {
    refuse(404);
    return;
  }
  response.writeHead(200, {
    "Content-Type": TYPES[extname(file)] ?? "application/octet-stream",
    "Cache-Control": "no-store",
  });
  createReadStream(file).pipe(response);
}

const server = createServer(serve);
server.listen(0, "127.0.0.1");
await once(server, "listening");
const origin = `http://127.0.0.1:${server.address().port}`;

// --- The browser ----------------------------------------------------------------------

let browser;
try {
  browser = await chromium.launch();
} catch (error) {
  server.close();
  stop(`Chromium did not start (${error.message.split("\n")[0]}); run: playwright install chromium`);
}

const requested = new Set();
let runs = 0;
try {
  const page = await browser.newPage();
  page.on("console", (message) => {
    if (message.type() === "error") fail(`console error: ${message.text()}`);
  });
  page.on("pageerror", (error) => fail(`page error: ${error.message}`));
  page.on("dialog", (dialog) => {
    fail(`dialog: ${dialog.message()}`);
    dialog.dismiss().catch(() => {});
  });
  page.on("request", (request) => {
    const url = new URL(request.url());
    if (url.origin !== origin) fail(`request to another origin: ${request.url()}`);
    else if (!PAGE_FILES.includes(url.pathname)) fail(`request for a file that is not the page's: ${url.pathname}`);
    else requested.add(url.pathname);
  });
  page.on("requestfailed", (request) => fail(`request failed: ${request.url()} (${request.failure()?.errorText})`));
  page.on("response", (response) => {
    if (response.status() >= 400) fail(`HTTP ${response.status()} for ${response.url()}`);
  });
  // A Content-Security-Policy violation, the module refused included.
  await page.addInitScript(() => {
    document.addEventListener("securitypolicyviolation", (event) => {
      console.error(`CSP violation: ${event.violatedDirective} ${event.blockedURI}`);
    });
  });

  await page.goto(`${origin}${PAGE_PATH}`);
  await page.waitForFunction(() => ["ready", "error"].includes(document.body.dataset.state), null, {
    timeout: TIMEOUT_MS,
  });
  const loaded = await page.evaluate(() => ({
    state: document.body.dataset.state,
    status: document.getElementById("status").textContent,
  }));
  if (loaded.state !== "ready") throw new Error(`the page did not load the module: ${loaded.status}`);

  for (const name of readdirSync(CASES).sort()) {
    const dir = join(CASES, name);
    const c = JSON.parse(readFileSync(join(dir, "case.json"), "utf8"));
    if (await run(page, `${name} (folder)`, dir, c, () => page.setInputFiles("#folder", dir))) runs += 1;
  }

  const dir004 = join(CASES, "004-multi-segment");
  const c004 = JSON.parse(readFileSync(join(dir004, "case.json"), "utf8"));
  const shuffled = SHUFFLED_004.map((f) => join(dir004, f));
  if (await run(page, "004-multi-segment (files, shuffled)", dir004, c004, () => page.setInputFiles("#files", shuffled))) {
    runs += 1;
  }

  // A folder dropped on #drop. Playwright has no call for dropping files from
  // disk, so the drag goes through the DevTools protocol with the folder's path,
  // which gives the page a real directory entry, as a person's drop does.
  const dirDrop = join(CASES, DROPPED_CASE);
  const cDrop = JSON.parse(readFileSync(join(dirDrop, "case.json"), "utf8"));
  const devtools = await page.context().newCDPSession(page);
  const dropFolder = async () => {
    await page.locator("#drop").scrollIntoViewIfNeeded();
    const box = await page.locator("#drop").boundingBox();
    const at = { x: box.x + box.width / 2, y: box.y + box.height / 2 };
    const data = { items: [], files: [dirDrop], dragOperationsMask: 1 };
    for (const type of ["dragEnter", "dragOver", "drop"]) {
      await devtools.send("Input.dispatchDragEvent", { type, ...at, data });
    }
  };
  if (await run(page, `${DROPPED_CASE} (dropped folder)`, dirDrop, cDrop, dropFolder)) runs += 1;

  for (const path of PAGE_FILES) if (!requested.has(path)) fail(`the page never requested ${path}`);
} catch (error) {
  fail(error.message.split("\n")[0]);
} finally {
  await browser.close();
  server.close();
}

if (failures.length) {
  for (const failure of failures) console.error(`page smoke: FAIL ${failure}`);
  console.error(`page smoke: ${failures.length} failures, ${runs} runs ok`);
  process.exit(1);
}
console.log(`page smoke: ${runs} runs ok`);

// --- One run ------------------------------------------------------------------------

// Sets #max-findings (without an event, so it does not verify the previous
// files again), chooses the files, waits for the run to finish, and compares
// what the page shows with the case's expected report. True if all matches.
async function run(page, label, dir, c, choose) {
  const max = c.max_findings ?? DEFAULT_MAX_FINDINGS;
  const before = await page.evaluate((value) => {
    document.getElementById("max-findings").value = String(value);
    return Number(document.body.dataset.runs);
  }, max);
  await choose();
  try {
    await page.waitForFunction((n) => Number(document.body.dataset.runs) > n, before, { timeout: TIMEOUT_MS });
  } catch {
    const status = await page.evaluate(() => document.getElementById("status").textContent);
    fail(`${label}: no run finished; status: ${status}`);
    return false;
  }
  const shown = await page.evaluate(() => {
    const text = (id) => document.getElementById(id).textContent;
    const rows = (id) =>
      [...document.getElementById(id).tBodies[0].rows].map((tr) => [...tr.cells].map((td) => td.textContent));
    return {
      state: document.body.dataset.state,
      report: text("report"),
      ok: document.getElementById("verdict").classList.contains("ok"),
      log: text("r-log"),
      head: text("r-head"),
      covered: text("r-covered"),
      checked: text("r-checked"),
      counts: text("r-counts"),
      truncated: text("r-truncated"),
      findings: rows("findings"),
      used: rows("used").map((cells) => cells[0]),
    };
  });

  const e = c.expect;
  const want = {
    state: "done",
    report: canonical(e),
    ok: e.ok,
    log: e.log ?? "none",
    head: e.head ? `seq ${e.head.seq}, entry hash ${e.head.entry_hash}` : "none",
    covered: e.covered ? `size ${e.covered.size}, checkpoint ${e.covered.checkpoint}` : "none",
    checked: `entries ${e.checked.entries}, objects ${e.checked.objects}, attestations ${e.checked.attestations}, checkpoints ${e.checked.checkpoints}`,
    counts: `forks ${e.counts.forks}, at_after_received ${e.counts.at_after_received}`,
    truncated: e.truncated ? "yes" : "no",
    findings: e.findings.map((f) => [f.code, f.seq ?? "", f.ref ?? "", f.core ?? "", f.file ?? ""].map(String)),
    used: ["manifest.json", ...segmentOrder(dir, e)],
  };
  let ok = true;
  for (const key of Object.keys(want)) {
    if (JSON.stringify(shown[key]) !== JSON.stringify(want[key])) {
      fail(`${label}: ${key} shows ${JSON.stringify(shown[key])}, expected ${JSON.stringify(want[key])}`);
      ok = false;
    }
  }
  return ok;
}

// The segment files in the order the page must use them: the manifest's list,
// or, for an invalid manifest, the six-digit files in name order.
function segmentOrder(dir, expect) {
  if (expect.findings.some((f) => f.code === "S_MANIFEST_INVALID")) {
    return readdirSync(dir)
      .filter((f) => SEGMENT_NAME.test(f))
      .sort();
  }
  return JSON.parse(readFileSync(join(dir, "manifest.json"), "utf8")).segments.map((s) => s.file);
}
