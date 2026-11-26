# verify-page

A static page that verifies a store/0 export (`manifest.json` and its segment files) in the browser. It runs `verifyExport` from the evidence WebAssembly module, the same verifier CI tests against every export case, so it prints the same report as `evidence store verify`, the Python package and the JavaScript reference. The page itself decides no finding. The files are read with the File API and sent nowhere.

| File | What |
|---|---|
| `index.html` | The page, with its Content-Security-Policy and how to serve it |
| `page.mjs` | Loads the module, reads the chosen files, orders the segments, shows the report |
| `page.css` | Layout for phone and desktop widths, light and dark colour schemes |
| `smoke.mjs` | The automated smoke test (Playwright and Chromium) |

There is no build step and no dependency. The full description is section 9 of [`docs/m1-store-spec.md`](../../docs/m1-store-spec.md).

## Serving and using it

Browsers load neither ES modules nor WebAssembly from a `file://` URL, so the page must be served over HTTP. From the repository root:

```sh
rustup target add wasm32-unknown-unknown   # once
sh tools/build-wasm.sh                      # writes crates/evidence-wasm/pkg/
python3 -m http.server 8000 --bind 127.0.0.1
```

Then open `http://127.0.0.1:8000/tools/verify-page/` and choose an export in one of three ways:

- **Export folder**: the folder input. Only the files directly in the folder are used; subfolders are ignored.
- **The export's files**: `manifest.json` and the segment files, in any order.
- **Drop**: one folder dropped on the box gives the files directly in it. Anything else dropped gives the dropped files; folders among them are ignored.

To try it, choose any folder under `test-vectors/store/exports/`. The 0xx cases are OK; each 1xx case has the findings its `case.json` expects. Case 129 expects `max_findings` 2: set **Max findings** to 2 to see its report.

To host the page elsewhere, copy `tools/verify-page/` and `crates/evidence-wasm/pkg/` keeping their relative layout: the page loads `../../crates/evidence-wasm/pkg/evidence.mjs`, which fetches `evidence_wasm.wasm` beside itself.

## What it does

1. Files are known by their names. Two chosen files with one name, or no `manifest.json`, is an error, and nothing is verified.
2. The segment order is the host rule of [`spec/store-0.md`](../../spec/store-0.md) ("Export only"). The page first calls `verifyExport(manifest, [])`. If that gives `S_MANIFEST_INVALID`, the segments are the chosen files named six digits and `.jsonl`, in name order. Otherwise they are the files the manifest lists, in its order; a listed file that was not chosen is passed as empty bytes, so it gets `S_SEGMENT_HASH`. Other files, such as `case.json`, are ignored.
3. Bytes go to the verifier exactly as read (`File.arrayBuffer()`); nothing is decoded first.
4. **Max findings** (default 1000, from 1 to 2^53 − 1) is passed as `maxFindings`. Changing it verifies the chosen files again.
5. The result shows OK or not OK, the log id, the head (seq and entry hash), the covered checkpoint (size and id), the `checked` and `counts` members, whether the findings were truncated, a findings table (code, seq, ref, core, file), the files used in order with their sizes, and the canonical report in `#report`. The table shows at most 1,000 findings; the report always holds all of them.

The verifier runs on the page's own thread, so the page does not respond while it works. On a 4-core machine an export of 20,000 entries (19 MB) takes about 2 seconds in Chromium. About a third of that is encoding the bytes for the module.

## Security

- The Content-Security-Policy, in a meta tag, is exactly `default-src 'self'; script-src 'self' 'wasm-unsafe-eval'; object-src 'none'; base-uri 'none'; form-action 'none'`. `'wasm-unsafe-eval'` is needed to compile the module; without it Chromium refuses `WebAssembly.compile`. There is no `'unsafe-eval'` and no `'unsafe-inline'`, and the page has no inline script or style.
- The page requests only five files from its own origin: `index.html`, `page.css`, `page.mjs`, `evidence.mjs` and `evidence_wasm.wasm`. No fonts, analytics or other origins.
- Everything from the export or the report, file names included, reaches the page through `textContent` or text nodes. `page.mjs` uses no `innerHTML`, `outerHTML`, `insertAdjacentHTML`, `document.write`, `eval` or `Function`, and makes no URL from the export's content.

## Smoke test

```sh
sh tools/build-wasm.sh
node tools/verify-page/smoke.mjs
# page smoke: 44 runs ok
```

It needs Playwright and its Chromium. It loads Playwright with `require`, so a global install on `NODE_PATH` or a `node_modules` folder above `tools/verify-page/` works. To install them outside the repository, as CI does:

```sh
PW="${TMPDIR:-/tmp}/pw"
npm install --no-save --no-package-lock --prefix "$PW" playwright@1.63.0
"$PW/node_modules/.bin/playwright" install chromium
NODE_PATH="$PW/node_modules" node tools/verify-page/smoke.mjs
```

The script serves the repository root itself (`node:http` on 127.0.0.1, a free port, GET only) and opens the page in headless Chromium. It runs:

- each of the 42 export cases through the folder input, with **Max findings** from `case.json` or 1000;
- case 004 through the files input, its manifest and four segments in a shuffled order;
- case 102 dropped on the drop box as a folder, through the DevTools protocol (`Input.dispatchDragEvent` with the folder's path), which gives the page a real directory entry.

For each run it compares the text of `#report` with the canonical form of the case's `expect`, computed in Node through the same module, and checks the verdict, log, head, covered, checked, counts, truncated, every row of the findings table, and the order of the files used. It fails on any console error or page error (a Content-Security-Policy violation included), on any request that is not one of the page's five files on its own origin, on a dialog, if `page.mjs` contains one of the sinks above, and if `index.html` lacks the exact policy or has inline script, style or event handlers. It exits 0 and prints `page smoke: 44 runs ok`, or lists every failure and exits 1. A missing Playwright, browser or module is a failure, never a skip.

## Manual check

Not automated: real drags from a file manager, and browsers other than Chromium. Before a release, in Chromium and in Firefox:

1. Serve the page as above and open it. The status says `Ready`.
2. Drag `test-vectors/store/exports/004-multi-segment` from the file manager onto the drop box. The result is OK, head seq 13, covered size 11, and the files used are `manifest.json` then `000001.jsonl` to `000004.jsonl`.
3. Open that folder, select all its files and drag them onto the box. The result is the same, and `case.json` and `inputs.json` are listed as ignored.
4. Drag `test-vectors/store/exports/116-segment-hash-mismatch` onto the box. The result is not OK, with `S_SEGMENT_HASH` for `000001.jsonl` and `S_OBJECT_INVALID` at seq 4.
5. Drag a folder that holds no `manifest.json`. The status says there is none, and no result is shown.
6. Open `tools/verify-page/index.html` as a file. The status stays on the loading message, which points to "Serving this page".
7. Narrow the window to phone width, and switch the system to the dark colour scheme. Nothing needs horizontal scrolling, and the text stays readable.
