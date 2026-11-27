// The offline verify page for store/0 exports (docs/m1-store-spec.md section 9).
//
// It reads the chosen files with the File API, orders the segment files by the
// host rule of spec/store-0.md ("Export only"), and calls verifyExport from the
// evidence WebAssembly module. Nothing here decides a finding, and nothing is
// sent anywhere. Text from the export or the report reaches the page only
// through textContent and text nodes; no markup is ever parsed from it.

const MODULE = "../../crates/evidence-wasm/pkg/evidence.mjs";
const MANIFEST = "manifest.json";
const SEGMENT_NAME = /^[0-9]{6}\.jsonl$/;
// Findings shown in the table; the report below it always holds all of them.
const TABLE_ROWS = 1000;
// Above this many segment bytes the status warns that verifying takes a while:
// the module runs on this page's thread, so the page waits for it.
const SLOW_BYTES = 5_000_000;
// The most bytes, manifest and segments together, the page passes to the
// module. They go as base64 inside one JSON string, which may hold at most
// 2^29 - 24 characters (about 536.9 million) in Chromium; 400 MB is 533.3
// million characters as base64. Larger exports are left to the CLI.
const MAX_BYTES = 400_000_000;
const READY = "Ready. Choose an export folder or its files, or drop them in the box.";

const byId = (id) => document.getElementById(id);
const body = document.body;
const ui = {
  status: byId("status"),
  folder: byId("folder"),
  files: byId("files"),
  drop: byId("drop"),
  maxFindings: byId("max-findings"),
  result: byId("result"),
  verdict: byId("verdict"),
  source: byId("r-source"),
  log: byId("r-log"),
  head: byId("r-head"),
  covered: byId("r-covered"),
  checked: byId("r-checked"),
  counts: byId("r-counts"),
  truncated: byId("r-truncated"),
  findingsNote: byId("findings-note"),
  findings: byId("findings").tBodies[0],
  usedNote: byId("used-note"),
  used: byId("used").tBodies[0],
  ignored: byId("ignored"),
  report: byId("report"),
};
const inputs = [ui.folder, ui.files, ui.maxFindings];

let ev = null; // the loaded module
// The files of the last choice, kept so a new max findings verifies them again:
// { manifest, manifestSize, segments: [{ name, bytes, use }], listed, ignored, skipped }
let chosen = null;
let generation = 0; // the latest choice or re-run; older ones stop at their next await
let runs = 0;

// --- State and status ----------------------------------------------------------

function setState(state, message) {
  body.dataset.state = state;
  ui.status.textContent = message;
}

function fail(message) {
  chosen = null;
  ui.result.hidden = true;
  setState("error", message);
}

function enable(on) {
  for (const input of inputs) input.disabled = !on;
  ui.drop.setAttribute("aria-disabled", on ? "false" : "true");
}

// Lets the browser paint the status before a long synchronous call. A frame
// callback does not run in a hidden tab, so a timer also ends the wait.
function nextFrame() {
  return new Promise((resolve) => {
    requestAnimationFrame(() => setTimeout(resolve, 0));
    setTimeout(resolve, 100);
  });
}

// --- Loading -------------------------------------------------------------------

async function start() {
  enable(false);
  setState("loading", "Loading the verifier.");
  try {
    const { load } = await import(MODULE);
    ev = await load();
  } catch (error) {
    setState(
      "error",
      "The verifier did not load. Build it with sh tools/build-wasm.sh, serve the repository root over HTTP " +
        `and open /tools/verify-page/ (see Serving this page). Reason: ${reason(error)}`,
    );
    return;
  }
  enable(true);
  setState("ready", READY);
}

function reason(error) {
  return error instanceof Error ? error.message : String(error);
}

// --- Choosing the files ----------------------------------------------------------

ui.folder.addEventListener("change", () => {
  const all = [...ui.folder.files];
  // webkitRelativePath is "<folder>/<name>" for a file directly in the folder.
  const direct = all.filter((f) => !f.webkitRelativePath || f.webkitRelativePath.split("/").length === 2);
  ui.files.value = "";
  choose(direct, all.length - direct.length);
});

ui.files.addEventListener("change", () => {
  ui.folder.value = "";
  choose([...ui.files.files], 0);
});

ui.maxFindings.addEventListener("change", () => {
  if (!ev) return;
  if (chosen) {
    verify(++generation);
    return;
  }
  // Nothing chosen yet: only say whether the value will do.
  if (maxFindings() === null) setState(body.dataset.state, maxFindingsMessage());
  else if (body.dataset.state === "ready") setState("ready", READY);
});

// A drop anywhere else on the page must not open the dropped file instead.
for (const type of ["dragover", "drop"]) {
  document.addEventListener(type, (event) => {
    if (ui.drop.contains(event.target)) return;
    event.preventDefault();
    if (event.dataTransfer) event.dataTransfer.dropEffect = "none";
  });
}

ui.drop.addEventListener("dragover", (event) => {
  event.preventDefault();
  const ready = ev !== null;
  event.dataTransfer.dropEffect = ready ? "copy" : "none";
  ui.drop.classList.toggle("over", ready);
});

ui.drop.addEventListener("dragleave", () => ui.drop.classList.remove("over"));

ui.drop.addEventListener("drop", (event) => {
  event.preventDefault();
  ui.drop.classList.remove("over");
  if (!ev) return;
  ui.folder.value = "";
  ui.files.value = "";
  // The dropped items can be read only during this event, so take the entries
  // and files now and read their contents afterwards.
  const transfer = event.dataTransfer;
  const items = transfer.items ? [...transfer.items].filter((item) => item.kind === "file") : [];
  if (!items.length) {
    choose([...(transfer.files ?? [])], 0);
    return;
  }
  const entries = items.map((item) => (typeof item.webkitGetAsEntry === "function" ? item.webkitGetAsEntry() : null));
  if (entries.length === 1 && entries[0]?.isDirectory) {
    chooseFolderEntry(entries[0]);
    return;
  }
  const files = [];
  let skipped = 0;
  items.forEach((item, i) => {
    if (entries[i]?.isDirectory) skipped += 1;
    else {
      const file = item.getAsFile();
      if (file) files.push(file);
    }
  });
  choose(files, skipped);
});

// The files directly in a dropped folder. readEntries returns them in batches
// and an empty batch at the end.
async function chooseFolderEntry(folder) {
  const gen = ++generation;
  ui.result.hidden = true;
  chosen = null;
  setState("verifying", "Reading the folder.");
  try {
    const reader = folder.createReader();
    const entries = [];
    for (;;) {
      const batch = await new Promise((resolve, reject) => reader.readEntries(resolve, reject));
      if (!batch.length) break;
      entries.push(...batch);
    }
    const files = [];
    let skipped = 0;
    for (const entry of entries) {
      if (entry.isFile) files.push(await new Promise((resolve, reject) => entry.file(resolve, reject)));
      else skipped += 1;
    }
    if (gen !== generation) return;
    choose(files, skipped);
  } catch (error) {
    if (gen === generation) fail(`Nothing was verified: the dropped folder could not be read: ${reason(error)}`);
  }
}

// Reads the chosen files, orders the segments, and verifies. `skipped` counts
// what was left out before this point (files in subfolders, dropped folders).
async function choose(files, skipped) {
  const gen = ++generation;
  ui.result.hidden = true;
  chosen = null;
  setState("verifying", `Reading ${count(files.length, "file")}.`);

  const byName = new Map();
  const twice = new Set();
  for (const file of files) {
    if (byName.has(file.name)) twice.add(file.name);
    else byName.set(file.name, file);
  }
  if (twice.size) {
    fail(`Nothing was verified: more than one chosen file is named ${[...twice].map(quote).join(", ")}.`);
    return;
  }
  if (!byName.has(MANIFEST)) {
    fail(
      files.length
        ? `Nothing was verified: there is no ${MANIFEST} among the ${count(files.length, "chosen file")}. ` +
            "Choose an export folder, or its manifest.json and segment files."
        : skipped
          ? `Nothing was verified: the ${count(skipped, "item")} given ` +
            `${skipped === 1 ? "is a folder or is" : "are folders or are"} inside a subfolder. ` +
            "The page uses only the files directly in one export folder, or the export's own files."
          : "Nothing was verified: no files were chosen.",
    );
    return;
  }
  // Sizes are known before anything is read.
  if (byName.get(MANIFEST).size > MAX_BYTES) {
    fail(tooLarge(`${MANIFEST} alone holds`, byName.get(MANIFEST).size));
    return;
  }

  let manifest;
  try {
    manifest = await bytesOf(byName.get(MANIFEST));
  } catch (error) {
    if (gen === generation) fail(`Nothing was verified: ${MANIFEST} could not be read: ${reason(error)}`);
    return;
  }
  if (gen !== generation) return;

  // The host rule: the files the manifest lists, in its order, or, when the
  // manifest is invalid (a finding that depends on its bytes alone), the
  // six-digit segment files in name order.
  let names;
  let listed;
  try {
    listed = !ev.verifyExport(manifest, []).findings.some((f) => f.code === "S_MANIFEST_INVALID");
    names = listed
      ? JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(manifest)).segments.map((s) => s.file)
      : [...byName.keys()].filter((name) => SEGMENT_NAME.test(name)).sort();
  } catch (error) {
    fail(`Nothing was verified: ${reason(error)}`);
    return;
  }

  const total = names.reduce((sum, name) => sum + (byName.get(name)?.size ?? 0), manifest.length);
  if (total > MAX_BYTES) {
    fail(tooLarge(`${MANIFEST} and the segment files hold`, total));
    return;
  }

  const segments = [];
  for (const name of names) {
    const file = byName.get(name);
    if (!file) {
      segments.push({ name, bytes: new Uint8Array(0), use: "missing: not among the chosen files, verified as empty" });
      continue;
    }
    try {
      segments.push({ name, bytes: await bytesOf(file), use: `segment ${segments.length + 1}` });
    } catch {
      segments.push({ name, bytes: new Uint8Array(0), use: "unreadable: verified as empty" });
    }
    if (gen !== generation) return;
  }
  const used = new Set([MANIFEST, ...names]);
  const ignored = [...byName.keys()].filter((name) => !used.has(name)).sort();
  chosen = { manifest, manifestSize: manifest.length, segments, listed, ignored, skipped };
  await verify(gen);
}

async function bytesOf(file) {
  return new Uint8Array(await file.arrayBuffer());
}

function tooLarge(what, bytes) {
  return (
    `Nothing was verified: ${what} ${megabytes(bytes)}, ` +
    `more than the ${MAX_BYTES / 1_000_000} MB this page can verify. Verify the folder with evidence store verify instead.`
  );
}

// --- Verifying -------------------------------------------------------------------

function maxFindings() {
  const text = ui.maxFindings.value.trim();
  if (!/^[0-9]{1,16}$/.test(text)) return null;
  const n = Number(text);
  return n >= 1 && n <= Number.MAX_SAFE_INTEGER ? n : null;
}

function maxFindingsMessage() {
  return `Max findings must be a whole number from 1 to ${Number.MAX_SAFE_INTEGER}.`;
}

async function verify(gen) {
  const max = maxFindings();
  if (max === null) {
    ui.result.hidden = true;
    setState("error", `Nothing was verified: ${maxFindingsMessage()}`);
    return;
  }
  const current = chosen;
  const bytes = current.segments.reduce((sum, s) => sum + s.bytes.length, 0);
  // Checked again on the bytes read, in case a file grew after its size was taken.
  if (current.manifestSize + bytes > MAX_BYTES) {
    fail(tooLarge(`${MANIFEST} and the segment files hold`, current.manifestSize + bytes));
    return;
  }
  setState(
    "verifying",
    `Verifying ${count(current.segments.length, "segment file")}, ${megabytes(bytes)}.` +
      (bytes > SLOW_BYTES ? " The page does not respond until this is done." : ""),
  );
  await nextFrame();
  if (gen !== generation || current !== chosen) return;

  let report;
  let canonical;
  const started = performance.now();
  try {
    report = ev.verifyExport(current.manifest, current.segments.map((s) => s.bytes), { maxFindings: max });
    canonical = ev.canonicalize(JSON.stringify(report));
  } catch (error) {
    fail(`Nothing was verified: ${reason(error)}`);
    return;
  }
  const ms = Math.round(performance.now() - started);

  render(report, canonical, current);
  runs += 1;
  body.dataset.runs = String(runs);
  const outcome = report.ok ? "OK" : `not OK, ${count(report.findings.length, "finding")}`;
  setState("done", `Verified ${count(report.checked.entries, "entry", "entries")} in ${ms} ms: ${outcome}.`);
}

// --- Showing the result ----------------------------------------------------------

function render(report, canonical, current) {
  ui.verdict.className = report.ok ? "verdict ok" : "verdict bad";
  ui.verdict.textContent = report.ok
    ? "OK: no findings."
    : `Not OK: ${count(report.findings.length, "finding")}` +
      (report.truncated ? ", and more that max findings left out." : ".");

  show(ui.source, report.source);
  show(ui.log, report.log ? code(report.log) : "none");
  show(ui.head, ...(report.head ? [`seq ${report.head.seq}, entry hash `, code(report.head.entry_hash)] : ["none"]));
  show(
    ui.covered,
    ...(report.covered ? [`size ${report.covered.size}, checkpoint `, code(report.covered.checkpoint)] : ["none"]),
  );
  const c = report.checked;
  show(ui.checked, `entries ${c.entries}, objects ${c.objects}, attestations ${c.attestations}, checkpoints ${c.checkpoints}`);
  show(ui.counts, `forks ${report.counts.forks}, at_after_received ${report.counts.at_after_received}`);
  show(ui.truncated, report.truncated ? "yes" : "no");

  const shown = report.findings.slice(0, TABLE_ROWS);
  ui.findings.replaceChildren(
    ...shown.map((f) =>
      row([
        ["Code", f.code],
        ["Seq", f.seq],
        ["Ref", f.ref, "hash"],
        ["Core", f.core],
        ["File", f.file],
      ]),
    ),
  );
  byId("findings").hidden = shown.length === 0;
  ui.findingsNote.textContent =
    shown.length === 0
      ? "None."
      : shown.length < report.findings.length
        ? `The first ${shown.length} of ${report.findings.length}; the report below holds them all.`
        : "";
  ui.findingsNote.hidden = !ui.findingsNote.textContent;

  ui.used.replaceChildren(
    row([
      ["File", MANIFEST],
      ["Bytes", current.manifestSize],
      ["Use", "manifest"],
    ]),
    ...current.segments.map((s) =>
      row([
        ["File", s.name],
        ["Bytes", s.bytes.length],
        ["Use", s.use],
      ]),
    ),
  );
  ui.usedNote.textContent = current.listed
    ? "The segment files the manifest lists, in its order."
    : "The manifest is invalid, so the files named with six digits and .jsonl are used, in name order.";
  const left = [];
  if (current.ignored.length) left.push(`Ignored: ${current.ignored.join(", ")}.`);
  if (current.skipped) {
    left.push(`Also ignored: ${count(current.skipped, "item")} that ${current.skipped === 1 ? "is a folder or is" : "are folders or are"} inside a subfolder.`);
  }
  ui.ignored.textContent = left.join(" ");
  ui.ignored.hidden = !left.length;

  ui.report.textContent = canonical;
  ui.result.hidden = false;
}

// Replaces a node's content with text and <code> elements.
function show(node, ...parts) {
  node.replaceChildren(...parts.map((p) => (typeof p === "string" ? document.createTextNode(p) : p)));
}

function code(text) {
  const element = document.createElement("code");
  element.textContent = text;
  return element;
}

// A table row; `cells` are [label, value, class]. The label is shown beside
// the value when the table is stacked on a narrow screen.
function row(cells) {
  const tr = document.createElement("tr");
  tr.setAttribute("role", "row");
  for (const [label, value, className] of cells) {
    const td = document.createElement("td");
    td.setAttribute("role", "cell");
    td.dataset.label = label;
    if (className) td.className = className;
    if (value !== undefined && value !== null) td.textContent = String(value);
    tr.append(td);
  }
  return tr;
}

function count(n, one, many = `${one}s`) {
  return `${n} ${n === 1 ? one : many}`;
}

function megabytes(n) {
  return n < 100_000 ? count(n, "byte") : `${(n / 1_000_000).toFixed(1)} MB`;
}

function quote(name) {
  return `"${name}"`;
}

start();
