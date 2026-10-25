// Real-browser audit of Z8 Studio. Drives the page with a mouse and keyboard
// the way a person does, in a THROWAWAY dated session filled with copies of
// real Z8 frames, and puts everything back afterwards.
// Playwright from, in order: Z8_PLAYWRIGHT, the normal module path, then the
// npm global folder under ~/.local. Hardcoding the last one alone
// meant the audit (and so the self-test) broke if that repo ever moved.
function loadPlaywright() {
  const tries = [process.env.Z8_PLAYWRIGHT, "playwright",
                 require("path").join(require("os").homedir(),
                                      ".local/lib/node_modules/playwright")];
  for (const t of tries.filter(Boolean)) {
    try { return require(t); } catch (_) { /* next */ }
  }
  console.log("FAIL  Playwright could be loaded   set Z8_PLAYWRIGHT to its folder");
  process.exit(2);
}
const { chromium } = loadPlaywright();
const fs = require("fs"), path = require("path"), { execFileSync } = require("child_process");

// Z8_BASE and Z8_ROOT point the audit at another server and photo root, so the
// Rust server can be driven by the same checks against a sandbox.
const B = process.env.Z8_BASE || "http://127.0.0.1:8420";
const OUT = process.env.Z8_AUDIT_OUT || require("os").tmpdir();
const SESSION = "1999-02-02";
const ROOT = process.env.Z8_ROOT || path.join(process.env.HOME, "Pictures/z8-studio");
// The frames to copy: Z8_AUDIT_SRC, else the 2026-09-21 headshot session, else
// (a machine without it, like a fresh laptop) the newest session holding at
// least five camera JPEGs (with three, a later step waits for a frame that is not there).
const SRC = process.env.Z8_AUDIT_SRC || (() => {
  const want = path.join(ROOT, "2026-09-21");
  if (fs.existsSync(want)) return want;
  const jpgs = d => { try { return fs.readdirSync(path.join(ROOT, d)).filter(f => /^\d{8}-\d{6}\.jpg$/.test(f)); } catch (_) { return []; } };
  const days = fs.existsSync(ROOT) ? fs.readdirSync(ROOT).filter(d => /^\d{4}-\d{2}-\d{2}$/.test(d) && jpgs(d).length >= 5).sort() : [];
  return days.length ? path.join(ROOT, days[days.length - 1]) : want;
})();
const results = [];
const bench = {};
let fails = 0;
const ok = (name, cond, detail = "") => {
  results.push(`${cond ? "PASS" : "FAIL"}  ${name}${detail ? "   " + detail : ""}`);
  if (!cond) fails++;
};
const api = async (p, body) => {
  const r = await fetch(B + p, body ? { method: "POST", headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body) } : {});
  return r.json();
};
const sleep = ms => new Promise(r => setTimeout(r, ms));
async function until(fn, ms = 8000, step = 50) {
  const t0 = Date.now();
  while (Date.now() - t0 < ms) { try { const v = await fn(); if (v) return v; } catch (_) {} await sleep(step); }
  return null;
}

(async () => {
  const was = (await api("/api/state")).session;
  const cfgWas = JSON.stringify((await api("/api/state")).config);
  await api("/api/session", { date: SESSION });
  const sd = (await api("/api/state")).dir;
  fs.mkdirSync(sd, { recursive: true });
  const real = fs.readdirSync(SRC).filter(f => f.endsWith(".jpg") && !f.endsWith(".dev.jpg")).sort().slice(-6);
  real.forEach(f => fs.copyFileSync(path.join(SRC, f), path.join(sd, f)));
  execFileSync("ffmpeg", ["-v", "error", "-f", "lavfi", "-i", "testsrc2=size=1280x720:rate=30:duration=6",
    "-f", "lavfi", "-i", "sine=frequency=440:duration=6", "-c:v", "libx264", "-g", "30", "-pix_fmt", "yuv420p",
    "-c:a", "aac", "-shortest", "-y", path.join(sd, "19990202-120000.mp4")]);
  // Where exports REALLY go: the session's own folder, unless a run (the
  // self-test) has redirected them to a sandbox.
  const expDir = (await api("/api/state")).exportDir;

  const browser = await chromium.launch();
  const ctx = await browser.newContext({ viewport: { width: 1600, height: 1000 } });
  const page = await ctx.newPage();
  const consoleErrors = [], pageErrors = [], badResponses = [], nativeDialogs = [];
  page.on("console", m => { if (m.type() === "error") consoleErrors.push(m.text()); });
  page.on("pageerror", e => pageErrors.push(String(e)));
  page.on("response", r => { if (r.status() >= 400) badResponses.push(`${r.status()} ${r.url().slice(0, 110)}`); });
  page.on("dialog", d => { nativeDialogs.push(d.type() + ": " + d.message().slice(0, 60)); d.dismiss(); });

  try {
    // ---------------------------------------------------------------- boot
    let t0 = Date.now();
    // domcontentloaded: with the camera on, the live MJPEG never finishes, so
    // "load" never fires.
    await page.goto(B + "/?tab=camera", { waitUntil: "domcontentloaded" });
    await page.waitForSelector("#strip .cell img");
    const CAM_ON = !!(await api("/api/state")).connected;
    if (CAM_ON) {
      const lv = await until(() => page.evaluate(() => {
        const l = document.querySelector("#live"); return !l.hidden && l.naturalWidth > 0; }), 15000);
      ok("camera on: the stage opens on live view", !!lv);
      // Reviewing a frame is what puts it on the stage while tethered.
      await page.locator("#strip .cell img").first().click();
    }
    bench["page to first thumbnail (ms)"] = Date.now() - t0;
    const n = await page.locator("#strip .cell").count();
    ok("the strip shows every frame in the session", n === real.length, `${n} of ${real.length}`);
    const st0 = await until(() => page.evaluate(() => {
      const s = document.querySelector("#still");
      return !s.hidden && s.naturalWidth > 0 && s.dataset.name ? s.dataset.name : null; }));
    bench["page to graded stage picture (ms)"] = Date.now() - t0;
    ok("the stage shows the first frame, rendered", !!st0, st0 || "nothing on the stage");
    ok("the idle message is out of the way", await page.locator("#idle").isHidden());
    ok("no broken image anywhere", await page.evaluate(() =>
      [...document.images].filter(i => !i.closest("[hidden]") && !i.hidden && i.getAttribute("src")
        && i.complete && i.naturalWidth === 0).map(i => i.id || i.src).length === 0));
    ok("the stage picture fits inside the stage", await page.evaluate(() => {
      const v = document.querySelector(".view").getBoundingClientRect();
      const s = document.querySelector("#still").getBoundingClientRect();
      return s.top >= v.top - 1 && s.bottom <= v.bottom + 1 && s.left >= v.left - 1 && s.right <= v.right + 1;
    }));

    // ---------------------------------------------------------- selection
    const cells = page.locator("#strip .cell img");
    // The strip's own order, not the directory's: it runs newest first.
    const target = await cells.nth(3).getAttribute("title");
    t0 = Date.now();
    await cells.nth(3).click();
    const ph = await until(() => page.evaluate(n => document.querySelector("#still").dataset.placeholder === n
      || document.querySelector("#still").dataset.name === n, target), 2000, 5);
    bench["thumbnail click to SOMETHING of that frame on stage (ms)"] = Date.now() - t0;
    ok("a click answers on the stage at once", !!ph && Date.now() - t0 < 400, `${Date.now() - t0} ms`);
    const sel3 = await until(() => page.evaluate(n => {
      const s = document.querySelector("#still"); return s.dataset.name === n && s.complete; }, target));
    bench["thumbnail click to new stage picture (ms)"] = Date.now() - t0;
    ok("clicking a thumbnail puts that frame on the stage", !!sel3);
    ok("within 2 s on a 45 MP frame", Date.now() - t0 < 2000 + 50, `${Date.now() - t0} ms`);
    ok("and highlights exactly one thumbnail", await page.locator('#strip img[data-on="1"]').count() === 1);

    // ------------------------------------------------ live grade on stage
    await page.click('.tabs [data-tab="edit"]');
    ok("the Edit tab opens", await page.locator('.pane[data-pane="edit"]').isVisible());
    await until(() => page.evaluate(() => document.querySelector("#cropImg").naturalWidth > 0));
    const meanOf = sel => page.evaluate(sel => {
      const im = document.querySelector(sel); const c = document.createElement("canvas");
      c.width = 64; c.height = 64; const g = c.getContext("2d"); g.drawImage(im, 0, 0, 64, 64);
      const d = g.getImageData(0, 0, 64, 64).data; let t = 0;
      for (let i = 0; i < d.length; i += 4) t += (d[i] + d[i + 1] + d[i + 2]) / 3;
      return t / (d.length / 4); }, sel);
    const before = await meanOf("#still");
    const srcBefore = await page.getAttribute("#still", "src");
    // drag the exposure thumb to the right with a real mouse
    const box = await page.locator("#exposure").boundingBox();
    await page.mouse.move(box.x + box.width * 0.385, box.y + box.height / 2);
    await page.mouse.down();
    t0 = Date.now();
    await page.mouse.move(box.x + box.width * 0.85, box.y + box.height / 2, { steps: 12 });
    const nudged = await page.evaluate(() => document.querySelector("#still").style.filter);
    ok("dragging a slider changes the big picture IMMEDIATELY", /brightness\(1\.[1-9]/.test(nudged), nudged || "no filter");
    await page.mouse.up();
    const rendered = await until(() => page.evaluate(b => {
      const s = document.querySelector("#still");
      return s.getAttribute("src") !== b && s.complete && s.style.filter === "" ? s.getAttribute("src") : null; }, srcBefore));
    bench["slider release to real render on stage (ms)"] = Date.now() - t0;
    ok("then swaps in the real render", !!rendered && rendered.includes("exposure"));
    const after = await meanOf("#still");
    ok("and the big picture is measurably brighter", after > before + 8, `${before.toFixed(1)} -> ${after.toFixed(1)}`);
    ok("the readout follows", parseFloat(await page.textContent("#oExposure")) > 1.3, await page.textContent("#oExposure"));
    ok("the moved knob is flagged", await page.evaluate(() => !!document.querySelector("#exposure").closest(".sld").dataset.moved));
    await until(() => page.evaluate(() => document.querySelector("#clipInfo").textContent.length > 0));
    ok("the histogram re-measures", (await page.textContent("#clipInfo")).length > 0, await page.textContent("#clipInfo"));

    // double click resets one knob, and the stage follows
    await page.dblclick("#exposure");
    await until(() => page.evaluate(() => document.querySelector("#oExposure").textContent === "1.00"));
    ok("double click resets a slider", (await page.textContent("#oExposure")) === "1.00");
    const back = await until(async () => { const m = await meanOf("#still"); return Math.abs(m - before) < 3 ? m : null; }, 6000);
    ok("and the big picture goes back", !!back, back ? back.toFixed(1) : "still bright");

    // hold to compare: the stage shows the original
    await page.fill("#exposure", "1.5"); await page.dispatchEvent("#exposure", "input");
    await sleep(900);
    const graded = await meanOf("#still");
    await page.hover("#btnCompare"); await page.mouse.down();
    await until(() => page.evaluate(() => document.querySelector("#still").getAttribute("src").includes("original=1")
      && document.querySelector("#still").complete));
    ok("holding Compare shows the original on the stage", (await meanOf("#still")) < graded - 8);
    ok("with a Before badge", await page.locator("#stageBadge").isVisible());
    await page.mouse.up();
    await until(() => page.evaluate(() => !document.querySelector("#still").getAttribute("src").includes("original=1")
      && document.querySelector("#still").complete));
    ok("releasing it brings the grade back", Math.abs((await meanOf("#still")) - graded) < 3);
    ok("and the badge goes", await page.locator("#stageBadge").isHidden());

    // undo
    await page.click(".view", { position: { x: 5, y: 5 } }).catch(() => {});
    await page.keyboard.press("Control+z");
    await until(() => page.evaluate(() => document.querySelector("#oExposure").textContent !== "1.50"));
    ok("ctrl+z undoes the last grade", (await page.textContent("#oExposure")) !== "1.50", await page.textContent("#oExposure"));

    // look preset moves everything, and the stage
    const s1 = await page.getAttribute("#still", "src");
    await page.selectOption("#lookPreset", { index: 1 });
    const s2 = await until(() => page.evaluate(a => { const s = document.querySelector("#still");
      return s.getAttribute("src") !== a && s.complete ? s.getAttribute("src") : null; }, s1));
    ok("choosing a look re-renders the big picture", !!s2);

    // ---------------------------------------------------------------- crop
    const aspect = () => page.evaluate(() => { const s = document.querySelector("#still"); return s.naturalWidth / s.naturalHeight; });
    // The shape comes from the export preset, which is on the Export tab.
    await page.click('.tabs [data-tab="export"]');
    await page.selectOption("#exportPreset", "ig-square");
    await page.click('.tabs [data-tab="edit"]');
    ok("the crop pane names the preset it follows", /Instagram|square|1080/i.test(await page.textContent("#cropShape")), await page.textContent("#cropShape"));
    await until(async () => Math.abs((await aspect()) - 1) < 0.03, 6000);
    ok("a square preset crops the big picture square", Math.abs((await aspect()) - 1) < 0.03, (await aspect()).toFixed(3));
    ok("the crop box is on screen", await page.locator("#cropBox").isVisible());
    const cb = await page.locator("#cropBox").boundingBox();
    await page.mouse.move(cb.x + cb.width / 2, cb.y + cb.height / 2);
    await page.mouse.down();
    await page.mouse.move(cb.x + cb.width / 2 + 30, cb.y + cb.height / 2, { steps: 5 });
    await page.mouse.up();
    const cb2 = await page.locator("#cropBox").boundingBox();
    ok("dragging moves the crop box", Math.abs(cb2.x - cb.x) > 5, `${cb.x.toFixed(0)} -> ${cb2.x.toFixed(0)}`);
    const moved = await until(() => page.evaluate(() => document.querySelector("#still").getAttribute("src").includes("crop")));
    ok("and the big picture follows the crop", !!moved);
    await page.click("#cropReset");
    await until(async () => Math.abs((await aspect()) - 1.5) < 0.05, 6000);
    ok("Full frame puts the whole picture back on stage", Math.abs((await aspect()) - 1.5) < 0.05, (await aspect()).toFixed(3));

    // loupe
    const cw = await page.locator("#cropImg").boundingBox();
    await page.mouse.dblclick(cw.x + cw.width / 2, cw.y + cw.height / 3);
    await until(() => page.evaluate(() => document.querySelector("#loupeImg").naturalWidth > 100));
    ok("double click opens the 1:1 loupe", await page.locator("#loupe").isVisible());
    await page.keyboard.press("Escape");
    ok("esc closes it", await page.locator("#loupe").isHidden());

    // --------------------------------------------------------------- culling
    await page.selectOption("#filterPick", "all");
    await cells.nth(0).click();
    const first = await cells.nth(0).getAttribute("title");
    await until(() => page.evaluate(n => document.querySelector("#still").dataset.name === n, first));
    await page.keyboard.press("p");
    const advanced = await until(() => page.evaluate(n => document.querySelector("#still").dataset.name !== n, first));
    ok("p picks and moves the stage on to the next frame", !!advanced);
    const picked = await until(() => page.evaluate(() => document.querySelector('#strip .cell[data-p="pick"]') ? 1 : 0));
    ok("the pick shows on its thumbnail", !!picked);
    await page.keyboard.press("x");
    const rej = await until(() => page.evaluate(() => document.querySelector('#strip .cell[data-p="reject"]') ? 1 : 0));
    ok("x rejects", !!rej);
    const beforeArrow = await page.evaluate(() => document.querySelector("#still").dataset.name);
    await page.keyboard.press("ArrowRight");
    const afterArrow = await until(() => page.evaluate(b => { const n = document.querySelector("#still").dataset.name; return n !== b ? n : null; }, beforeArrow));
    ok("arrow keys walk the frames on the stage", !!afterArrow);
    await page.selectOption("#filterPick", "pick");
    await until(async () => (await page.locator("#strip .cell").count()) === 1);
    ok("the Picks filter shows only picks", (await page.locator("#strip .cell").count()) === 1);
    await page.selectOption("#filterPick", "all");
    await until(async () => (await page.locator("#strip .cell").count()) === real.length);
    // put the flags back
    for (let i = 0; i < real.length; i++) await api("/api/pick", { name: real[i], flag: null });

    // ------------------------------------------------- live without reload
    await sleep(4000);                       // let the flag reset above land
    const firstThumb = await page.locator("#strip .cell img").first().elementHandle();
    await sleep(7000);                       // two polls with nothing changing
    ok("the strip is not rebuilt by the poll when nothing changed",
       await page.evaluate(el => el.isConnected, firstThumb));
    fs.copyFileSync(path.join(SRC, real[0]), path.join(sd, "19990202-235959.jpg"));
    t0 = Date.now();
    const appeared = await until(async () => (await page.locator("#strip .cell").count()) === real.length + 1, 9000);
    bench["new file on disk to thumbnail, no reload (ms)"] = Date.now() - t0;
    ok("a new frame appears without reloading", !!appeared);
    ok("the session dropdown names the day on screen", (await page.inputValue("#sessionPick")) === SESSION);
    const optText = await page.evaluate(d => [...document.querySelectorAll("#sessionPick option")]
      .find(o => o.value === d).textContent, SESSION);
    ok("and its frame count followed the new frame", optText.includes(`${real.length + 1} frames`), optText.trim());

    // --------------------------------------------------------------- export
    await page.click('.tabs [data-tab="export"]');
    await page.selectOption("#exportPreset", "yt-thumb");
    // Full frame (pressed in the crop test) deliberately turns this on; a
    // thumbnail is meant to fill 1280x720.
    await page.uncheck("#wholeFrame");
    const expBefore = fs.existsSync(expDir) ? fs.readdirSync(expDir).length : 0;
    t0 = Date.now();
    await page.click("#btnExport");
    const link = await until(() => page.evaluate(() => { const a = document.querySelector("#exportOut a"); return a && a.textContent; }), 30000);
    bench["single export, 45 MP frame (ms)"] = Date.now() - t0;
    ok("Export writes a file and links it", !!link, link || await page.textContent("#exportOut"));
    ok("the message shows in the pinned footer", await page.locator("#exportOut").isVisible());
    ok("and as a toast over the stage", await page.locator("#toast.is-on").count() === 1);
    const onDisk = link && fs.existsSync(path.join(expDir, link));
    ok("the export is on disk", !!onDisk, link || "");
    if (onDisk) {
      const dims = execFileSync("python3", ["-c", `from PIL import Image;print(Image.open(${JSON.stringify(path.join(expDir, link))}).size)`]).toString().trim();
      ok("at the preset's size", dims === "(1280, 720)", dims);
    }
    ok("exports went to the configured folder, not a stale one", (fs.readdirSync(expDir).length) > expBefore);

    // multi-select export with a working cancel
    await page.locator("#strip .cell img").nth(0).click();
    await page.locator("#strip .cell img").nth(4).click({ modifiers: ["Shift"] });
    ok("shift-click marks a run", (await page.locator("#strip .cell[data-mark]").count()) === 5);
    await page.click("#btnExport");
    await until(() => page.evaluate(() => /cancel/.test(document.querySelector("#btnExport").textContent)));
    ok("a marked batch shows a live counter", /cancel/.test(await page.textContent("#btnExport")), await page.textContent("#btnExport"));
    ok("and its cancel is clickable", await page.locator("#btnExport").isEnabled());
    await page.click("#btnExport");
    const idle = await until(() => page.evaluate(() => /^Export \d+ selected|^Export selected/.test(document.querySelector("#btnExport").textContent)), 30000);
    ok("cancel stops it", !!idle, await page.textContent("#btnExport"));
    await page.click("#btnClearMarks");

    // ------------------------------------------------ the other workflows
    await page.click('.tabs [data-tab="edit"]');
    await cells.nth(1).click();
    const nm1 = await cells.nth(1).getAttribute("title");
    await until(() => page.evaluate(n => document.querySelector("#still").dataset.name === n, nm1));
    const preAuto = await page.getAttribute("#still", "src");
    await page.click("#btnAuto");
    await until(() => page.evaluate(() => !document.querySelector("#autoWhy").hidden), 20000);
    ok("Auto explains what it corrected", (await page.textContent("#autoWhy")).length > 5, (await page.textContent("#autoWhy")).slice(0, 70));
    const postAuto = await until(() => page.evaluate(a => { const s = document.querySelector("#still");
      return s.getAttribute("src") !== a && s.complete && !s.style.filter; }, preAuto), 8000);
    ok("and the big picture shows the correction", !!postAuto);
    await page.click("#btnLight");
    await until(() => page.evaluate(() => !document.querySelector("#lightOut").hidden), 20000);
    ok("Light measures the frame and reports", (await page.locator("#lightOut .lt tr").count()) >= 3);

    // copy / paste with keys
    await page.keyboard.press("c");
    ok("c copies the look", await page.locator("#btnPasteEdits").isEnabled());
    await cells.nth(2).click();
    const nm2 = await cells.nth(2).getAttribute("title");
    await until(() => page.evaluate(n => document.querySelector("#still").dataset.name === n, nm2));
    const copied = await page.evaluate(() => document.querySelector("#btnPasteEdits").disabled ? null : 1);
    await page.keyboard.press("v");
    await until(() => page.evaluate(() => /pasted the look/.test(document.querySelector("#exportOut").textContent)));
    ok("v pastes it onto another frame", /pasted the look onto/.test(await page.textContent("#exportOut")) && !!copied,
       await page.textContent("#exportOut"));

    // Auto all: a live counter, and the selected frame's sliders reload
    await page.click("#btnAutoAll");
    await until(() => page.evaluate(() => /\d+ \/ \d+/.test(document.querySelector("#btnAutoAll").textContent)), 5000, 20);
    const doneAll = await until(() => page.evaluate(() => /graded \d+ frame|could not/.test(document.querySelector("#exportOut").textContent)
      && document.querySelector("#btnAutoAll").textContent === "Auto all" && !document.querySelector("#btnAutoAll").disabled), 90000);
    ok("Auto all runs to completion and resets its button", !!doneAll);
    ok("and reports per-frame grading", /graded \d+ frame/.test(await page.textContent("#exportOut")), await page.textContent("#exportOut"));

    // Rank: jumps to Library, rows open frames on the stage
    await page.click("#btnRank");
    await until(() => page.evaluate(() => !document.querySelector("#rankSec").hidden), 90000);
    ok("Rank opens the Library tab with its results", await page.locator('.pane[data-pane="library"]').isVisible()
       && (await page.locator("#rankOut tr[data-n]").count()) >= real.length);
    const rowName = await page.locator("#rankOut tr[data-n]").nth(2).getAttribute("data-n");
    await page.locator("#rankOut tr[data-n]").nth(2).click();
    ok("clicking a ranked row puts that frame on the stage",
       !!await until(() => page.evaluate(n => document.querySelector("#still").dataset.name === n, rowName)));

    // Contact sheet: a real file
    await page.click('.tabs [data-tab="export"]');
    await page.click("#btnSheet");
    const sheet = await until(() => page.evaluate(() => /frames on \d+ page/.test(document.querySelector("#exportOut").textContent)
      && document.querySelector("#exportOut").textContent), 60000);
    ok("Contact sheet builds", !!sheet, (sheet || "").split("\n")[0]);
    const sheetFile = sheet && sheet.split("\n")[1] && sheet.split("\n")[1].split(/\s+/)[0];
    ok("and the sheet is on disk", !!sheetFile && fs.readdirSync(expDir).includes(sheetFile), sheetFile || "");

    // Delete and undo through the buttons
    const victim = await cells.nth(0).getAttribute("title");
    await cells.nth(0).click();
    await page.click("#btnDelete");
    await until(async () => (await page.locator(`#strip img[title="${victim}"]`).count()) === 0);
    ok("Delete takes the frame off the strip", (await page.locator(`#strip img[title="${victim}"]`).count()) === 0);
    ok("and off the stage", await page.evaluate(v => document.querySelector("#still").dataset.name !== v, victim));
    ok("Undo delete appears", await page.locator("#btnUndelete").isVisible());
    await page.click("#btnUndelete");
    await until(async () => (await page.locator(`#strip img[title="${victim}"]`).count()) === 1);
    ok("and brings it back", (await page.locator(`#strip img[title="${victim}"]`).count()) === 1);
    ok("the file is really back on disk", fs.existsSync(path.join(sd, victim)));

    // Tidy exports: an orphan export, the in-app dialog, the move
    fs.writeFileSync(path.join(expDir, "19990202-000000_yt-thumb.jpg"), fs.readFileSync(path.join(sd, real[0])));
    await page.click('.tabs [data-tab="library"]');
    await page.click("#btnTidy");
    const dlgUp = await until(() => page.locator("#dlg[open]").isVisible(), 8000);
    ok("Tidy asks in the app's own dialog", !!dlgUp, dlgUp ? await page.textContent("#dlgTitle") : "no dialog");
    if (dlgUp) {
      ok("naming the file it will move", (await page.textContent("#dlgBody")).includes("19990202-000000_yt-thumb.jpg"));
      await page.click("#dlgOk");
      await until(() => page.evaluate(() => /moved to the trash/.test(document.querySelector("#exportOut").textContent)));
      ok("confirming moves it out of exports", !fs.existsSync(path.join(expDir, "19990202-000000_yt-thumb.jpg")));
    }

    // ------------------------------------------- an ungraded export is said
    await page.click('.tabs [data-tab="export"]');
    await page.selectOption("#exportPreset", "yt-thumb");
    // Auto all (above) graded everything; take one frame back to "as shot".
    const fresh = "19990202-235959.jpg";
    await api("/api/edits", { name: fresh, adjust: null, crop: null });
    await until(() => page.evaluate(n => !(window.__z8edits || {})[n], fresh), 500);
    await page.waitForTimeout(3500);                     // one state poll
    await page.locator(`#strip img[title="${fresh}"]`).click();
    await until(() => page.evaluate(n => document.querySelector("#still").dataset.name === n, fresh));
    ok("an ungraded frame is flagged above Export", await page.locator("#gradeNote").isVisible());
    const bt = await page.textContent("#batchGradeTxt");
    ok("and the batch says how many have no grade", /\d+ of \d+ frame/.test(bt), bt);
    await page.click("#btnGradeAuto");
    const gradedNow = await until(() => page.evaluate(() => !document.querySelector("#autoWhy").hidden
      && document.querySelector("#gradeNote").hidden), 20000);
    ok("Auto-grade it grades the frame and the flag goes", !!gradedNow);
    const nBefore = parseInt(bt, 10);
    const dropped = await until(async () => {
      const t = await page.textContent("#batchGradeTxt");
      return !(await page.locator("#batchGradeNote").isVisible()) || parseInt(t, 10) < nBefore ? t || "none left" : null; }, 8000);
    ok("the batch count drops once the grade is saved", !!dropped, dropped || bt);
    await page.selectOption("#exportPreset", "original").catch(() => {});
    const isCopy = await page.evaluate(() => { const o = [...document.querySelectorAll("#exportPreset option")]
      .find(o => o.value === document.querySelector("#exportPreset").value); return o && /original/i.test(o.textContent); });
    if (isCopy) ok("the copy-the-camera-file preset never nags", await page.locator("#batchGradeNote").isHidden());

    // ----------------------------------------------------------- dialogs
    await page.click('.tabs [data-tab="camera"]');
    await page.click("#btnCamSave");
    ok("Save opens the app's own dialog", await page.locator("#dlg[open]").isVisible());
    ok("with the name field focused", await page.evaluate(() => document.activeElement.id === "dlgInput"));
    await page.keyboard.press("Space");
    ok("typing a space in it does NOT fire the shutter", !(await page.evaluate(() => document.body.dataset.shooting === "1")));
    await page.keyboard.press("Escape");
    ok("esc cancels it", await page.locator("#dlg[open]").count() === 0);
    await page.click("#btnCamSave");
    await page.click("#dlgCancel");
    ok("Cancel closes it", await page.locator("#dlg[open]").count() === 0);

    // ------------------------------------------------------ keyboard help
    await page.click("#btnKeys");
    ok("the ? button opens the shortcut list", await page.locator("#keys").isVisible());
    ok("which lists real bindings", (await page.locator("#keysTbl tr").count()) >= 10);
    await page.keyboard.press("Escape");
    ok("esc closes it", await page.locator("#keys").isHidden());

    // -------------------------------------------------------- clip player
    await page.click('.tabs [data-tab="library"]');
    await until(() => page.locator("#clipsSec").isVisible());
    ok("the clip is listed in Library", await page.locator("#clipStrip img").count() === 1);
    await page.click("#clipStrip img");
    await until(() => page.evaluate(() => document.querySelector("#playerVid").readyState >= 2));
    ok("clicking it opens the player", await page.locator("#player").isVisible());
    await page.evaluate(() => document.querySelector("#playerVid").pause());
    await page.keyboard.press("Space");
    await sleep(300);
    ok("space plays the clip", await page.evaluate(() => !document.querySelector("#playerVid").paused));
    ok("and does NOT fire the shutter behind it", await page.evaluate(() => document.body.dataset.shooting !== "1"));
    await page.keyboard.press("Space");
    await sleep(200);
    ok("space again pauses", await page.evaluate(() => document.querySelector("#playerVid").paused));
    await page.keyboard.press("Escape");
    ok("esc closes the player", await page.locator("#player").isHidden());

    // ----------------------------------------------------------- tabs
    await page.click('.tabs [data-tab="export"]');
    await page.goto(B + "/", { waitUntil: "domcontentloaded" });  // no ?tab=, memory decides
    await page.waitForSelector("#strip .cell img");
    ok("the chosen tab survives a reload", await page.locator('.pane[data-pane="export"]').isVisible());
    await page.focus('.tabs [data-tab="export"]');
    await page.keyboard.press("ArrowRight");
    ok("arrow keys move between tabs", await page.locator('.pane[data-pane="library"]').isVisible());

    // ------------------------------------------------ layout at real sizes
    for (const [w, h] of [[1920, 1080], [1366, 768], [1280, 720], [900, 600]]) {
      await page.setViewportSize({ width: w, height: h });
      for (const tab of ["camera", "edit", "export", "library"]) {
        await page.click(`.tabs [data-tab="${tab}"]`);
        await sleep(150);
        const r = await page.evaluate(() => {
          const vw = document.documentElement.clientWidth;
          const bad = [];
          document.querySelectorAll("button, select, input").forEach(el => {
            if (el.closest("[hidden]") || el.closest("dialog:not([open])") || el.closest("#player,#keys,#loupe")) return;
            const b = el.getBoundingClientRect();
            if (b.width < 4 || b.height < 4) bad.push(el.id || el.tagName);
            else if (b.right > vw + 1 || b.left < -1) bad.push((el.id || el.tagName) + ":off");
          });
          // text that overflows its own button
          const clipped = [...document.querySelectorAll("button")].filter(b =>
            !b.closest("[hidden]") && b.offsetParent && b.scrollWidth > b.clientWidth + 2).map(b => b.id || b.textContent.trim());
          return { overflow: document.documentElement.scrollWidth - vw, bad, clipped };
        });
        ok(`${w}x${h} ${tab}: no horizontal overflow, every control reachable, no clipped labels`,
           r.overflow <= 0 && !r.bad.length && !r.clipped.length,
           `overflow ${r.overflow} ${r.bad.join(",")} ${r.clipped.join(",")}`);
      }
      await page.click('.tabs [data-tab="edit"]');
      await page.screenshot({ path: path.join(OUT, `pw-${w}x${h}.png`) });
    }

    // ---------------------------------------------------------- benchmark
    await page.setViewportSize({ width: 1600, height: 1000 });
    const lat = [];
    for (let i = 0; i < 12; i++) {
      const a = JSON.stringify({ exposure: 1 + i * 0.03 });
      const t = Date.now();
      await fetch(`${B}/api/preview?name=${encodeURIComponent(real[1])}&adjust=${encodeURIComponent(a)}&s=${i}`).then(r => r.arrayBuffer());
      lat.push(Date.now() - t);
    }
    lat.sort((a, b) => a - b);
    bench["/api/preview cached-base render p50 / max (ms)"] = `${lat[6]} / ${lat[11]}`;
    const t1 = Date.now();
    await fetch(`${B}/api/state`).then(r => r.json());
    bench["/api/state (ms)"] = Date.now() - t1;
    // a fast scrub: 40 inputs in 1s must not queue 40 renders on screen
    const reqs = [];
    const onReq = r => { if (r.url().includes("/api/preview")) reqs.push(r.url()); };
    page.on("request", onReq);
    await page.click('.tabs [data-tab="edit"]');
    for (let i = 0; i < 40; i++) {
      await page.evaluate(v => { const e = document.querySelector("#contrast"); e.value = v; e.dispatchEvent(new Event("input")); }, String(1 + i * 0.01));
      await sleep(25);
    }
    await sleep(1500);
    page.off("request", onReq);
    bench["preview requests for a 40-step, 1 s scrub"] = reqs.length;
    ok("a fast scrub is debounced, not 40 renders", reqs.length <= 8, `${reqs.length} requests`);
    ok("and lands on the final value", (await page.getAttribute("#still", "src")).includes("1.39"));
    const heap = await page.evaluate(() => performance.memory ? Math.round(performance.memory.usedJSHeapSize / 1048576) : -1);

    // ------------------------------------------ browsing a past day (LAST:
    // "Go to today" leaves the page on today's session, read only)
    await page.setViewportSize({ width: 1600, height: 1000 });
    ok("browsing an older day warns where the shutter is", await page.locator("#pastWarn").isVisible());
    ok("and names the day being shown", (await page.textContent("#pastWarn")).includes(SESSION),
       (await page.textContent("#pastWarn")).trim().slice(0, 60));
    await page.click("#pastWarn button");
    const today = await until(async () => { const st = await api("/api/state");
      return !st.browsingPast ? st.session : null; });
    ok("one click goes back to today", !!today, today || "");
    ok("and the warning goes with it", !!await until(() => page.locator("#pastWarn").isHidden()));
    ok("the dropdown follows", !!await until(async () => (await page.inputValue("#sessionPick")) === today));
    bench["JS heap after the whole audit (MB)"] = heap;
    const stats = await page.evaluate(() => { try { return null; } catch (_) { return null; } });
  } catch (e) {
    ok("audit ran to the end", false, String(e).slice(0, 300));
  } finally {
    ok("no JavaScript errors", !pageErrors.length, pageErrors.slice(0, 3).join(" | "));
    ok("no console errors", !consoleErrors.length, consoleErrors.slice(0, 3).join(" | "));
    ok("no native browser popups", !nativeDialogs.length, nativeDialogs.join(" | "));
    ok("no failed requests", !badResponses.length, badResponses.slice(0, 4).join(" | "));
    await browser.close();
    await api("/api/session", { date: was });
    // A background prefetch or export started by the audit can still be
    // finishing and recreate the folder's exports/ after the first removal.
    for (let i = 0; i < 20 && fs.existsSync(sd); i++) {
      fs.rmSync(sd, { recursive: true, force: true });
      await sleep(250);
    }
    const after = await api("/api/state");
    ok("the real session is back", after.session === was, after.session);
    ok("the config is untouched", JSON.stringify(after.config) === cfgWas);
    ok("the throwaway session is gone", !fs.existsSync(sd));
    console.log(results.join("\n"));
    console.log("\nBENCHMARK");
    for (const [k, v] of Object.entries(bench)) console.log(`  ${k.padEnd(52)} ${v}`);
    console.log(`\n${results.length - fails} passed, ${fails} failed`);
    process.exit(fails ? 1 : 0);
  }
})();
