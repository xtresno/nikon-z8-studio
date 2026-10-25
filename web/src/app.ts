(function(){
"use strict";

/* ---- what the server sends ------------------------------------------------
   One declaration per shape, so a field read that the server does not send is
   a compile error here instead of an `undefined` on the page. */
type Knob = "exposure" | "shadows" | "highlights" | "contrast" | "saturation"
          | "warmth" | "tint" | "vignette" | "sharpness" | "straighten";
type Adjust = Record<Knob, number>;
interface Crop { x: number; y: number; w: number; h: number }
interface Edit { adjust: Partial<Adjust>; crop: Crop | null }
type Pick = "pick" | "reject" | null;
interface Shot { name: string; size: number; at: number; pick: Pick; edited: boolean;
                 provisional?: boolean }
interface Clip { name: string; size: number; at: number; w: number | null; h: number | null;
                 duration: number | null }
interface Take { name: string; size: number; at: number; seconds: number | null }
interface ExportPreset { id: string; label: string; note?: string; w?: number; h?: number;
                         fit?: string; anchor?: string; format?: string; quality?: number;
                         dpi?: number; max_kb?: number }
interface LookPreset extends Partial<Adjust> { id: string; label: string; note?: string }
interface Presets { export: ExportPreset[]; look: LookPreset[];
                    _anchors: Record<string, [number, number]> }
interface NextStep { step: string; why: string; stage: number; stageName: string;
                     total: number; ladder: string[] }
interface FeedStatus { state: string; since?: number; frames?: number; sent?: number;
                       lastFrameAge?: number | null; engine?: string;
                       holdMs?: {p50: number; p90: number; max: number; n: number} | null }
interface AudioRec { name: string; seconds: number; mb: number; limit: number }
interface ServerState {
  present: boolean; manualOff: boolean; connected: boolean; recording: boolean;
  warning: string | null; audioRec: AudioRec | null; error: string | null;
  rotation: number; mirror: boolean; liveTurn: number; camPresets: string[];
  transientConfig: string[]; orphanExports: number; converting: number;
  webcam: boolean; feed: FeedStatus | null; dir: string; session: string;
  exportDir: string; freeMB: number; config: Config; today: string;
  browsingPast: boolean; sessions: {date: string; frames: number}[]; shots: Shot[];
  clips: Clip[]; takes: Take[]; edits: Record<string, Edit>; sourceId: string;
  startedAt: number; nextStep: NextStep | null;
  toolsMissing: {bin: string; why: string; install: string}[];
  trashMB: number; trash: number;
}
/** config.json as the server resolves it (stored plus any transient override). */
interface Config { export_dir: string; name_template: string; autofocus_before_shoot: boolean;
                   autofocus_settle_ms: number; webcam_look_id: string; capture_dir: string;
                   camera_default_preset: string }
/* ---- every route's reply ------------------------------------------------
   A failure is {ok: false, error}. After `if (!r.ok) return`, the compiler
   knows the success fields are there, so no reply field is ever read on the
   strength of a guess. Shapes follow src/http.rs. */
interface Fail { ok: false; error: string; busy?: boolean; disk_full?: boolean }
type Result<T> = ({ok: true} & T) | Fail;
/** One item a job finished or failed on. */
interface JobItem { name: string; error?: string; path?: string; size?: number;
  dims?: [number, number]; seconds?: number; changes?: number; faceY?: number;
  offset?: number; how?: string; copied?: boolean; ignored_edits?: string[] }
interface JobState { ok: boolean; id: string; kind: string; total: number; done: number;
  state: string; results: JobItem[]; errors: JobItem[]; started: number; elapsed?: number;
  restarted?: boolean; error?: string; ranked?: Ranked[] }
type Started = Result<{job: string; total: number}>;
interface ExportOut { name: string; path: string; size: number; dims: [number, number];
  copied?: boolean; ignored_edits?: string[] }
interface Orphan { name: string; stem: string; state: string; size: number; at: number }
interface Note { level: string; what: string; why: string; fix: string }
interface LightMeasured { faceY: number | null; eyeY: number | null; eyeVsCheek: number | null;
  catchlightPct: number; catchlightPx: number; rangeUsed: number; p1: number; p99: number;
  backdropY: number; separation: number | null; litShadowRatio: number; skinSpread: number;
  skinOnlyBands: boolean }
interface AudioStatus { source: string | null; volume: number | null; muted: boolean | null;
  cards: {index: string; id: string; name: string}[];
  meter: {card: string; values: number[]; max: number} | null; signal: boolean | null;
  phantom: boolean | null; hints: string[]; level: number | null; why: string | null }
type CamPresets = Record<string, {values: Record<string, string>; at: number}>;
interface Routes {
  "/api/state": ServerState;
  "/api/presets": Presets;
  "/api/settings": SettingsReply;
  "/api/audio": AudioStatus;
  "/api/card": Result<{files: CardFile[]; truncated: boolean; warning: string | null}>;
  "/api/orient": {rotation: number; mirror: boolean; leased: boolean};
  "/api/af": {ok: boolean; msg: string};
  "/api/setting": {ok: boolean; msg: string};
  "/api/movie": {ok: boolean; msg: string};
  "/api/lever": {lever: "photo" | "video" | null};
  "/api/shoot": {ok: boolean; name: string | null; error: string | null; warning: string | null};
  "/api/mode": {ok: boolean; mode: string; webcam: boolean; connected: boolean;
                error?: string | null; filter?: string | null};
  "/api/exposure-fix": Result<{steps: string[]; before: Record<string, string | null>;
                               after: Record<string, string | null>}>;
  "/api/session": Result<{session: string}>;
  "/api/pick": Result<{picks: Record<string, string>}>;
  "/api/edits": Result<{edits: Edit | null}>;
  "/api/edits-apply-all": {ok: boolean; applied: number};
  "/api/delete": Result<{removed: string[]; recoverable: boolean}>;
  "/api/undelete": {ok: boolean; restored: string[]; error: string | null};
  "/api/empty-trash": {ok: boolean; removed: number};
  "/api/camera-preset": Result<{name?: string; values?: Record<string, string>;
    presets: CamPresets; applied?: {key: string; value: string; retried?: boolean}[];
    refused?: {key: string; value: string; error: string}[]}>
    | {ok: false; error?: string; name: string; presets: CamPresets;
       applied: {key: string; value: string}[]; refused: {key: string; value: string; error: string}[]};
  "/api/config": Result<{config: Config; transient: string[]; resolved: string; stored?: Config}>;
  "/api/call-look": Result<{live: boolean; id: string}>;
  "/api/look": Result<{id: string}>;
  "/api/lighting": Result<{measured: LightMeasured; notes: Note[]}>;
  "/api/auto-look": Result<{adjust: Adjust; why: string[];
    measured: {source: string; faceY: number; spread: number; skinFraction: number; p1: number; p99: number}}>;
  "/api/auto-look-all": Started;
  "/api/rank": Started;
  "/api/export": Result<ExportOut>;
  "/api/export-all": Started;
  "/api/export-many": Started;
  "/api/card-import": Started;
  "/api/trim": Result<{job: string; total: number; out: string}>;
  "/api/sync-audio": Result<{job: string; total: number; offset: number; how: string;
                             camera_clap: number; take_clap: number}>;
  "/api/grab": Result<{name: string; path: string; at: number; size: number; dims: [number, number]}>;
  "/api/contact-sheet": Result<{path: string; frames: number; name: string; dims: [number, number];
    pages: {name: string; path: string; frames: number; dims: [number, number]}[]}>;
  // The dry run always succeeds; the real move reports ok: false when any
  // file failed, WITH the lists, which a plain failure shape would lose.
  "/api/tidy-exports": {ok: true; dryRun: true; orphans: Orphan[]; bytes: number; kept: number; unknown: number}
                     | {ok: boolean; dryRun?: undefined; moved: string[];
                        failed: {name: string; error: string}[]; bytes: number; recoverable: boolean};
  "/api/open-folder": Result<{path: string}>;
  "/api/job-cancel": {ok: boolean};
  "/api/audio-record": {ok: boolean; recording?: boolean; name?: string; source?: string;
    note?: string; path?: string; size?: number; seconds?: number | null; error?: string | null};
}

/** One row of a Rank job's result. */
interface Ranked { name: string; ok: boolean; why?: string; focus?: number; focusPct?: number;
  faults?: string[]; penalty?: number; faceY?: number; clipped?: number; catchlight?: number }

/* ---- page state ---- */
interface State {
  rotation: number; mirror: boolean; shots: Shot[]; clips: Clip[]; takes: Take[];
  edits: Record<string, Edit>; editMap: Record<string, Edit>; sel: string | null;
  presets: Presets | null; grid: string; review: boolean; liveTurn: number;
  trash: number; trashMB: number; webcam: boolean; exportDir: string;
  browsingPast: boolean; session: string; toolsMissing: ServerState["toolsMissing"];
}

/**
 * The one element lookup in this file. An id from index.html comes back as
 * its real DOM type (src/elements.d.ts is generated from the page), so a
 * `.value` on a button or a typo'd id is a compile error, not an undefined.
 * Any other selector may match nothing, and says so.
 */
function $<K extends keyof Z8Elements>(sel: K): Z8Elements[K];
function $(sel: string): HTMLElement | null;
function $(sel: string): HTMLElement | null { return document.querySelector<HTMLElement>(sel); }

/** querySelectorAll as an array of HTMLElement. */
const $$ = (sel: string, root?: ParentNode): HTMLElement[] =>
  [...(root || document).querySelectorAll<HTMLElement>(sel)];

/** A lookup the page cannot work without: a clear error, never a null crash. */
function must<T>(el: T | null | undefined, what: string): T {
  if (el == null) throw new Error(`the page is missing ${what}`);
  return el;
}

/** A grade slider by its knob name. */
const knob = (k: Knob): HTMLInputElement => $(`#${k}` as `#${Knob}`);
const body = document.body;

/** Text for innerHTML. File names come off the camera or a card import, and
 *  server messages quote them, so none of it is markup. */
const ESC: Record<string, string> =
  {"&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;"};
const esc = (v: unknown): string => String(v == null ? "" : v).replace(/[&<>"']/g, c => ESC[c]);
const state: State = {rotation: 0, mirror: false, shots: [], clips: [], takes: [], edits: {},
  editMap: {}, sel: null, presets: null, grid: "off", review: false, liveTurn: 0, trash: 0,
  trashMB: 0, webcam: false, exportDir: "", browsingPast: false, session: "", toolsMissing: []};

const SLIDERS: Knob[] = ["exposure","shadows","highlights","contrast",
                 "saturation","warmth","tint","vignette","sharpness","straighten"];
// multiplicative knobs sit at 1, additive ones at 0
const NEUTRAL: Adjust = {exposure:1, contrast:1, saturation:1, sharpness:1,
                 shadows:0, highlights:0, warmth:0, tint:0, vignette:0,
                 straighten:0};

const EMPTY_PRESETS: Presets = {export: [], look: [], _anchors: {}};
/** The presets once loaded; empty lists before, never null. */
const presets = (): Presets => state.presets || EMPTY_PRESETS;

function adj(): Adjust {
  const o = {} as Adjust;
  SLIDERS.forEach(k => o[k] = parseFloat(knob(k).value));
  return o;
}

/* The browser preview only approximates the render. Brightness, contrast and
   saturation map cleanly onto CSS filters; warmth is faked with sepia so the
   slider at least moves in the right direction. Export is the truth. */
function cssFilter(){
  const a = adj();  // live view only: CSS cannot do tone curves
  const warm = a.warmth > 0 ? `sepia(${(a.warmth*0.5).toFixed(2)})`
             : a.warmth < 0 ? `hue-rotate(${(a.warmth*22).toFixed(0)}deg)` : "";
  return `brightness(${a.exposure}) contrast(${a.contrast}) saturate(${a.saturation}) ${warm}`;
}

function applyTransform(){
  const el = $("#live");
  // The camera's live view is SENSOR-oriented: with the body turned for a
  // portrait it arrived sideways. liveTurn comes from the newest frame's EXIF
  // Orientation (the only place the Z8 reports how it is held), on top of the
  // user's own Rotate. Frames need none of this: they are uprighted from the
  // same tag when they are read.
  const turn = ((state.rotation || 0) + (state.liveTurn || 0)) % 360;
  // A CSS rotation does not change the layout box, so a landscape box turned
  // upright overflowed the stage top and bottom. Scale the turned picture to
  // fit the stage the other way round.
  let fit = 1;
  if ((turn === 90 || turn === 270) && el.offsetWidth && el.offsetHeight) {
    const v = must($(".view"), ".view");
    fit = Math.min((v.clientHeight - 28) / el.offsetWidth,
                   (v.clientWidth - 28) / el.offsetHeight);
  }
  el.style.transform = `rotate(${turn}deg) scale(${fit.toFixed(4)})`
    + ` scaleX(${state.mirror?-1:1})`;
  // Show the angle actually applied. The body turn is learned from the last
  // photo and can be stale (the camera turned since); a visible "270" is one
  // click from fixed, a hidden one looked like a broken live view.
  const rt = $("#rotTxt");
  if (rt) rt.innerHTML = turn + "&deg;";
  el.style.filter = cssFilter();
}
$("#live").addEventListener("load", applyTransform);
// The stage changes size without a window resize: the next-step banner comes
// and goes, and the past-day warning adds a row. A fit computed on the first
// frame then overflowed until something else redrew it.
if (typeof ResizeObserver === "function")
  new ResizeObserver(() => applyTransform()).observe(must($(".view"), ".view"));
else window.addEventListener("resize", applyTransform);

async function api<P extends keyof Routes>(path: P, body?: object,
    signal?: AbortSignal, keepalive = false): Promise<Routes[P]> {
  const controller = new AbortController();
  const timer = (!body || keepalive) && !signal ? setTimeout(() => controller.abort(), 12000) : undefined;
  try {
    const r = await fetch(path, {signal: signal || controller.signal, keepalive,
      ...(body ? {method:"POST", headers:{"Content-Type":"application/json"},
        body:JSON.stringify(body)} : {})});
    return await r.json() as Routes[P];
  } finally { clearTimeout(timer); }
}

/** A background job's progress. */
async function job(id: string): Promise<JobState> {
  const r = await fetch("/api/job?id=" + encodeURIComponent(id));
  return r.json() as Promise<JobState>;
}

/* ---- dialogs ----
   The browser's own prompt() and confirm() are modal to the whole tab, look
   like an error from another program, and freeze every timer on the page,
   including the state poll. One in-page dialog instead, resolving a promise. */
interface DialogOpts { ok?: string; danger?: boolean; input?: boolean; value?: string }
function openDialog(title: string, text: string, opts: DialogOpts): Promise<string | true | null> {
  const d = $("#dlg"), inp = $("#dlgInput");
  $("#dlgTitle").textContent = title;
  $("#dlgBody").textContent = text || "";
  $("#dlgOk").textContent = opts.ok || "OK";
  $("#dlgOk").className = opts.danger ? "danger" : "pri";
  inp.hidden = !opts.input;
  inp.value = opts.value || "";
  return new Promise(resolve => {
    const done = (v: string | true | null) => {
      d.removeEventListener("close", onClose);
      $("#dlgCancel").onclick = null; $("#dlgOk").onclick = null;
      $("#dlgForm").onsubmit = null;
      if (d.open) d.close();
      resolve(v);
    };
    const onClose = () => done(null);
    const accept = () => {
      if (opts.input && !inp.value.trim()) { inp.focus(); return; }
      done(opts.input ? inp.value.trim() : true);
    };
    d.addEventListener("close", onClose);
    $("#dlgCancel").onclick = () => done(null);
    $("#dlgOk").onclick = accept;
    // Enter in the name field submits the form; route it through the same path.
    $("#dlgForm").onsubmit = (e: SubmitEvent) => { e.preventDefault(); accept(); };
    d.showModal();
    if (opts.input) { inp.focus(); inp.select(); } else $("#dlgOk").focus();
  });
}
async function askText(title: string, text: string, value?: string): Promise<string | null> {
  const r = await openDialog(title, text, {input: true, value, ok: "Save"});
  return typeof r === "string" ? r : null;
}
async function askConfirm(title: string, text: string, ok?: string): Promise<boolean> {
  return !!(await openDialog(title, text, {ok, danger: true}));
}

/* ---- sidebar tabs ----
   Ten panels stacked in one scrolling column put Export and the look sliders
   a long scroll below the camera controls. Four tabs, one job each. Inactive
   panes carry `hidden`, so the layout probe skips them and so does the tab
   order. Remembered per browser, which is only a convenience. */
const TABS = ["camera", "edit", "export", "library"];
function showTab(want: string | null){
  const name = want && TABS.includes(want) ? want : "camera";
  $$(".tabs [data-tab]").forEach(b =>
    b.setAttribute("aria-selected", String(b.dataset.tab === name)));
  $$(".pane[data-pane]").forEach(p => { p.hidden = p.dataset.pane !== name; });
  try { localStorage.setItem("z8.tab", name); } catch (_) { /* private mode */ }
  // The crop box is positioned from the image's on-screen box, which is
  // zero-sized while its pane is hidden.
  if (name === "export" && tabsReady) prefetchForExport();
  if (name === "edit") {
    requestAnimationFrame(() => drawCrop());
    // Opening Edit means grading the selected frame, so show it big. Not at
    // boot: a remembered tab must not hide the live view on page load.
    if (tabsReady) setReview(true);
  }
}
let tabsReady = false;
$$(".tabs [data-tab]").forEach(b =>
  b.addEventListener("click", () => showTab(b.dataset.tab || null)));
must($(".tabs"), ".tabs").addEventListener("keydown", (e: KeyboardEvent) => {
  if (e.key !== "ArrowLeft" && e.key !== "ArrowRight") return;
  e.preventDefault(); e.stopPropagation();
  // From the tab that has FOCUS, which is the one the arrow is pressed on.
  const act = document.activeElement as HTMLElement | null;
  const from = act && act.dataset && act.dataset.tab
    ? act : $(".tabs [aria-selected=true]");
  const cur = TABS.indexOf((from && from.dataset.tab) || "camera");
  const nx = TABS[(cur + (e.key === "ArrowRight" ? 1 : TABS.length - 1)) % TABS.length];
  showTab(nx);
  $(`.tabs [data-tab="${nx}"]`)?.focus();
});
// ?tab=edit opens a given panel, which is also how a headless screenshot
// reaches a panel other than the first.
try { showTab(new URLSearchParams(location.search).get("tab")
              || localStorage.getItem("z8.tab") || "camera"); }
catch (_) { showTab("camera"); }
tabsReady = true;

/* ---- mode: tether / webcam / off ----
   Tether and webcam cannot share the camera, so this does the handoff and the
   user never has to know the webcam pipeline exists. */
type ModeId = "mTether" | "mWebcam" | "mOff";
const MODES: Record<ModeId, string> = {mTether:"tether", mWebcam:"webcam", mOff:"off"};
const MODE_IDS = Object.keys(MODES) as ModeId[];
const modeBtn = (id: ModeId): HTMLButtonElement => $(`#${id}` as `#${ModeId}`);

// One action, chosen by the server from what is actually true. Rendered as a
// single line on purpose: a checklist of five is five things to forget, and
// the point of this panel is that he can act on it without reading it twice.
function paintNextStep(ns: NextStep | null){
  const box = $("#nextStep");
  if (!ns || !ns.step) { box.hidden = true; return; }
  box.hidden = false;
  $("#nsStage").textContent = `${ns.stageName} \u00b7 step ${ns.stage + 1} of ${ns.total}`;
  $("#nsStep").textContent = ns.step;
  $("#nsWhy").textContent = ns.why || "";
  const dots = $("#nsDots");
  if (dots.childElementCount !== ns.total) {
    dots.textContent = "";
    for (let i = 0; i < ns.total; i++) dots.appendChild(document.createElement("i"));
  }
  ([...dots.children] as HTMLElement[]).forEach((d, i) => {
    d.className = i < ns.stage ? "done" : (i === ns.stage ? "now" : "");
    d.title = (ns.ladder && ns.ladder[i]) || "";
  });
}

/* ---- review vs live ----
   With the camera connected the stage used to be live view, ALWAYS: clicking
   a frame, culling, or dragging a slider changed only the thumbnails and the
   small crop pane, so the big graded preview existed only with the camera
   off. Reviewing a frame now takes the stage, the way a tethered Lightroom
   session does, and Back to live (or l, Shoot, Focus) returns it. The MJPEG
   stream is closed while reviewing, so it stops competing for the camera. */
type PaintArgs = [string, boolean, boolean, ServerState?];
let lastPaint: PaintArgs | null = null;
function setReview(on: boolean){
  if (!!state.review === !!on) return;
  state.review = !!on;
  if (lastPaint) paintMode(...lastPaint);
}

function paintMode(m: string, connected: boolean, webcam: boolean, st?: ServerState){
  lastPaint = [m, connected, webcam, st];
  MODE_IDS.forEach(id =>
    modeBtn(id).setAttribute("aria-pressed", String(MODES[id] === m)));
  body.dataset.conn = connected ? "1" : "0";
  body.dataset.mode = m;
  // "Not connected" answers two different questions, and only one of them is
  // the user's problem. Say which: no camera on the bus means power it on,
  // a camera on the bus means the server is already opening it.
  const present = st ? st.present : false, off = st ? st.manualOff : false;
  $("#connTxt").textContent = connected ? "Live" : (webcam ? "Webcam"
                            : present ? (off ? "Released" : "Connecting…") : "No camera");
  const feed = st && st.feed;
  $("#modeTxt").textContent = webcam
    ? (feed && feed.state === "reconnecting" ? "Reconnecting the camera… OBS/Zoom stay connected"
       : feed && feed.state === "no-camera" ? "Camera off or unplugged, reconnects by itself"
       : "Feeding OBS / Zoom")
                            : connected ? "Tethered"
                            : present ? (off ? "Camera released, press Tether"
                                             : "Camera found, opening it…")
                            : "No camera on USB, power the Z8 on";
  const live = $("#live");
  $("#callLookNote").textContent = webcam
    ? "Call looks update Zoom/OBS live. Camera settings and in-camera recordings are unchanged."
    : "Call looks apply in Webcam mode for Zoom/OBS. The Tether preview and camera recordings are separate.";
  // A headless run must never open the live stream: /preview.mjpg never ends,
  // so the page never finishes loading and `chrome --dump-dom` HANGS. With the
  // camera off that was invisible; the moment real hardware appeared the layout
  // probe reported nothing and Chrome sat there until it was killed.
  const HEADLESS = new URLSearchParams(location.search).has("e2e")
                || new URLSearchParams(location.search).has("layout");
  const reviewing = connected && state.review && !!state.sel;
  $("#btnLive").hidden = !reviewing;
  if (connected && !HEADLESS && !reviewing && !document.hidden) {
    live.hidden = false;
    if (!live.src.includes("preview.mjpg")) live.src = "/preview.mjpg?t=" + Date.now();
    $("#idle").hidden = true;
    $("#still").hidden = true;          // the feed wins the stage back
    applyTransform();
  } else {
    live.hidden = true; live.src = "";
    // No live feed, but there may still be a frame to look at. Culling is the
    // task he does most and on the most frames, and the stage was showing a
    // camera message while the selected frame was only visible as a thumbnail
    // and in the crop pane below the fold.
    const still = $("#still");
    const showStill = !!state.sel && !HEADLESS;
    if (showStill) {
      // The graded, cropped render: what the export will look like. It used
      // to be the raw file, so every slider moved only the small crop pane
      // in the sidebar while the big picture stayed ungraded.
      if (still.dataset.name !== state.sel
          && still.dataset.placeholder !== state.sel) refreshStage(false);
    } else if (still.src) {
      still.removeAttribute("src");
      delete still.dataset.name;
    }
    still.hidden = !showStill;
    $("#idle").hidden = showStill;
    if (!showStill && !$("#idle").dataset.sticky) {
      $("#idle").innerHTML = present
        ? (off ? "Camera is plugged in and released.<br>Press <b>Tether</b> to take it back."
               : "Camera found on USB, opening it…<br><span style='color:var(--dim)'>"
                 + "This happens by itself, no need to press anything.</span>")
        : "Plug in the Z8 and switch it on.<br><span style='color:var(--dim)'>"
          + "It connects by itself. Set the photo/video switch to <b>photo</b> for stills.</span>";
    }
  }
  (["#btnShoot", "#btnAf", "#btnRec"] as const).forEach(b => $(b).disabled = !connected);
}

// True while a mode switch is in flight, so the state poll does not repaint
// the buttons back to the old mode halfway through the handover.
let modeBusy = false;

async function setMode(m: string){
  state.review = false;
  modeBusy = true;
  try { await setModeInner(m); }
  catch (e) { flash($("#exportOut"), "mode switch failed: " + (e instanceof Error ? e.message : String(e)), true); }
  finally {
    modeBusy = false;
    // A thrown request used to leave all three mode buttons disabled for good.
    MODE_IDS.forEach(id => modeBtn(id).disabled = false);
  }
  refresh();
}
async function setModeInner(m: string){
  MODE_IDS.forEach(id => modeBtn(id).disabled = true);
  $("#modeTxt").textContent = "Switching…";
  await callLookWrites;
  // Normally NO look is sent and the server resolves the configured call
  // look. This used to always send adj(), the edit sliders, so grading a dark
  // frame and then joining a call sent exposure 1.8 and a vignette out live.
  //
  // "follow the edit sliders" is the one setting that asks for the old
  // behaviour, and only the page knows where the sliders are, so it is the
  // page that has to send them.
  const followSliders = $("#callLook").value === "sliders";
  const r = await api("/api/mode", m === "webcam" && followSliders
    ? {mode: m, look: adj()} : {mode: m});
  MODE_IDS.forEach(id => modeBtn(id).disabled = false);
  paintMode(m, r.connected, r.webcam);
  if (m === "tether" && !r.connected) {
    $("#idle").dataset.sticky = "1";
    $("#idle").innerHTML = "Could not open the camera.<br><span style='color:var(--tally)'>"
      + esc(r.error || "") + "</span><br><br>Check it is powered on, awake, and set to "
      + "<b>photo</b> mode. A flat battery shows as <i>Unknown model</i> or a PTP timeout.";
  }
  if (r.connected) { delete $("#idle").dataset.sticky; loadSettings(); }
}
MODE_IDS.forEach(id =>
  modeBtn(id).addEventListener("click", () => setMode(MODES[id])));

/* ---- movie, for skits ---- */
// Mirrors the SERVER's view, not the page's. A reload used to reset this to
// false while the camera kept rolling, and the next press sent start again.
let recording = false;
$("#btnRec").addEventListener("click", async () => {
  if (!recording && lever === "photo")
    flash($("#exportOut"), "The camera is on PHOTO: move its lever to VIDEO to record.", true);
  toggleEpoch++;
  const r = await api("/api/movie", {on: !recording});
  toggleEpoch++;
  if (!r.ok) return flash($("#exportOut"), r.msg, true);
  recording = !recording;
  $("#btnRec").textContent = recording ? "Stop rec" : "Record";
  $("#btnRec").setAttribute("aria-pressed", String(recording));
  body.dataset.shooting = recording ? "1" : "0";
});

/* ---- orientation ---- */
$("#btnRot").addEventListener("click", async () => {
  state.rotation = (state.rotation + 90) % 360;
  $("#rotTxt").innerHTML = state.rotation + "&deg;";
  applyTransform();
  await api("/api/orient", {rotation: state.rotation});
});
$("#btnMir").addEventListener("click", async () => {
  state.mirror = !state.mirror;
  $("#btnMir").setAttribute("aria-pressed", String(state.mirror));
  applyTransform();
  await api("/api/orient", {mirror: state.mirror});
});
$("#btnGrid").addEventListener("click", () => {
  const order = ["off","thirds","center"];
  state.grid = order[(order.indexOf(state.grid)+1) % order.length];
  body.dataset.grid = state.grid;
  $("#btnGrid").textContent = "Grid: " + state.grid;
});

/* ---- shooting ---- */
$("#btnAf").addEventListener("click", async () => {
  setReview(false);
  const r = await api("/api/af", {});
  if (!r.ok) flash($("#exportOut"), r.msg, true);
});

function sleep(ms: number): Promise<void> { return new Promise(r => setTimeout(r, ms)); }

async function countdown(n: number){
  if (!n) return;
  body.dataset.count = "1";
  for (let i = n; i > 0; i--) { $("#count").textContent = String(i); await sleep(1000); }
  body.dataset.count = "0";
}

$("#btnShoot").addEventListener("click", async () => {
  setReview(false);                   // you frame a shot on the live view
  if (lever === "video")
    flash($("#exportOut"), "The camera is on VIDEO: move its lever to PHOTO for stills.", true);
  const btn = $("#btnShoot");
  const frames = parseInt($("#burst").value, 10) || 1;
  const timer  = parseInt($("#timer").value, 10) || 0;
  btn.disabled = true;
  await countdown(timer);
  body.dataset.shooting = "1";
  for (let i = 0; i < frames; i++) {
    btn.textContent = frames > 1 ? `Shooting ${i+1}/${frames}` : "Shooting…";
    // Focus on the FIRST frame of a burst only. A burst is one pose at one
    // distance and re-driving the lens between frames adds half a second each
    // and can hunt, which is how a ten-frame burst of the same expression
    // ends up with three soft frames in the middle.
    // Shooting while browsing an old day sends the frame to TODAY, so jump
    // there rather than leave the strip showing a day the frame is not in.
    // Pressing the shutter is not a request to add to yesterday.
    if (state.browsingPast) {
      await switchSession("");
      flash($("#exportOut"), "switched to today, which is where new frames go");
    }
    const r = await api("/api/shoot",
      {focus: $("#afFirst").checked && i === 0});
    if (!r.ok) { flash($("#exportOut"), r.error, true); break; }
    if (r.warning) { $("#toolWarn").textContent = r.warning;
                     $("#toolWarn").hidden = false; }
    await refresh();
    if (i < frames - 1) await sleep(700);
  }
  body.dataset.shooting = "0";
  btn.disabled = false; btn.textContent = "Shoot";
});

/* ---- camera settings ---- */
/* ---- camera settings as sliders ---------------------------------------
   Camera settings are CHOICE LISTS, not ranges: ISO has 19 values, shutter
   has 55. A slider over the index beats a dropdown for both, because the
   values are ordered and you want to nudge them, not hunt in a menu.

   Double-click resets to the value the setting had when this session first
   read it. The camera does not report a factory default over PTP, so
   "default" can only honestly mean "where it was when we got here", and the
   tooltip says exactly that. */
const CAM_DEFAULTS: Record<string, string> = {};     // key -> value first seen this session
const CAM_ORDER = ["iso", "shutter", "aperture", "exposurecomp",
                   "whitebalance", "expprogram", "quality", "focusmode",
                   "focallength"];
const CAM_LABEL: Record<string, string> = {iso: "ISO", shutter: "Shutter", aperture: "Aperture",
                   exposurecomp: "Exp. comp", whitebalance: "White balance",
                   expprogram: "Mode", quality: "Quality",
                   focusmode: "Focus mode", focallength: "Zoom"};

interface CamSetting { value: string; choices: string[]; readonly: boolean }
/** What the camera actually used, from the newest frame's EXIF. */
type Actual = Partial<Record<"iso" | "fNumber" | "exposureTime" | "exposureCompensation" | "focusMode"
  | "quality" | "lens" | "shutterCount", string>>;
interface SettingsReply { _notices?: string[]; _confirms?: string[]; _actual?: Actual | null;
                          battery?: CamSetting }
/** One mapped setting out of the reply, whose other keys are the fields above. */
const camField = (s: SettingsReply, k: string): CamSetting | undefined =>
  (s as unknown as Record<string, CamSetting | undefined>)[k];

async function loadSettings(){
  const s = await api("/api/settings");
  // What the camera ACTUALLY used, and every control that is not in control.
  // The panel below shows what was asked for, which is not the same thing:
  // with Auto ISO on it read ISO 640 while the camera shot 1400.
  if (s) {
    const nb = $("#camNotice"), ab = $("#camActual"), cb = $("#camConfirm");
    const notes = s._notices || [];
    nb.hidden = !notes.length;
    if (notes.length) nb.textContent = notes.join(" ");
    // The three settings he had to change ON the camera, because they are not
    // reachable over USB. A warning that stops appearing is not confirmation
    // that the change took, and it would never tell him about a knocked dial.
    const good = s._confirms || [];
    cb.hidden = !good.length;
    if (good.length) cb.textContent = good.join(" ");
    const a = s._actual;
    if (a) {
      const bits = [];
      if (a.iso) bits.push("ISO " + a.iso);
      if (a.fNumber) bits.push("f/" + a.fNumber);
      if (a.exposureTime) bits.push(a.exposureTime + "s");
      if (a.exposureCompensation && a.exposureCompensation !== "0")
        bits.push("exp " + a.exposureCompensation);
      if (a.focusMode) bits.push(a.focusMode);
      if (a.quality) bits.push(a.quality);
      ab.textContent = "Last frame actually used: " + bits.join(", ")
        + (a.lens ? " on the " + a.lens : "")
        + (a.shutterCount ? ". Shutter count " + a.shutterCount + "." : "");
      ab.hidden = false;
    } else { ab.hidden = true; }
  }
  const box = $("#camSettings");
  if (!s || !Object.keys(s).length) {
    box.innerHTML = '<div class="empty">No settings.</div>'; return;
  }
  if (s.battery) paintBattery(s.battery.value);

  // Don't rebuild while a slider is being dragged: the node would be replaced
  // under the pointer and the drag would die mid-gesture. Same for an open
  // dropdown anywhere in the panel.
  if (box.dataset.dragging === "1") return;
  if (box.querySelector("select") && selectBusy(box.querySelector("select")))
    return;

  box.innerHTML = "";
  CAM_ORDER.forEach(k => {
    const f = camField(s, k);
    if (!f) return;
    if (!(k in CAM_DEFAULTS)) CAM_DEFAULTS[k] = f.value;

    const row = document.createElement("div");
    row.className = "cs-row";
    const name = CAM_LABEL[k] || k;

    if (!f.choices.length || f.readonly) {
      // A bare "101.0" is ambiguous. The camera reports focal length as a
      // number with no unit, and for a portrait the unit is the whole point:
      // 24 and 120 on the same lens are different pictures of the same face.
      const shown = k === "focallength"
        ? `${Math.round(parseFloat(f.value))} mm` : f.value;
      row.innerHTML = `<span class="cs-name">${name}</span>`
        + `<span class="cs-val cs-ro">${shown}</span>`;
      box.appendChild(row);
      return;
    }

    const i = Math.max(0, f.choices.indexOf(f.value));
    const isDefault = f.value === CAM_DEFAULTS[k];
    row.innerHTML =
      `<span class="cs-name">${name}</span>`
      + `<input class="cs-slider" type="range" min="0" max="${f.choices.length - 1}"`
      + ` step="1" value="${i}" id="cs-${k}"`
      + ` title="drag to change, double-click to reset to ${CAM_DEFAULTS[k]}">`
      + `<span class="cs-val${isDefault ? "" : " cs-changed"}" id="csv-${k}">`
      + `${f.value}</span>`;
    box.appendChild(row);

    const sl = row.querySelector("input") as HTMLInputElement;
    const out = row.querySelector(".cs-val") as HTMLElement;

    const show = (idx: number) => {
      out.textContent = f.choices[idx];
      out.classList.toggle("cs-changed", f.choices[idx] !== CAM_DEFAULTS[k]);
    };
    // Show the value while dragging, but only TELL THE CAMERA on release:
    // a drag across 55 shutter speeds would otherwise be 55 USB round trips.
    sl.addEventListener("pointerdown", () => { box.dataset.dragging = "1"; });
    sl.addEventListener("input", () => show(+sl.value));
    const commit = async () => {
      delete box.dataset.dragging;
      const want = f.choices[+sl.value];
      if (want === f.value) return;
      out.classList.add("cs-busy");
      const r = await api("/api/setting", {key: k, value: want});
      out.classList.remove("cs-busy");
      if (!r.ok) {
        flash($("#exportOut"), name + ": " + (r.msg || "refused"), true);
        sl.value = String(Math.max(0, f.choices.indexOf(f.value)));
        show(+sl.value);
        return;
      }
      f.value = want;
      loadSettings();
    };
    sl.addEventListener("change", commit);
    ["pointerup", "pointercancel", "lostpointercapture"].forEach(event =>
      sl.addEventListener(event, () => { delete box.dataset.dragging; }));
    sl.addEventListener("dblclick", () => {
      const d = CAM_DEFAULTS[k];
      const di = f.choices.indexOf(d);
      if (di < 0) return;
      sl.value = String(di); show(di); commit();
    });
  });
}

/* ---- session strip ---- */
// Bumped by every button that starts or stops a recording. A state answer
// that was requested under an older epoch must not overrule the button.
let toggleEpoch = 0;
async function refresh(signal?: AbortSignal){
  const epoch = toggleEpoch;
  const lookEpoch = callLookEpoch;
  const st = await api("/api/state", undefined, signal);
  if (signal?.aborted) return;
  restoreQueuedEdits(st.dir);
  const cl = $("#callLook"), callId = st.config.webcam_look_id;
  if (!callLookPending && lookEpoch === callLookEpoch && !selectBusy(cl)
      && [...cl.options].some(o => o.value === callId))
    cl.value = callLookSaved = callId;
  if (state.session && state.session !== st.session) {
    // A second device can change the shared session. Re-select in that day;
    // queued edits still carry the day on which they were made.
    state.sel = null;
    marked.clear(); paintMarks();
    $("#cropImg").removeAttribute("src");
    $("#cropBox").hidden = true;
  }
  state.session = st.session;
  paintNextStep(st.nextStep);
  // The status lamps, the idle message and the Tether/Webcam/Off buttons used
  // to be painted ONCE at boot and after a mode button. Plugging the camera in
  // (auto-connect happens server side) left the page saying "No camera" until
  // a reload. Every refresh repaints them now, and the poll below refreshes.
  if (!modeBusy) {
    paintMode(st.connected ? "tether" : (st.webcam ? "webcam" : "off"),
              st.connected, st.webcam, st);
    if (st.connected) delete $("#idle").dataset.sticky;
  }
  if (typeof st.liveTurn === "number" && st.liveTurn !== state.liveTurn) {
    state.liveTurn = st.liveTurn;
    applyTransform();
  }
  state.shots = st.shots || [];
  state.editMap = st.edits || {};
  for (const {session, name, edit} of pendingEdits.values())
    if (session === state.session) state.editMap[name] = edit;
  state.clips = st.clips || [];    // the trim controls need the clip, not just its row
  state.takes = st.takes || [];
  renderTakes(state.takes);
  state.edits = st.edits || {};    // so a caller can see WHICH grade a frame carries
  paintGradeNote();
  state.trash = st.trash || 0;
  // Follow the SERVER's view of the audio take, not the page's: a reload used
  // to show "Record audio" while pw-record kept writing.
  //
  // Only when no button changed the recording state while this request was in
  // flight. With the page polling, an answer fetched just BEFORE Stop landed
  // just AFTER it and flipped the button back to "Stop audio" for a cycle.
  const ar = st.audioRec;
  const fresh = epoch === toggleEpoch;
  if (fresh && !!ar !== audioRec) {
    audioRec = !!ar;
    $("#btnRecAudio").textContent = audioRec ? "Stop audio" : "Record audio";
    $("#btnRecAudio").classList.toggle("busy", audioRec);
  }
  // The running time rides in an attribute drawn by CSS, so the button's
  // label stays exactly "Stop audio" for everything that reads it.
  if (ar && audioRec) {
    $("#btnRecAudio").dataset.elapsed = `${Math.floor(ar.seconds / 60)}:`
      + String(Math.floor(ar.seconds % 60)).padStart(2, "0");
  } else {
    delete $("#btnRecAudio").dataset.elapsed;
  }
  if (fresh && typeof st.recording === "boolean" && st.recording !== recording) {
    recording = st.recording;
    $("#btnRec").textContent = recording ? "Stop rec" : "Record";
    $("#btnRec").setAttribute("aria-pressed", String(recording));
    body.dataset.shooting = recording ? "1" : "0";
  }
  // A missing binary used to surface only as a cryptic errno on one failed
  // export, and then on every export after it. Say it once, at the top.
  const miss = st.toolsMissing || [];
  state.toolsMissing = miss;
  const warn = $("#toolWarn");
  // A camera problem worth interrupting the session for goes in the same
  // banner: shooting an unusable raw format is only visible here, otherwise
  // the whole session exports as failures much later.
  const lines = miss.map((t: ServerState["toolsMissing"][number]) =>
    `<b>${esc(t.bin)}</b> is missing, so ${esc(t.why)} cannot work. `
    + `<code>sudo apt install ${esc(t.install)}</code>`);
  if (st.warning) lines.unshift(`<b>${esc(st.warning)}</b>`);
  warn.hidden = !lines.length;
  if (lines.length) warn.innerHTML = lines.join("<br>");
  // Nothing empties the trash automatically, and it should not: deleting his
  // frames to save disk is the wrong trade. Showing the cost is not.
  state.trashMB = st.trashMB || 0;
  state.webcam = !!st.webcam;
  // Populate the camera panel as soon as we know the camera is there. It used
  // to appear only on the 15 second refresh interval, so opening the page with
  // the camera already connected showed an empty panel for up to 15 seconds.
  if (st.connected && !$("#camSettings").querySelector(".cs-row")
      && $("#camSettings").dataset.loading !== "1") {
    $("#camSettings").dataset.loading = "1";
    loadSettings().catch(() => {}).finally(() => { delete $("#camSettings").dataset.loading; });
  }
  $("#btnEmptyTrash").textContent = state.trash
    ? `Empty trash (${state.trash}, ${state.trashMB >= 1024
        ? (state.trashMB / 1024).toFixed(1) + " GB"
        : Math.round(state.trashMB) + " MB"})`
    : "Empty trash";
  $("#btnEmptyTrash").disabled = !state.trash;
  renderClips(state.clips);
  $("#dirNote").textContent = st.dir;
  // Say it where the shutter is, not in the session panel: the mistake is
  // pressing Shoot, and that is where the eye is.
  const pw = $("#pastWarn");
  pw.hidden = !st.browsingPast;
  // Rebuilt when the browsed day changes. It was built ONCE, so browsing a
  // second past day kept naming the first one.
  if (st.browsingPast && pw.dataset.built !== st.session) {
    pw.dataset.built = st.session;
    pw.innerHTML = `<span>Showing <b>${esc(st.session)}</b>. New frames go to `
      + `today.</span><button type="button">Go to today</button>`;
    must(pw.querySelector("button"), "the Go to today button").addEventListener("click",
      () => switchSession(""));
  }
  state.exportDir = st.exportDir || "";
  state.browsingPast = !!st.browsingPast;
  $("#shotCount").textContent = state.shots.length ? "(" + state.shots.length + ")" : "";
  const strip = $("#strip");
  $("#btnExportAll").disabled = !state.shots.length;
  // Populated BEFORE the empty-day return below. It used to sit after it, so
  // on a day with no frames yet the session dropdown rendered completely
  // blank, which is exactly the morning when he needs it to reach yesterday's
  // work. Only a screenshot showed this; every DOM check passed.
  const sp = $("#sessionPick");
  // Keyed on every day AND its count, and the selection re-asserted: keyed on
  // the number of days alone, the counts never moved as frames were shot, and
  // "Open it" / "Go to today" left the dropdown naming the day just left.
  const spSig = JSON.stringify(st.sessions || []) + "|" + st.today;
  if (st.sessions && sp.dataset.n !== spSig && !selectBusy(sp)) {
    sp.dataset.n = spSig;
    sp.innerHTML = "";
    st.sessions.forEach(x => {
      const o = document.createElement("option");
      o.value = x.date;
      o.textContent = x.date + (x.date === st.today ? "  today" : "")
        + `  \u00b7  ${x.frames} frame${x.frames === 1 ? "" : "s"}`;
      sp.appendChild(o);
    });
    sp.onchange = () => switchSession(sp.value);
  }
  if (!selectBusy(sp) && sp.value !== st.session) sp.value = st.session;
  $("#btnExportAll").disabled = !state.shots.length;
  if (!state.shots.length) {
    // A new day starts empty, and the work from last night is one tab away.
    // "No shots yet" is true and unhelpful: say where the frames are.
    const other = (st.sessions || []).filter(x => x.date !== st.session
                                                  && x.frames > 0);
    const esig = "empty|" + st.session + "|" + (other[0] ? other[0].date : "");
    if (esig === stripSig) return st;
    if (other.length) {
      const o = other[0];
      strip.innerHTML = `<div class="empty">No shots yet today.<br>`
        + `<b>${o.date}</b> has ${o.frames} frame${o.frames === 1 ? "" : "s"}. `
        + `<button type="button" id="goPrev">Open it</button></div>`;
      const gp = $("#goPrev");
      if (gp) gp.addEventListener("click", () => switchSession(o.date));
    } else {
      strip.innerHTML = '<div class="empty">No shots yet.</div>';
    }
    stripSig = esig;
    return st;
  }
  const shown = shownShots();
  // Rebuild only when what the strip SHOWS changed. The page now polls, and
  // tearing down every thumbnail every few seconds flickers, loses the scroll
  // position, and detaches a cell from under a click in progress.
  const sig = filterMode + "|" + st.session + "|" + shown.map(s =>
    s.name + ":" + (s.pick || "") + ":" + (s.edited ? 1 : 0) + ":" + (s.provisional ? 1 : 0)).join(",");
  if (sig === stripSig) {
    if (!state.sel && state.shots.length) select(state.shots[0].name);
    return st;
  }
  stripSig = sig;
  strip.innerHTML = "";
  if (!shown.length) { strip.innerHTML = '<div class="empty">Nothing matches this filter.</div>'; }
  shown.forEach(s => {
    const cell = document.createElement("div");
    cell.className = "cell";
    if (s.pick) cell.dataset.p = s.pick;
    const im = document.createElement("img");
    // A provisional raw tile gets its own URL, so the developed one is
    // fetched fresh rather than served from the browser's cache.
    im.src = "/img/" + encodeURIComponent(s.name) + "?w=240" + (s.provisional ? "&p=1" : "");
    im.title = s.name; im.loading = "lazy";
    im.dataset.on = (state.sel === s.name) ? "1" : "0";
    if (marked.has(s.name)) cell.dataset.mark = "1";
    // Click selects. SHIFT-click takes the run between the last click and this
    // one. Ctrl or Cmd click toggles one. Exporting frames one at a time was
    // the single most tedious thing left in the app.
    im.addEventListener("click", (ev) => {
      const names = shown.map(x => x.name);
      if (ev.shiftKey && anchorName && names.includes(anchorName)) {
        const a = names.indexOf(anchorName), b = names.indexOf(s.name);
        for (let i = Math.min(a, b); i <= Math.max(a, b); i++)
          marked.add(names[i]);
      } else if (ev.ctrlKey || ev.metaKey || touchSelecting) {
        marked.has(s.name) ? marked.delete(s.name) : marked.add(s.name);
        anchorName = s.name;
      } else {
        marked.clear();
        anchorName = s.name;
      }
      select(s.name);
      setReview(true);
      paintMarks();
    });
    cell.appendChild(im);
    if (s.pick) cell.appendChild(document.createElement("b"));
    if (s.edited) { const e = document.createElement("u"); cell.appendChild(e); }
    strip.appendChild(cell);
  });
  if (!state.sel && state.shots.length) select(state.shots[0].name);
  return st;
}

// Frames marked for a batch export. Separate from `state.sel`, which is the
// one frame being GRADED: you mark ten to export and still edit one of them.
const marked = new Set();
let touchSelecting = false;
$("#btnSelectFrames").addEventListener("click", () => {
  touchSelecting = !touchSelecting;
  $("#btnSelectFrames").setAttribute("aria-pressed", String(touchSelecting));
  $("#btnSelectFrames").textContent = touchSelecting ? "Done selecting" : "Select frames";
});
let anchorName: string | null = null;
let stripSig = "";

// The frames the strip is showing under the current filter. Arrow keys walk
// THIS list: walking every shot landed on rejected frames hidden by the filter.
function shownShots(){
  return state.shots.filter(s =>
    filterMode === "all" ? true :
    filterMode === "pick" ? s.pick === "pick" : s.pick !== "reject");
}

function paintMarks(){
  $$("#strip .cell").forEach(c => {
    const img = c.querySelector("img");
    const n = img && img.title;
    if (n && marked.has(n)) c.dataset.mark = "1"; else delete c.dataset.mark;
  });
  const n = marked.size;
  $("#btnExport").textContent = n > 1
    ? `Export ${n} selected frames` : "Export selected shot";
  $("#btnClearMarks").hidden = !n;
  $("#markNote").hidden = !n;
  if (n) $("#markNote").textContent =
    `${n} frames marked. Use Select frames to tap more; Shift-click selects a run.`;
}

function select(name: string){
  if (state.sel !== name) void flushEditSaves();
  state.sel = name;
  $$("#strip img").forEach(i => {
    i.dataset.on = (i.title === name) ? "1" : "0";
    // Keep the frame being graded in view: arrowing through forty frames
    // used to walk the selection off the end of the visible strip.
    if (i.title === name && i.scrollIntoView)
      i.scrollIntoView({block: "nearest", inline: "nearest"});
  });
  $("#btnExport").disabled = false;
  $("#btnDelete").disabled = false;
  $("#exportOut").textContent = "";
  prefetchForExport();
  applyEditsToUI(state.editMap && state.editMap[name]);
  hist = []; histAt = -1; histFor = name; setTimeout(snapshot, 0);
  refreshPreview(false);
  drawHistogram();
}

// ---- camera setting presets -------------------------------------------
// Settings drift: a dial gets knocked, a menu gets explored, and the next
// shoot starts from somewhere unknown. These are named snapshots of the
// values the camera will actually accept over USB.
// Never rebuild a <select> while the user has it open.
//
// Rebuilding the option list destroys the node the browser is showing the
// popup for, and the popup vanishes the instant it appears. The page refreshes
// several lists from /api/state and reloads the camera panel every 15 seconds,
// so any of them could close a dropdown under the pointer. A focused select is
// one that is open or about to be used, which is exactly when leaving it alone
// is right: it repopulates on the next pass a moment later.
function selectBusy(sel: HTMLElement | null): boolean {
  return !!sel && (document.activeElement === sel
                 || sel.dataset.open === "1");
}
["focus", "mousedown", "touchstart"].forEach(ev =>
  document.addEventListener(ev, e => {
    const t = e.target as HTMLElement | null;
    if (t && t.tagName === "SELECT") t.dataset.open = "1";
  }, true));
["change", "blur"].forEach(ev =>
  document.addEventListener(ev, e => {
    const t = e.target as HTMLElement | null;
    if (t && t.tagName === "SELECT")
      delete t.dataset.open;
  }, true));

function camPresetMsg(t: string, bad?: boolean) {
  const e = $("#camPresetOut");
  e.textContent = t; e.hidden = !t;
  e.style.color = bad ? "var(--amber)" : "var(--dim)";
}
/** The PHOTO/VIDEO lever lamp. The lever cannot be moved over USB (the
 *  camera reports it read-only), so the most the page can do is say which
 *  side it is on before a walk to the camera is wasted. */
let lever: "photo" | "video" | null = null;
function paintLever(v: "photo" | "video" | null) {
  lever = v;
  const lamp = $("#leverLamp");
  if (v) lamp.dataset.lever = v; else delete lamp.dataset.lever;
  $("#leverTxt").textContent = v ? v.toUpperCase() : "";
}
async function pollLever() {
  if (document.hidden || offline || body.dataset.conn !== "1") {
    if (body.dataset.conn !== "1") paintLever(null);
    return;
  }
  try {
    const r = await api("/api/lever");
    paintLever(r.lever);
  } catch (_) { /* keep the last reading */ }
}
/** The battery lamp: amber at 20% and under, red at 10%, with a word, so a
 *  camera about to drop off USB says so before it does. */
function paintBattery(value: string) {
  const pct = parseInt(String(value).replace(/[^0-9]/g, ""), 10);
  const lamp = $("#battLamp");
  const level = isNaN(pct) ? "" : pct <= 10 ? "crit" : pct <= 20 ? "low" : "";
  if (level) lamp.dataset.batt = level; else delete lamp.dataset.batt;
  $("#battTxt").textContent = "Batt " + value + (level === "crit" ? " (charge now)" : level === "low" ? " (low)" : "");
  lamp.title = level ? "The Z8 drops off USB when its battery runs down. Charge it or power it over USB-C." : "";
}
function fillCamPresets(list: string[] | null | undefined, keep?: string) {
  const sel = $("#camPreset");
  if (selectBusy(sel)) return;
  const want = keep !== undefined ? keep : sel.value;
  sel.innerHTML = "";
  (list || []).forEach(n => {
    const o = document.createElement("option");
    o.value = n; o.textContent = n; sel.appendChild(o);
  });
  if (!list || !list.length) {
    const o = document.createElement("option");
    o.value = ""; o.textContent = "no saved settings"; sel.appendChild(o);
  }
  if (want && (list || []).includes(want)) sel.value = want;
  const none = !list || !list.length;
  $("#btnCamApply").disabled = none;
  $("#btnCamDel").disabled = none;
}

/* ---- presets ---- */
async function loadPresets(){
  const pr = await api("/api/presets");
  state.presets = pr;
  const ex = $("#exportPreset"), lk = $("#lookPreset");
  pr.export.forEach(p => {
    const o = document.createElement("option");
    o.value = p.id; o.textContent = p.label + (p.w ? `  ${p.w}x${p.h}` : "");
    ex.appendChild(o);
  });
  pr.look.forEach(p => {
    const o = document.createElement("option"); o.value = p.id; o.textContent = p.label;
    lk.appendChild(o);
  });
  // A real option, not selectedIndex -1. Setting a select to a value no option
  // carries leaves it blank in Chrome and reading it back gives "", which is
  // indistinguishable from a preset whose id is empty.
  const am = document.createElement("option");
  am.value = ""; am.textContent = "Measured by Auto";
  lk.appendChild(am);

  // The call look, separate from the editing look.
  const cl = $("#callLook");
  if (!selectBusy(cl)) fillCallLook();
  cl.addEventListener("change", async () => {
    await saveCallLook();
  });
  ex.addEventListener("change", () => {
    showPresetNote();
    paintGradeNote();
    refitCrop();
  });
  $("#btnCamSave").addEventListener("click", async () => {
    const name = await askText("Save camera settings",
      "Name for the camera's current settings.", "Headshot");
    if (!name) return;
    const r = await api("/api/camera-preset", {action: "save", name});
    if (!r.ok) return camPresetMsg(r.error || "refused", true);
    fillCamPresets(Object.keys(r.presets).sort(), name);
    camPresetMsg("saved " + Object.keys(r.values || {}).length + " settings as \""
                 + name + "\"");
  });
  $("#btnCamApply").addEventListener("click", () => {
    const name = $("#camPreset").value;
    if (name) applyCamPreset(name, $("#btnCamApply"), "Restore");
  });
  // One press back to the default preset, at the top of the panel: a knocked
  // dial or a stray slider should not mean hunting through the saved list.
  $("#btnCamDefault").addEventListener("click", () => {
    const name = $("#btnCamDefault").dataset.preset || "Headshot";
    applyCamPreset(name, $("#btnCamDefault"), $("#btnCamDefault").textContent || "Restore default");
  });
  async function applyCamPreset(name: string, b: HTMLButtonElement, label: string) {
    b.disabled = true; b.textContent = "...";
    try {
      const r = await api("/api/camera-preset", {action: "apply", name});
      await loadSettings();
      // Per setting, never one verdict for eight: the camera declines writes
      // for reasons that have nothing to do with this app, and one summary
      // hides which one is actually stuck.
      const refused = "refused" in r ? r.refused || [] : [];
      const applied = "applied" in r ? r.applied || [] : [];
      if (!r.ok && !refused.length)
        camPresetMsg("error" in r && r.error ? r.error : "refused", true);
      else if (refused.length)
        camPresetMsg("restored " + applied.length + ", but the camera"
          + " refused " + refused.map(x => x.key).join(", ")
          + " (" + refused[0].error + ")", true);
      else
        camPresetMsg("restored " + applied.length + " settings from \"" + name + "\"");
    } finally { b.disabled = false; b.textContent = label; }
  }
  $("#btnCamDel").addEventListener("click", async () => {
    const name = $("#camPreset").value;
    if (!name) return;
    const r = await api("/api/camera-preset", {action: "delete", name});
    if (!r.ok) return camPresetMsg(r.error || "refused", true);
    fillCamPresets(Object.keys(r.presets).sort(), "");
    camPresetMsg("deleted \"" + name + "\"");
  });

  $("#btnLight").addEventListener("click", async () => {
    if (!state.sel) return flash($("#exportOut"), "select a frame first", true);
    const b = $("#btnLight");
    b.disabled = true; b.textContent = "...";
    try {
      const e = currentEdits();
      const r = await api("/api/lighting",
        {name: state.sel, crop: e.crop, straighten: e.adjust.straighten || 0});
      const box = $("#lightOut");
      if (!r.ok) {
        box.innerHTML = '<p class="note">' + esc(r.error || "could not measure")
          + "</p>";
        box.hidden = false; return;
      }
      const m = r.measured;
      // The measurements first, then the verdicts. A verdict with no number
      // behind it is an opinion, and this panel exists because an opinion
      // about lighting sent a whole shoot down the wrong path.
      const rows = [
        // A band with no skin to measure reads n/a. It used to print "null levels".
        ["eyes vs cheeks", m.eyeVsCheek == null ? "n/a"
          : (m.eyeVsCheek > 0 ? "+" : "") + m.eyeVsCheek + " levels",
          m.eyeVsCheek != null && m.eyeVsCheek > -12],
        ["catchlight", m.catchlightPx ? `found (${m.catchlightPx} px)` : "none found",
          m.catchlightPct >= 0.001],
        ["range used", m.rangeUsed + "% (" + m.p1 + " to " + m.p99 + ")",
          m.rangeUsed >= 75],
        ["face vs backdrop", m.separation == null ? "n/a" : "+" + m.separation + " levels",
          m.separation != null && m.separation >= 40],
        ["lit / shadow", m.litShadowRatio,
          m.litShadowRatio >= 1.7 && m.litShadowRatio <= 4.0],
      ];
      box.innerHTML =
        '<table class="lt">' + rows.map(([k, v, ok]) =>
          `<tr><td>${k}</td><td>${v}</td>`
          + `<td class="${ok ? "lt-ok" : "lt-no"}">${ok ? "ok" : "fix"}</td></tr>`
        ).join("") + "</table>"
        + r.notes.map((n: {level: string; what: string; why: string; fix: string}) =>
          `<div class="notice ${n.level === "good" ? "lt-good" : ""}">`
          + `<b>${esc(n.what)}</b>`
          + (n.why ? `<br>${esc(n.why)}` : "")
          + (n.fix ? `<br><i>${esc(n.fix)}</i>` : "") + "</div>").join("")
        + (m.skinOnlyBands ? "" :
           '<p class="note">Too little skin in the bands to measure the eyes'
           + " reliably on this frame.</p>");
      box.hidden = false;
      flash($("#exportOut"), "lighting: " + r.notes.length + " note"
            + (r.notes.length === 1 ? "" : "s"));
    } finally { b.disabled = false; b.textContent = "Light"; }
  });

  $("#btnAuto").addEventListener("click", async () => {
    if (!state.sel) return flash($("#exportOut"), "select a frame first", true);
    const b = $("#btnAuto");
    b.disabled = true; b.textContent = "...";
    try {
      // Measured on the crop that will actually be exported. Grading the
      // uncropped file measures the backdrop the crop is about to discard.
      const e = currentEdits();
      const r = await api("/api/auto-look",
        {name: state.sel, crop: e.crop, straighten: e.adjust.straighten || 0});
      if (!r.ok) return flash($("#exportOut"), r.error || "auto failed", true);
      SLIDERS.forEach(k => {
        if (k === "straighten") return;       // geometry is not a look
        if (k in r.adjust) { knob(k).value = String(r.adjust[k]); syncOut(k); }
      });
      // The look select no longer describes the sliders, so say so rather than
      // leave it naming a preset these values are not.
      lk.value = "";
      const w = $("#autoWhy");
      w.textContent = r.why.length ? r.why.join(". ") + "."
                                   : "already correct, nothing to change.";
      w.hidden = false;
      applyTransform(); schedulePreview(); queueEditSave();
      flash($("#exportOut"), "auto: " + r.why.length + " correction"
            + (r.why.length === 1 ? "" : "s"));
    } finally { b.disabled = false; b.textContent = "Auto"; }
  });

  lk.addEventListener("change", () => {
    const p = presets().look.find(x => x.id === lk.value);
    if (!p) return;
    $("#autoWhy").hidden = true;
    SLIDERS.forEach(k => { knob(k).value = String(p[k] ?? NEUTRAL[k]); syncOut(k); });
    applyTransform(); schedulePreview(); queueEditSave();
  });
  showPresetNote();
}
let callLookWrites: Promise<void> = Promise.resolve();
let callLookSaved = "", callLookPending = 0, callLookEpoch = 0;
function saveCallLook(report = true): Promise<void> {
  const cl = $("#callLook"), id = cl.value;
  const label = cl.selectedOptions[0]?.textContent || "Call look";
  const payload = id === "sliders" ? {id, look: adj(), follow: !report} : {id};
  callLookEpoch++;
  cl.disabled = true;
  callLookPending++;
  // Serialize slider changes too, so a slow earlier write cannot win last.
  callLookWrites = callLookWrites.then(async () => {
    const r = await api("/api/call-look", payload, undefined, true);
    if (!r.ok) throw new Error(r.error || "could not change the call look");
    callLookSaved = r.id;
    if (cl.value === id) cl.value = r.id;
    if (report) flash($("#exportOut"), label + (r.live ? " — updated in Zoom/OBS" : " — saved for Webcam mode"));
  }).catch(e => {
    if (cl.value === id) cl.value = callLookSaved;
    flash($("#exportOut"), "Call look failed: " + (e instanceof Error ? e.message : String(e)), true);
  }).finally(() => { callLookEpoch++; cl.disabled = --callLookPending > 0; });
  return callLookWrites;
}
// Refresh after saving a preset too, while preserving a focused selection.
function fillCallLook(){
  const cl = $("#callLook");
  if (selectBusy(cl)) return;
  const keep = cl.value;
  cl.innerHTML = "";
  const none = document.createElement("option");
  none.value = ""; none.textContent = "Call: no look";
  cl.appendChild(none);
  presets().look.forEach(pp => {
    const o = document.createElement("option");
    o.value = pp.id; o.textContent = "Call: " + pp.label;
    cl.appendChild(o);
  });
  const sl = document.createElement("option");
  sl.value = "sliders"; sl.textContent = "Call: follow the edit sliders";
  cl.appendChild(sl);
  if ([...cl.options].some(o => o.value === keep)) cl.value = keep;
}
function showPresetNote(){
  const p = presets().export.find(x => x.id === $("#exportPreset").value);
  $("#presetNote").textContent = p && p.note ? p.note : "";
  // The crop's shape comes from the export preset, which lives on another
  // tab. Say which one, so a square crop is never a mystery.
  $("#cropShape").textContent = p
    ? `Shape follows the export preset: ${p.label}`
      + (p.w ? ` (${p.w}\u00d7${p.h})` : "") + ". Change it on the Export tab."
    : "";
}

// The toggle lives in the server config, not in localStorage: it changes what
// the SHUTTER does, and a setting that governs the camera should not depend on
// which browser opened the page.
$("#afFirst").addEventListener("change", async () => {
  await api("/api/config", {autofocus_before_shoot: $("#afFirst").checked});
});

/* ---- ungraded exports ----
   On 2026-09-22 a headshot went out with every slider at neutral, the
   camera's own flat JPEG with a crop, and nothing on the Export tab said so.
   "As shot" is a legitimate choice, so this never blocks: it says it, above
   the button, with the fix one press away. Silent for the preset that copies
   the camera file, where ignoring the grade is the whole point. */
function isNeutral(a: Partial<Adjust> | null | undefined){
  return SLIDERS.every(k => k === "straighten"
    || Math.abs(((a || {})[k] ?? NEUTRAL[k]) - NEUTRAL[k]) < 1e-6);
}
function paintGradeNote(){
  if (!state.presets) return;
  const p = presets().export.find(x => x.id === $("#exportPreset").value);
  const copy = !!p && String(p.format || "").toUpperCase() === "COPY";
  $("#gradeNote").hidden = !state.sel || copy || !isNeutral(adj());
  const picksOnly = $("#picksOnly").checked;
  const q = (state.shots || []).filter(sh => sh.pick !== "reject"
    && (!picksOnly || sh.pick === "pick"));
  const bare = q.filter(sh => isNeutral(((state.edits || {})[sh.name] || {}).adjust));
  $("#batchGradeNote").hidden = copy || !bare.length;
  if (bare.length) $("#batchGradeTxt").textContent =
    `${bare.length} of ${q.length} frame${q.length === 1 ? "" : "s"}`
    + ` ${bare.length === 1 ? "has" : "have"} no grade and will export flat.`;
}
$("#btnGradeAuto").addEventListener("click", () => $("#btnAuto").click());
$("#btnBatchGradeAuto").addEventListener("click", () =>
  autoAll($("#picksOnly").checked));
$("#picksOnly").addEventListener("change", paintGradeNote);

function syncOut(k: Knob){
  const v = parseFloat(knob(k).value);
  must($("#o"+k[0].toUpperCase()+k.slice(1)), "the readout for " + k).textContent = v.toFixed(2);
  // A moved knob reads amber, so a stray +0.03 of tint is visible at a glance.
  const row = knob(k).closest<HTMLElement>(".sld");
  if (row) { if (Math.abs(v - NEUTRAL[k]) > 1e-6) row.dataset.moved = "1";
             else delete row.dataset.moved; }
  paintGradeNote();
}
SLIDERS.forEach(k => {
  knob(k).addEventListener("input", () => { setReview(true); syncOut(k); applyTransform(); nudgeStage(); schedulePreview(); queueEditSave(); });
  // Double click puts one knob back to neutral without touching the others.
  knob(k).addEventListener("dblclick", () => {
    knob(k).value = String(NEUTRAL[k]);
    knob(k).dispatchEvent(new Event("input"));
  });
  $("[data-reset='" + k + "']")?.addEventListener("click", () => {
    knob(k).value = String(NEUTRAL[k]);
    knob(k).dispatchEvent(new Event("input"));
  });
  syncOut(k);
});

/* ---- export ---- */
// Every status message used to land in one line at the bottom of the Export
// panel, below the fold from every other panel, so "select a frame first" or
// a refused camera setting happened silently. The message area is now pinned
// under the tabs, and the first line of it also shows as a toast over the
// stage, where the eye is.
let toastTimer: number | undefined;
function toast(msg: string | null | undefined, bad?: boolean){
  const t = $("#toast");
  if (!t || !msg) return;
  t.textContent = String(msg).split("\n")[0];
  t.classList.toggle("is-bad", !!bad);
  t.classList.add("is-on");
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => t.classList.remove("is-on"), bad ? 5200 : 3200);
}
function flash(el: HTMLElement, msg: string | null | undefined, bad?: boolean){
  el.textContent = msg || ""; el.className = "out" + (bad ? " err" : "");
  el.style.color = "";
  if (el.id === "exportOut") toast(msg, bad);
}

$("#btnClearMarks").addEventListener("click", () => {
  marked.clear(); anchorName = state.sel; paintMarks();
});

let manyJob: string | null = null;
function showDownloads(files: {name: string; path?: string}[]){
  const box = $("#downloadLinks");
  box.replaceChildren();
  for (const file of files) {
    const name = file.path?.split(/[\\/]/).pop() || file.name;
    const a = document.createElement("a");
    a.href = "/export/" + encodeURIComponent(name);
    a.download = name;
    a.textContent = "Download " + name;
    box.appendChild(a);
  }
  box.hidden = !box.childElementCount;
}
$("#btnExport").addEventListener("click", () => {
  void exportSelected().catch(() => {
    manyJob = null;
    $("#btnExport").disabled = !state.sel;
    $("#btnExport").classList.remove("busy");
    paintMarks();
    flash($("#exportOut"), "Connection lost during export. Check the computer's exports before retrying.", true);
  });
});
async function exportSelected(){
  if (!state.sel) return;
  await flushEditSaves();
  if (pendingEdits.size) return flash($("#exportOut"), "Reconnect and let your edits finish saving before exporting.", true);
  const btn = $("#btnExport");
  const preset = presets().export.find(p => p.id === $("#exportPreset").value);

  // A marked SET goes as a batch. Each frame keeps its OWN saved grade and
  // crop, which is the point: a crop framed on one face is wrong on the next,
  // so the sliders showing right now apply only to the frame being graded.
  // The counter said "cancel" while the button was DISABLED, so the cancel it
  // advertised could never be pressed. A second click now really cancels.
  if (manyJob) {
    await api("/api/job-cancel", {id: manyJob});
    btn.textContent = "Cancelling…";
    return;
  }
  if (marked.size > 1) {
    const names = [...marked];
    btn.disabled = true;
    const r = await api("/api/export-many",
      {names, preset: exportPreset(preset), adjust: adj()});
    if (!r.ok) { btn.disabled = false; paintMarks();
                 return flash($("#exportOut"), r.error, true); }
    manyJob = r.job;
    btn.disabled = false;
    btn.classList.add("busy");
    let st = {} as JobState;
    try {
      for (let i = 0; i < 6000; i++) {
        st = await job(r.job);
        if (btn.textContent !== "Cancelling…")
          btn.textContent = `${st.done || 0} / ${st.total}  cancel`;
        if (st.state !== "running") break;
        await sleep(400);
      }
    } finally {
      manyJob = null;
      btn.classList.remove("busy");
    }
    paintMarks();
    const bad = (st.errors || []).length;
    flash($("#exportOut"),
      `${(st.results || []).length} of ${st.total} exported`
      + (bad ? `, ${bad} failed: ${(st.errors[0] || {}).error || ""}` : "")
      + `  ->  ${state.exportDir || ""}`, !!bad);
    showDownloads(st.results || []);
    return;
  }

  btn.disabled = true; btn.textContent = "Rendering…";
  const full = !crop || (crop.x < .002 && crop.y < .002 && crop.w > .998 && crop.h > .998);
  const r = await api("/api/export",
    {name: state.sel, preset: exportPreset(preset), adjust: adj(),
     crop: full ? null : crop});
  btn.disabled = false; paintMarks();
  if (!r.ok) return flash($("#exportOut"), r.error, true);
  showDownloads([r]);
  // The server has always been able to hand the file straight back, but
  // nothing linked to it, so the only way to the export was a file manager.
  const out = $("#exportOut");
  out.textContent = "";
  out.style.color = "var(--ok)";
  const a = document.createElement("a");
  a.href = "/export/" + encodeURIComponent(r.name);
  a.download = r.name;
  a.textContent = r.name;
  a.style.color = "var(--ok)";
  out.appendChild(a);
  out.appendChild(document.createTextNode(
    `  ${r.dims[0]}x${r.dims[1]}  ${(r.size/1024).toFixed(0)} KB\n${r.path}`));
  toast(`Exported ${r.name}  ${r.dims[0]}\u00d7${r.dims[1]}`);
  // The Original preset hands over the camera's file untouched, so any grade
  // on the sliders was NOT applied. Reporting "exported" and leaving him to
  // discover that in DarkTable would be the worst version of this feature.
  if (r.copied) {
    const n = document.createElement("div");
    n.className = "hint";
    n.textContent = (r.ignored_edits && r.ignored_edits.length)
      ? `Copied the camera file untouched, so your ${r.ignored_edits.join(", ")}`
        + ` ${r.ignored_edits.length > 1 ? "were" : "was"} not applied.`
        + ` Use "Full size, lossless" if you want the grade kept.`
      : "Copied the camera file untouched, byte for byte.";
    out.appendChild(n);
  }
}


/* ---- crop ----------------------------------------------------------------
   Stored normalised 0..1 against the source frame, so the box the user drags
   on a small preview maps straight onto the full-resolution file and nothing
   here needs to know the real pixel size. */
let crop: Crop | null = null;              // {x,y,w,h} or null for the full frame
let lockRatio = true;

function presetRatio(){
  const p = state.presets && presets().export.find(x => x.id === $("#exportPreset").value);
  return (p && p.w && p.h) ? p.w / p.h : null;
}

// The crop pane's frame size. naturalWidth reads 0 while a new render of the
// same frame is loading, and every slider move reloads it, so the crop box
// used to VANISH for the length of each render and a drag begun in that
// moment grabbed nothing. The size of the frame does not change between
// renders of it, so remember it per frame.
function cropDims(){
  const img = $("#cropImg");
  if (img.naturalWidth) return {w: img.naturalWidth, h: img.naturalHeight};
  if (img.dataset.for && img.dataset.for === state.sel)
    return {w: Number(img.dataset.nw || 0), h: Number(img.dataset.nh || 0)};
  return null;
}

interface Box { left: number; top: number; w: number; h: number }
function imgBox(): Box | null {
  const img = $("#cropImg"), wrap = $("#cropWrap");
  if (!cropDims() || !img.getAttribute("src")) return null;
  const r = img.getBoundingClientRect(), wr = wrap.getBoundingClientRect();
  return {left: r.left - wr.left, top: r.top - wr.top, w: r.width, h: r.height};
}

function drawCrop(){
  const box = $("#cropBox"), b = imgBox();
  if (!b || !crop) { box.hidden = true; $("#cropInfo").textContent = ""; return; }
  box.hidden = false;
  box.style.left   = (b.left + crop.x * b.w) + "px";
  box.style.top    = (b.top  + crop.y * b.h) + "px";
  box.style.width  = (crop.w * b.w) + "px";
  box.style.height = (crop.h * b.h) + "px";
  const dm = cropDims() || {w: 0, h: 0}, nw = dm.w, nh = dm.h;
  $("#cropInfo").textContent =
    `${Math.round(crop.w*nw)}×${Math.round(crop.h*nh)} of source`;
}

// The preset as it will actually be used. "Whole frame" turns a cover preset
// into a contain one, which scales the picture to fit inside the target size
// rather than filling it and cropping the overflow.
function exportPreset(p: ExportPreset | undefined): ExportPreset | undefined {
  if (!p) return p;
  return $("#wholeFrame").checked ? Object.assign({}, p, {fit: "contain"}) : p;
}

// Where the export would place this preset's crop, as (x, y) in 0..1. The
// server serves the map with the presets so there is one definition: the crop
// tool used to centre vertically while a face-anchored preset biased to 0.35,
// which meant "reset crop" silently reframed the picture by 206 pixels of
// headroom on a portrait frame.
function presetCentering(){
  const p = state.presets && presets().export.find(
    x => x.id === $("#exportPreset").value);
  const map = (state.presets && state.presets._anchors) || {};
  const a = (p && p.anchor) || "center";
  const c = map[a] || map.center || [0.5, 0.5];
  return {x: c[0], y: c[1]};
}

function defaultCrop(){
  const r = lockRatio ? presetRatio() : null;
  const dm = cropDims();
  if (!dm) return null;
  if (!r) return {x:0, y:0, w:1, h:1};
  const src = dm.w / dm.h;
  const c = presetCentering();
  // Largest rect of the target ratio that still fits, placed where the export
  // would place it rather than always dead centre.
  return src > r ? {x:(1 - r/src) * c.x, y:0, w:r/src, h:1}
                 : {x:0, y:(1 - src/r) * c.y, w:1, h:src/r};
}


/* Changing the export aspect must refit the crop even when the preview has
   not finished loading. Silently skipping left a stale crop of the previous
   aspect, and the export then cropped twice. */
let refitPending = false;
$("#wholeFrame").addEventListener("change", () => {
  // Whole frame means the preset no longer dictates a shape, so locking the
  // crop to it would be meaningless.
  if ($("#wholeFrame").checked) {
    lockRatio = false;
    $("#cropLock").setAttribute("aria-pressed", "false");
  }
  refreshPreview(false);
});

function refitCrop(){
  if (!lockRatio) return;
  const img = $("#cropImg");
  if (!cropDims()) { refitPending = true; return; }
  refitPending = false;
  crop = defaultCrop();
  drawCrop();
  queueEditSave();
  refreshStage(false);
}

function clampCrop(c: Crop): Crop {
  c.w = Math.min(1, Math.max(0.05, c.w));
  c.h = Math.min(1, Math.max(0.05, c.h));
  c.x = Math.min(1 - c.w, Math.max(0, c.x));
  c.y = Math.min(1 - c.h, Math.max(0, c.y));
  return c;
}

interface Drag { h: string | undefined; sx: number; sy: number; start: Crop; b: Box }
let drag: Drag | null = null;
$("#cropWrap").addEventListener("pointerdown", (e: PointerEvent) => {
  const b = imgBox(); if (!b || !crop) return;
  const h = (e.target as HTMLElement | null)?.dataset?.h;
  drag = {h, sx:e.clientX, sy:e.clientY, start:{...crop}, b};
  $("#cropWrap").setPointerCapture(e.pointerId);
  e.preventDefault();
});
$("#cropWrap").addEventListener("pointermove", (e: PointerEvent) => {
  if (!drag) return;
  const dx = (e.clientX - drag.sx) / drag.b.w, dy = (e.clientY - drag.sy) / drag.b.h;
  const s = drag.start;
  if (!drag.h) {
    crop = clampCrop({x:s.x + dx, y:s.y + dy, w:s.w, h:s.h});
  } else {
    const r = lockRatio ? presetRatio() : null;
    const dm = cropDims() || {w: 1, h: 1};
    const srcRatio = dm.w / dm.h;
    let nx = s.x, ny = s.y, nw = s.w, nh = s.h;
    if (drag.h.includes("e")) nw = s.w + dx;
    if (drag.h.includes("w")) { nw = s.w - dx; nx = s.x + dx; }
    if (drag.h.includes("s")) nh = s.h + dy;
    if (drag.h.includes("n")) { nh = s.h - dy; ny = s.y + dy; }
    if (r) {
      // Height follows width so the box keeps the export aspect exactly.
      nh = (nw * srcRatio) / r;
      if (drag.h.includes("n")) ny = s.y + s.h - nh;
    }
    crop = clampCrop({x:nx, y:ny, w:nw, h:nh});
  }
  drawCrop();
});
["pointerup","pointercancel"].forEach(ev =>
  $("#cropWrap").addEventListener(ev, () => {
    if (drag) { drag = null; queueEditSave(); refreshStage(false); drawHistogram(); }
  }));

$("#cropReset").addEventListener("click", () => {
  // THE WHOLE FRAME, not "the biggest rectangle of the preset's shape".
  //
  // defaultCrop() returns {0,0,1,1} only when Lock to preset is off, and it
  // defaults to ON, so this button used to hand back a crop of the preset
  // aspect and call it Full frame. Pressing it has to release the lock as
  // well, or the name is a lie.
  lockRatio = false;
  $("#cropLock").setAttribute("aria-pressed", "false");
  crop = {x: 0, y: 0, w: 1, h: 1};
  drawCrop();
  // And the EXPORT has to stop cropping too. A preset with fit "cover" fills
  // its shape and throws away the overflow, so a full-frame crop still came
  // out cropped. "Whole frame" scales the picture to fit inside the preset's
  // box instead, which is what someone means when they say they will crop it
  // later themselves.
  $("#wholeFrame").checked = true;
  refreshPreview(false);
  flash($("#exportOut"),
        "whole frame: the export will not crop, it scales to fit the preset");
});
$("#cropLock").addEventListener("click", () => {
  lockRatio = !lockRatio;
  $("#cropLock").setAttribute("aria-pressed", String(lockRatio));
  if (lockRatio) refitCrop();
});
window.addEventListener("resize", drawCrop);



/* ---- true render preview -------------------------------------------------
   The crop panel shows what the export will actually produce. The server
   renders a downscaled copy with the same code path the export uses, so the
   sliders stop lying. Debounced, because each move is a real render. */
let previewTimer: number | undefined, previewSeq = 0;

function previewURL(original: boolean){
  const p = new URLSearchParams({name: state.sel || "", adjust: JSON.stringify(adj())});
  if (original) p.set("original", "1");
  p.set("s", String(++previewSeq));
  return "/api/preview?" + p.toString();
}

/* ---- the stage render ----------------------------------------------------
   The big picture is the same server render the export uses, graded AND
   cropped, at up to 1100 px. Two things make it feel immediate:
     - while a slider moves, a CSS filter nudges the picture already on screen
       by the RATIO of the new values to the ones it was rendered with, so it
       answers the pointer at once;
     - the real render loads off-screen and swaps in only when complete, and a
       late answer for an older slider position is thrown away, so the picture
       never blanks and never steps backwards. */
let stageSeq = 0;
let stageAdj: Adjust | null = null;          // the adjust the picture on screen was rendered with

function stageURL(original: boolean){
  const p = new URLSearchParams({name: state.sel || "", adjust: JSON.stringify(adj())});
  const full = !crop || (crop.x < .002 && crop.y < .002 && crop.w > .998 && crop.h > .998);
  if (!full) p.set("crop", JSON.stringify(crop));
  if (original) p.set("original", "1");
  return "/api/preview?" + p.toString();
}

const IS_HEADLESS = new URLSearchParams(location.search).has("e2e")
                 || new URLSearchParams(location.search).has("layout");
function refreshStage(original: boolean){
  const still = $("#still");
  // Not while live view owns the stage. Keyed on the review flag rather than
  // on `still.hidden`: paintMode renders the still BEFORE un-hiding it, so a
  // hidden check made the first click of a review wait a whole poll cycle.
  if (!state.sel || IS_HEADLESS
      || (body.dataset.conn === "1" && !state.review)) return;
  const name = state.sel;
  const url = stageURL(original);
  const want = original ? null : adj();
  const my = ++stageSeq;
  const t0 = performance.now();
  // A different frame: put its thumbnail up at once, already in the browser's
  // cache from the strip, so a click answers in one frame instead of after a
  // 45 MP decode. The render replaces it the moment it lands.
  if (still.dataset.name !== name && still.dataset.placeholder !== name) {
    still.src = "/img/" + encodeURIComponent(name) + "?w=240";
    still.dataset.placeholder = name;
    still.style.filter = "";
    stageAdj = null;
  }
  const im = new Image();
  im.onload = () => {
    if (state.sel !== name) return;
    // A newer request for the SAME frame is on its way. Drop this one, unless
    // the stage is still showing the placeholder: a render of the right frame
    // at the previous crop beats a blurry thumbnail while the next one loads.
    if (my !== stageSeq && still.dataset.name === name) return;
    still.src = url;
    still.dataset.name = name;
    delete still.dataset.placeholder;
    still.style.filter = "";
    stageAdj = want;
    prefetchNeighbours(name);
    stageStats.push(performance.now() - t0);
    if (stageStats.length > 50) stageStats.shift();
  };
  im.src = url;
  const badge = $("#stageBadge");
  badge.hidden = !original;
  if (original) badge.textContent = "Before";
}
const stageStats: number[] = [];        // render round trips, read by the benchmark

/* Once a photo is on the stage, have the server decode its neighbours in the
   strip, so arrowing to the next one renders from memory instead of waiting
   on a 17 MB decode. Fire and forget; the server caps the work. */
/* With the Export tab open the next press is Export: have the server decode
   the selected frame in full now, so the export reuses it instead of paying
   the ~270 ms decode itself. Never while culling on other tabs. */
/* Fire and forget, but READ the two-byte answer: WebKit keeps a response
   nobody consumed open, so a page that prefetched never went network-idle. */
function prefetch(query: string){
  void fetch("/api/prefetch?" + query).then(r => r.text()).catch(() => {});
}
function prefetchForExport(){
  if (!state.sel || $(".pane[data-pane=\"export\"]")?.hidden) return;
  prefetch("full=1&name=" + encodeURIComponent(state.sel));
}
function prefetchNeighbours(name: string){
  const list = shownShots();
  const i = list.findIndex(s => s.name === name);
  if (i < 0) return;
  for (const n of [i + 1, i - 1]) {
    if (n >= 0 && n < list.length)
      prefetch("name=" + encodeURIComponent(list[n].name));
  }
}

// Immediate, approximate feedback on the stage while the real render is out.
function nudgeStage(){
  const still = $("#still");
  if (!stageAdj || still.hidden) return;
  const a = adj(), b = stageAdj;
  const r = (k: Knob) => (a[k] / (b[k] || 1)).toFixed(3);
  const dw = a.warmth - b.warmth;
  const warm = dw > 0 ? ` sepia(${(dw * 0.5).toFixed(2)})`
             : dw < 0 ? ` hue-rotate(${(dw * 22).toFixed(0)}deg)` : "";
  still.style.filter = `brightness(${r("exposure")}) contrast(${r("contrast")})`
    + ` saturate(${r("saturation")})${warm}`;
}

function refreshPreview(original: boolean){
  if (!state.sel) return;
  const img = $("#cropImg");
  const first = !img.naturalWidth;
  const hadCrop = JSON.stringify(crop);
  img.onload = () => {
    img.dataset.for = state.sel || "";
    img.dataset.nw = String(img.naturalWidth);
    img.dataset.nh = String(img.naturalHeight);
    if (first || refitPending || !crop) { refitPending = false; crop = defaultCrop(); }
    drawCrop();
    // The crop is only known once the frame's size is: render the stage
    // again if loading settled it differently.
    if (JSON.stringify(crop) !== hadCrop) refreshStage(original);
  };
  img.src = previewURL(original);
  refreshStage(original);
}

function schedulePreview(){
  clearTimeout(previewTimer);
  previewTimer = setTimeout(() => {
    refreshPreview(false); drawHistogram();
    if (state.webcam && $("#callLook").value === "sliders") void saveCallLook(false);
  }, 220);
}

/* hold to see the untouched frame */
let comparing = false;
const cmp = () => { comparing = true; refreshPreview(true); };
const uncmp = () => { if (comparing) { comparing = false; refreshPreview(false); } };
["mousedown","touchstart"].forEach(e => $("#btnCompare").addEventListener(e, cmp));
["mouseup","mouseleave","touchend","touchcancel","pointercancel"].forEach(e =>
  $("#btnCompare").addEventListener(e, uncmp));
window.addEventListener("blur", uncmp);

$("#btnSaveLook").addEventListener("click", async () => {
  const label = $("#lookName").value.trim();
  if (!label) return flash($("#exportOut"), "give the look a name first", true);
  const r = await api("/api/look", Object.assign({label}, adj()));
  if (!r.ok) return flash($("#exportOut"), r.error, true);
  $("#lookName").value = "";
  const lk = $("#lookPreset");
  lk.innerHTML = "";
  state.presets = await api("/api/presets");
  presets().look.forEach(x => {
    const o = document.createElement("option"); o.value = x.id; o.textContent = x.label;
    lk.appendChild(o);
  });
  const am2 = document.createElement("option");
  am2.value = ""; am2.textContent = "Measured by Auto";
  lk.appendChild(am2);
  lk.value = r.id;
  fillCallLook();
  flash($("#exportOut"), "saved look: " + label);
});


/* ---- sessions and culling ---------------------------------------------- */
let filterMode = "all";

async function switchSession(d: string){
  await flushEditSaves();
  if (pendingEdits.size) return flash($("#exportOut"), "Reconnect and let your edits finish saving before changing sessions.", true);
  await api("/api/session", {date: d});
  marked.clear();
  paintMarks();
  state.sel = null;
  $("#btnExport").disabled = $("#btnDelete").disabled = true;
  $("#cropImg").removeAttribute("src");
  $("#cropBox").hidden = true;
  await refresh();
}

async function flagShot(flag: Pick){
  if (!state.sel) return;
  const was = state.sel;
  const r = await api("/api/pick", {name: state.sel, flag});
  if (!r.ok) return flash($("#exportOut"), r.error, true);
  await refresh();
  // Culling is a rhythm: decide, move on. Stopping on the frame you just
  // judged means reaching for the arrow key forty times in a shoot.
  // The next frame the strip SHOWS: under "Not rejected" the next raw frame
  // can be one the filter hides.
  if (flag) {
    const i = state.shots.findIndex(x => x.name === was);
    const vis = new Set(shownShots().map(x => x.name));
    const next = state.shots.slice(i + 1).find(x => vis.has(x.name));
    if (i >= 0 && next) { select(next.name); setReview(true); }
  }
}
$("#btnPick").addEventListener("click", () => flagShot("pick"));
$("#btnReject").addEventListener("click", () => flagShot("reject"));
$("#btnClearFlag").addEventListener("click", () => flagShot(null));

// Rank the session on the three faults that make a headshot unusable.
// Deliberately NOT a beauty judgement, and deliberately does not touch picks:
// culling is the photographer's call, and a tool that silently rejected frames
// would be trusted exactly once.
$("#btnRank").addEventListener("click", async () => {
  const btn = $("#btnRank");
  btn.disabled = true;
  let r: Routes["/api/rank"], st = {} as JobState;
  try {
    r = await api("/api/rank", {});
    if (!r.ok) return flash($("#exportOut"), r.error || "nothing to rank", true);
    for (let i = 0; i < 4000; i++) {
      st = await job(r.job);
      btn.textContent = `${st.done || 0} / ${st.total || r.total}`;
      if (st.state !== "running") break;
      await sleep(400);
    }
  } finally { btn.disabled = false; btn.textContent = "Rank"; }
  const rows = st.ranked || [];
  const box = $("#rankOut");
  const clean = rows.filter((x: Ranked) => x.ok && !(x.faults || []).length).length;
  box.innerHTML =
    `<p class="note">${clean} of ${rows.length} with no fault. `
    + `Click a row to open that frame.</p>`
    + '<table class="lt rk">' + rows.map((x: Ranked, i: number) => {
        if (!x.ok) return `<tr data-n="${esc(x.name)}"><td>${i + 1}</td>`
          + `<td>${esc(x.name.slice(9))}</td><td colspan="2" class="lt-no">`
          + `${esc(x.why || "could not measure")}</td></tr>`;
        const f = esc((x.faults || []).join(", "));
        return `<tr data-n="${esc(x.name)}"><td>${i + 1}</td>`
          + `<td>${esc(x.name.slice(9))}</td>`
          + `<td>${x.focusPct}% sharp</td>`
          + `<td class="${f ? "lt-no" : "lt-ok"}">${f || "clean"}</td></tr>`;
      }).join("") + "</table>";
  box.hidden = false;
  $("#rankSec").hidden = false;
  showTab("library");
  box.querySelectorAll<HTMLElement>("tr[data-n]").forEach(tr =>
    tr.addEventListener("click", () => { select(tr.dataset.n || ""); setReview(true); }));
  flash($("#exportOut"), `ranked ${rows.length} frames, ${clean} clean`);
});
$("#filterPick").addEventListener("change", () => { filterMode = $("#filterPick").value; refresh(); });


/* ---- per-frame edits -----------------------------------------------------
   Each frame keeps its own look and crop. Without this, selecting a second
   shot silently inherits the first one's grade and every export after the
   first is wrong. */
let editSaveTimer: number | undefined;
const pendingEdits = new Map<string, {session: string; name: string; edit: Edit}>();
let savingEdits: Promise<void> | null = null;
let editStoragePrefix = "";
function restoreQueuedEdits(dir: string){
  const prefix = "z8.pendingEdit:" + dir.slice(0, dir.lastIndexOf("/")) + ":";
  if (prefix === editStoragePrefix) return;
  // A server pointed at a different photo root must not receive old drafts.
  if (editStoragePrefix) pendingEdits.clear();
  editStoragePrefix = prefix;
  try {
    for (let i = 0; i < localStorage.length; i++) {
      const key = localStorage.key(i);
      if (!key?.startsWith(prefix)) continue;
      const p = JSON.parse(localStorage.getItem(key) || "null") as
        {session?: string; name?: string; edit?: Edit} | null;
      if (typeof p?.session === "string" && /^\d{4}-\d{2}-\d{2}$/.test(p.session) && typeof p.name === "string" && p.name
          && !p.name.includes("/") && p.edit?.adjust && typeof p.edit.adjust === "object")
        pendingEdits.set(p.session + "/" + p.name, {session: p.session, name: p.name, edit: p.edit});
    }
  } catch { /* Existing edits still work when browser storage is unavailable. */ }
}

function currentEdits(){
  const full = !crop || (crop.x < .002 && crop.y < .002 && crop.w > .998 && crop.h > .998);
  return {adjust: adj(), crop: full ? null : crop};
}

function queueEditSave(){
  if (!state.sel || !state.session) return;
  const edit = currentEdits();
  const key = state.session + "/" + state.sel;
  const pending = {session: state.session, name: state.sel, edit};
  pendingEdits.set(key, pending);
  try { localStorage.setItem(editStoragePrefix + key, JSON.stringify(pending)); }
  catch { flash($("#exportOut"), "Browser storage is unavailable. Keep this page open until edits save.", true); }
  state.editMap[state.sel] = edit;
  state.edits[state.sel] = edit;
  clearTimeout(editSaveTimer);
  editSaveTimer = setTimeout(() => { void flushEditSaves(); }, 500);
}

// Capture edits for THEIR frame, before a tap selects another one. Keep failed
// saves for reconnection, and serialize requests so an older reply cannot win.
function flushEditSaves(): Promise<void> {
  clearTimeout(editSaveTimer);
  if (savingEdits) return savingEdits;
  if (!pendingEdits.size) return Promise.resolve();
  let failed = false;
  savingEdits = (async () => {
    for (const [key, pending] of [...pendingEdits]) {
      const {session, name, edit} = pending;
      const r = await api("/api/edits", {session, name, ...edit}, undefined, true);
      if (!r.ok) throw new Error(r.error);
      if (pendingEdits.get(key) === pending) pendingEdits.delete(key);
      try {
        // A second tab may have written a newer edit to this same frame.
        if (localStorage.getItem(editStoragePrefix + key) === JSON.stringify(pending))
          localStorage.removeItem(editStoragePrefix + key);
      } catch { /* The server has saved this edit. */ }
      if (state.session === session && state.sel === name) snapshot();
      const row = state.session === session && state.shots.find(x => x.name === name);
      if (row) row.edited = true;
    }
  })().catch(e => {
    failed = true;
    const why = e instanceof TypeError || (e instanceof Error && e.name === "AbortError")
      ? "Check Wi-Fi and Tailscale." : e instanceof Error ? e.message : "Reconnect to the computer.";
    flash($("#exportOut"), "Edits are waiting to save. Keep this page open. "
      + why, true);
  }).finally(() => {
    savingEdits = null;
    if (!failed && pendingEdits.size)
      editSaveTimer = setTimeout(() => { void flushEditSaves(); }, 0);
  });
  return savingEdits;
}

function applyEditsToUI(e: Edit | null | undefined){
  const a = (e && e.adjust) || {};
  SLIDERS.forEach(k => { knob(k).value = String(a[k] ?? NEUTRAL[k]); syncOut(k); });
  crop = (e && e.crop) ? e.crop : null;   // null means refit on image load
}

$("#btnApplyPicks").addEventListener("click", async () => {
  const btn = $("#btnApplyPicks");
  btn.disabled = true;
  const r = await api("/api/edits-apply-all", {adjust: adj(), picks_only: true});
  btn.disabled = false;
  flash($("#exportOut"), r.applied
    ? `look copied to ${r.applied} picked frame${r.applied === 1 ? "" : "s"}`
    : "nothing is picked yet", !r.applied);
  refresh();
});

// Auto, per frame, as a job.
//
// "Look to all" copies ONE grade across the shoot, which is right when a hero
// frame sets the look and wrong when the frames differ. Measured on a real
// session: face brightness ran from 52 to 141 across 66 frames of one sitting,
// because the subject moves relative to the key. Grading each frame on its own
// pulled the spread from 57.6 levels to 16.2, which is the difference between
// a set and a pile of photos.
async function autoAll(picksOnly: boolean){
  const btn = picksOnly ? $("#btnAutoPicks") : $("#btnAutoAll");
  const label = btn.textContent;
  btn.disabled = true;
  const r = await api("/api/auto-look-all", {picks_only: picksOnly});
  if (!r.ok) {
    btn.disabled = false;
    return flash($("#exportOut"), r.error || "nothing to grade", true);
  }
  let st = {} as JobState;
  for (let i = 0; i < 4000; i++) {
    st = await job(r.job);
    btn.textContent = `${st.done || 0} / ${st.total || r.total}`;
    if (st.state !== "running") break;
    await sleep(400);
  }
  btn.disabled = false; btn.textContent = label;
  await refresh();
  // Reload the SELECTED frame's sliders, or the panel keeps showing the look
  // it had before the batch and the next export silently uses the stale one.
  // refresh() has just repopulated state.edits, so read it from there.
  // There is no GET /api/edits: the map arrives with /api/state, and asking
  // for a route that does not exist returned an empty body and silently left
  // the sliders showing the pre-batch look, which the next export would use.
  if (state.sel) {
    applyEditsToUI((state.edits || {})[state.sel]);
    applyTransform(); schedulePreview();
  }
  const bad = (st.errors || []).length;
  flash($("#exportOut"),
    `graded ${(st.results || []).length} frame${(st.results||[]).length===1?"":"s"}`
    + (bad ? `, ${bad} could not be measured` : "")
    + " (each on its own, crops untouched)", !!bad);
}
$("#btnAutoAll").addEventListener("click", () => autoAll(false));
$("#btnAutoPicks").addEventListener("click", () => autoAll(true));

$("#btnApplyAll").addEventListener("click", async () => {
  const btn = $("#btnApplyAll");
  btn.disabled = true; btn.textContent = "Applying…";
  const r = await api("/api/edits-apply-all", {adjust: adj()});
  btn.disabled = false; btn.textContent = "Look to all";
  await refresh();
  flash($("#exportOut"), `look applied to ${r.applied} frames (crops left alone)`);
});


/* ---- histogram -----------------------------------------------------------
   Drawn from the rendered frame, so it reflects the grade rather than the
   file. Clipping percentages matter more than the shape: a blown forehead or
   blocked-up hair cannot be recovered later. */
let histSeq = 0;
async function drawHistogram(){
  if (!state.sel) return;
  // Only the newest request may paint. Arrowing through frames fired one per
  // frame, and a slow answer for an earlier frame landed last and left its
  // histogram and clipping warning on the frame being looked at.
  const my = ++histSeq;
  let h: {r: number[]; g: number[]; b: number[]; clipHigh: number; clipLow: number} | null;
  try {
    const p = new URLSearchParams({name: state.sel || "", adjust: JSON.stringify(adj())});
    const full = !crop || (crop.x < .002 && crop.y < .002 && crop.w > .998 && crop.h > .998);
    if (!full) p.set("crop", JSON.stringify(crop));
    h = await (await fetch("/api/histogram?" + p.toString())).json();
  } catch (e) { return; }
  if (my !== histSeq || !h || !h.r) return;
  const c = $("#hist"), g = c.getContext("2d");
  if (!g) return;
  const W = c.width, H = c.height;
  g.clearRect(0, 0, W, H);
  g.fillStyle = "#0A0D11"; g.fillRect(0, 0, W, H);
  g.strokeStyle = "#1C2530"; g.lineWidth = 1;
  for (let i = 1; i < 4; i++) {
    g.beginPath(); g.moveTo(W*i/4, 0); g.lineTo(W*i/4, H); g.stroke();
  }
  const peak = Math.max(...h.r, ...h.g, ...h.b, 1);
  g.globalCompositeOperation = "lighter";
  const hh = h;
  ([["r","#E5484D"],["g","#3DD68C"],["b","#5B93F5"]] as const).forEach(([k, col]) => {
    g.fillStyle = col + "88"; g.beginPath(); g.moveTo(0, H);
    hh[k].forEach((v, i) => g.lineTo(i / 255 * W, H - (v / peak) * (H - 4)));
    g.lineTo(W, H); g.closePath(); g.fill();
  });
  g.globalCompositeOperation = "source-over";
  const warn = [];
  if (h.clipHigh > 0.15) warn.push(`${h.clipHigh.toFixed(2)}% blown`);
  if (h.clipLow  > 0.50) warn.push(`${h.clipLow.toFixed(2)}% crushed`);
  const el = $("#clipInfo");
  el.textContent = warn.length ? warn.join("  ") : "no clipping";
  el.style.color = warn.length ? "var(--tally)" : "var(--dim)";
}

/* ---- batch export and delete ---- */
/* A 40 frame raw batch takes minutes. The server runs it as a job so the
   request cannot time out; the button turns into a live counter and a
   cancel, because a batch you cannot stop is worse than no batch. */
let exportJob: string | null = null;

$("#btnExportAll").addEventListener("click", async () => {
  const btn = $("#btnExportAll");
  if (exportJob) {                       // second click cancels
    await api("/api/job-cancel", {id: exportJob});
    btn.textContent = "Cancelling…";
    return;
  }
  await flushEditSaves();
  if (pendingEdits.size) return flash($("#exportOut"), "Reconnect and let your edits finish saving before exporting.", true);
  const preset = presets().export.find(p => p.id === $("#exportPreset").value);
  const start = await api("/api/export-all",
    {preset: exportPreset(preset), adjust: adj(),
     picks_only: $("#picksOnly").checked});
  if (!start.ok) return flash($("#exportOut"), start.error, true);
  exportJob = start.job;
  btn.classList.add("busy");
  btn.textContent = `0 / ${start.total}  cancel`;

  while (true) {
    await new Promise(r => setTimeout(r, 400));
    const j = await fetch(`/api/job?id=${encodeURIComponent(exportJob || "")}`)
      .then(r => r.json()).catch(() => null);
    if (!j || !j.ok) {
      // A batch that silently vanishes is the worst outcome: the button just
      // goes back to normal and the reader has no idea how far it got.
      flash($("#exportOut"), (j && j.error)
        || "lost contact with the batch, check the exports folder", true);
      break;
    }
    if (j.state === "running") {
      btn.textContent = `${j.done} / ${j.total}  cancel`;
      $("#exportOut").textContent = `rendering ${j.done} of ${j.total}…`;
      $("#exportOut").style.color = "var(--dim)";
      continue;
    }
    const n = (j.results || []).length, e = (j.errors || []).length;
    const stopped = j.state === "cancelled";
    flash($("#exportOut"),
      `${stopped ? "stopped after " : "exported "}${n} frame${n === 1 ? "" : "s"}`
      + (j.elapsed ? ` in ${j.elapsed}s` : "")
      + (e ? `, ${e} failed: ` + j.errors.map((x: {error: string}) => x.error).join("; ") : ""),
      e > 0);
    showDownloads(j.results || []);
    break;
  }
  exportJob = null;
  btn.classList.remove("busy");
  btn.textContent = "Export all";
  refresh();
});

$("#btnOpenSession").addEventListener("click", () =>
  api("/api/open-folder", {which: "session"}));
$("#btnOpenExports").addEventListener("click", () =>
  api("/api/open-folder", {which: "exports"}));

/* Delete moves the frame aside rather than unlinking it. Culling is fast and
   one-handed, so a delete will eventually land on the wrong take; the cost of
   keeping it is disk, and the cost of not keeping it is the take. */
$("#btnDelete").addEventListener("click", async () => {
  if (!state.sel) return;
  const r = await api("/api/delete", {name: state.sel});
  if (!r.ok) return flash($("#exportOut"), r.error, true);
  lastDeleted = r.removed;
  // The stage forgets a deleted frame in every mode. With the camera live the
  // still is only HIDDEN, and it kept the deleted frame, ready to come back.
  const still = $("#still");
  if (still.dataset.name && r.removed.includes(still.dataset.name)) {
    still.removeAttribute("src");
    delete still.dataset.name;
  }
  $("#btnUndelete").hidden = false;
  flash($("#exportOut"), "deleted " + r.removed.join(", ")
        + "\nrecoverable, press Undo delete");
  state.sel = null;
  $("#btnExport").disabled = $("#btnDelete").disabled = true;
  refresh();
});

let lastDeleted: string[] | null = null;
$("#btnUndelete").addEventListener("click", async () => {
  if (!lastDeleted) return;
  const r = await api("/api/undelete", {names: lastDeleted});
  if (!r.ok) return flash($("#exportOut"), r.error, true);
  flash($("#exportOut"), "restored " + r.restored.join(", "));
  lastDeleted = null;
  $("#btnUndelete").hidden = true;
  refresh();
});

$("#btnEmptyTrash").addEventListener("click", async () => {
  const n = state.trash || 0;
  if (!n) return flash($("#exportOut"), "the trash is already empty");
  // The one genuinely unrecoverable action in the app, so it asks.
  if (!$("#btnEmptyTrash").dataset.armed) {
    $("#btnEmptyTrash").dataset.armed = "1";
    $("#btnEmptyTrash").textContent = `Delete ${n} for good?`;
    setTimeout(() => {
      delete $("#btnEmptyTrash").dataset.armed;
      $("#btnEmptyTrash").textContent = "Empty trash";
    }, 4000);
    return;
  }
  delete $("#btnEmptyTrash").dataset.armed;
  const r = await api("/api/empty-trash", {});
  flash($("#exportOut"), `removed ${r.removed} file${r.removed === 1 ? "" : "s"} for good`);
  lastDeleted = null;
  $("#btnUndelete").hidden = true;
  refresh();
});


$("#btnSheet").addEventListener("click", async () => {
  const btn = $("#btnSheet");
  btn.disabled = true; btn.textContent = "Building…";
  const r = await api("/api/contact-sheet", {picks_only: $("#picksOnly").checked});
  btn.disabled = false; btn.textContent = "Contact sheet";
  if (!r.ok) return flash($("#exportOut"), r.error, true);
  const pages = r.pages || [{name: r.name, frames: r.frames, dims: r.dims}];
  flash($("#exportOut"),
    `${r.frames} frames on ${pages.length} page${pages.length === 1 ? "" : "s"}\n`
    + pages.map((p: {name: string; frames: number; dims: [number, number]}) => `${p.name}  ${p.frames}  ${p.dims[0]}×${p.dims[1]}`).join("\n"));
});


/* ---- output settings and grade copy/paste ---- */
let clipboardEdits: {from: string; adjust: Adjust} | null = null;

$("#btnSaveCfg").addEventListener("click", async () => {
  const r = await api("/api/config", {
    export_dir: $("#exportDir").value.trim(),
    name_template: $("#nameTpl").value.trim() || "{stem}_{preset}",
  });
  if (!r.ok) return flash($("#cfgNote"), r.error, true);
  $("#cfgNote").textContent = "exports go to " + r.resolved;
  $("#cfgNote").style.color = "var(--ok)";
});

$("#btnCopyEdits").addEventListener("click", () => {
  if (!state.sel) return;
  clipboardEdits = {from: state.sel, adjust: adj()};
  $("#btnPasteEdits").disabled = false;
  flash($("#exportOut"), "copied the look from " + state.sel);
});

$("#btnPasteEdits").addEventListener("click", async () => {
  const copied = clipboardEdits;
  if (!copied || !state.sel) return;
  SLIDERS.forEach(k => {
    if (k === "straighten") return;          // geometry stays with its frame
    knob(k).value = String(copied.adjust[k] ?? NEUTRAL[k]);
    syncOut(k);
  });
  applyTransform(); schedulePreview(); queueEditSave();
  flash($("#exportOut"), "pasted the look onto " + state.sel);
});


/* ---- 1:1 focus loupe -----------------------------------------------------
   The live view is 1024x576, so focus simply cannot be judged from it. After
   the shot this pulls a native-resolution patch out of the real file. */
function openLoupe(fx: number, fy: number){
  if (!state.sel) return;
  const p = new URLSearchParams({
    name: state.sel || "", x: fx.toFixed(4), y: fy.toFixed(4),
    size: "560", adjust: JSON.stringify(adj()),
  });
  $("#loupeImg").src = "/api/loupe?" + p.toString();
  $("#loupe").hidden = false;
}
$("#btnFocusCheck").addEventListener("click", () =>
  openLoupe(crop ? crop.x + crop.w / 2 : 0.5, crop ? crop.y + crop.h / 2 : 0.5));
$("#loupe").addEventListener("click", (e: MouseEvent) => {
  if ((e.target as HTMLElement | null)?.id === "loupeImg") return;
  $("#loupe").hidden = true;
});
document.addEventListener("keydown", e => {
  if (e.key === "Escape") {
    $("#keys").hidden = true;
    $("#loupe").hidden = true;
    $("#playerVid").pause(); $("#player").hidden = true;
  }
});
// double click the preview: single click is already the crop drag
$("#cropWrap").addEventListener("dblclick", (e: MouseEvent) => {
  const b = imgBox(); if (!b || !state.sel) return;
  const wr = $("#cropWrap").getBoundingClientRect();
  const fx = (e.clientX - wr.left - b.left) / b.w;
  const fy = (e.clientY - wr.top - b.top) / b.h;
  if (fx < 0 || fx > 1 || fy < 0 || fy > 1) return;
  openLoupe(fx, fy);
});


/* ---- clips ---------------------------------------------------------------
   Skits land here. The poster is pulled a second in, because frame zero is
   usually a fade and every clip then looks identical in the strip. */
function renderClips(list: Clip[]){
  const sec = $("#clipsSec"), strip = $("#clipStrip");
  sec.hidden = !list.length;
  $("#clipCount").textContent = list.length ? `(${list.length})` : "";
  const sig = list.map(c => c.name + ":" + c.duration).join(",");
  if (strip.dataset.sig === sig) return;
  strip.dataset.sig = sig;
  strip.innerHTML = "";
  list.forEach(c => {
    const cell = document.createElement("div");
    cell.className = "cell";
    const im = document.createElement("img");
    im.src = "/api/poster?name=" + encodeURIComponent(c.name);
    im.title = c.name; im.loading = "lazy";
    im.addEventListener("click", () => playClip(c));
    const tag = document.createElement("span");
    tag.textContent = c.duration ? `${Math.floor(c.duration/60)}:${String(Math.round(c.duration%60)).padStart(2,"0")}` : "?";
    cell.appendChild(im); cell.appendChild(tag);
    strip.appendChild(cell);
  });
}

/* ---- layout probe ---------------------------------------------------------
   Opened with ?layout=1, the page measures its own geometry and writes it out.
   A DOM dump is identical at every window size, so it proves nothing about
   whether the layout actually holds. */
if (new URLSearchParams(location.search).get("layout") === "1") {
  // NOT window "load": with the camera connected the page opens the live view
  // MJPEG stream, which never completes, so load NEVER FIRES and this probe
  // reported nothing at all (0/4 window sizes) the moment real hardware
  // appeared. DOMContentLoaded plus a settle delay always fires.
  document.addEventListener("DOMContentLoaded", () => setTimeout(() => {
    const out = [];
    const vw = document.documentElement.clientWidth;
    const vh = document.documentElement.clientHeight;
    out.push(`viewport ${vw}x${vh}`);
    out.push(`scrollWidth ${document.documentElement.scrollWidth}`);
    // Anything interactive that is invisible, zero-sized, or off-screen is a
    // control the reader cannot reach.
    const bad: string[] = [];
    $$("button, select, input").forEach(el => {
      // A closed <dialog> is not on screen and not reachable by design.
      if (el.hidden || el.closest("[hidden]") || el.closest("dialog:not([open])")) return;
      const r = el.getBoundingClientRect();
      if (r.width < 4 || r.height < 4) bad.push(`${el.id || el.tagName}:tiny`);
      else if (r.right < 0 || r.left > vw) bad.push(`${el.id || el.tagName}:offscreen`);
      else if (r.height < 20 && el.tagName !== "INPUT") bad.push(`${el.id || el.tagName}:short`);
    });
    out.push("unreachable " + (bad.length ? bad.join(",") : "none"));
    const el = $("#layout");
    el.textContent = "LAYOUT-BEGIN\n" + out.join("\n") + "\nLAYOUT-END";
  }, 1200));
}

/* ---- keyboard help -------------------------------------------------------
   The shortcuts live in each button's title attribute, so build the list FROM
   those rather than typing it out twice: a hand-written table goes stale the
   first time a binding changes and then actively misleads. */
const KEY_EXTRA: [string, string][] = [["\u2190 \u2192", "previous / next frame"],
                   ["space", "shoot"], ["i / o", "trim in / out point"],
                   ["ctrl+z / ctrl+y", "undo / redo"], ["esc", "close an overlay"],
                   ["?", "this list"]];

function toggleKeys(){
  const box = $("#keys");
  if (box.hidden) {
    const rows: [string, string][] = [];
    document.querySelectorAll("button[title]").forEach(b => {
      const t = b.getAttribute("title");
      if (t && t.length <= 2) rows.push([t, (b.textContent || "").trim() || b.id]);
    });
    KEY_EXTRA.forEach(r => rows.push(r));
    $("#keysTbl").innerHTML = rows.map(
      ([k, v]) => `<tr><td>${k}</td><td>${v}</td></tr>`).join("");
  }
  box.hidden = !box.hidden;
}
$("#keys").addEventListener("click", () => { $("#keys").hidden = true; });
$("#btnKeys").addEventListener("click", toggleKeys);
$("#btnLive").addEventListener("click", () => setReview(false));

/* ---- clip trimming -------------------------------------------------------
   Cutting the slate off the head and the walk-away off the tail is the only
   video edit a self-tape actually needs, and doing it in another application
   means leaving this one mid-session. */
/** A clip to play: a session clip, or the cut just made (which has no `at`). */
interface ClipRef { name: string; w: number | null; h: number | null; duration: number | null; at?: number }
interface Trim { clip: ClipRef | null; in: number; out: number | null }
let trim: Trim = {clip: null, in: 0, out: null};
/** The last cut made, so Play the cut and Grab frame can reach it. */
let lastTrim: {name: string; duration: number | null; w: number | null; h: number | null} | null = null;

const tc = (t: number | null) => {
  if (t == null) return "end";
  const m = Math.floor(t / 60), sec = t - m * 60;
  return `${m}:${sec < 10 ? "0" : ""}${sec.toFixed(1)}`;
};

function paintTrim(){
  $("#trimTxt").textContent = `in ${tc(trim.in)}   out ${tc(trim.out)}`;
  const dur = (trim.clip ? trim.clip.duration : 0) || 0;
  const end = trim.out == null ? dur : trim.out;
  const bad = !(end - trim.in >= 0.2);
  $("#btnTrimGo").disabled = bad;
  $("#btnTrimGo").textContent = bad ? "Save trim"
    : `Save trim  ${(end - trim.in).toFixed(1)}s`;
}

function playClip(c: ClipRef){
  const v = $("#playerVid");
  v.src = "/clip/" + encodeURIComponent(c.name);
  $("#playerName").textContent = `${c.name}  ${c.w}x${c.h}  ${c.duration}s`;
  trim = {clip: c, in: 0, out: null};
  $("#btnPlayTrim").hidden = true;
  $("#btnPlayTrim").textContent = "Play the cut";
  $("#trimOut").textContent = "";
  paintTrim();
  $("#player").hidden = false;
  v.play().catch(() => {});
}
$("#player").addEventListener("click", (e: MouseEvent) => {
  // Anything inside the bar is a control, not a click-away to dismiss.
  const tgt = e.target as HTMLElement | null;
    if (!tgt || tgt.id === "playerVid" || tgt.closest("#playerBar")) return;
  $("#playerVid").pause(); $("#player").hidden = true;
});

$("#btnIn").addEventListener("click", () => {
  trim.in = $("#playerVid").currentTime || 0;
  if (trim.out != null && trim.out <= trim.in) trim.out = null;
  paintTrim();
});
$("#btnOut").addEventListener("click", () => {
  const t = $("#playerVid").currentTime || 0;
  if (t > trim.in) trim.out = t;
  paintTrim();
});
$("#btnTrimClear").addEventListener("click", () => {
  trim.in = 0; trim.out = null; paintTrim();
});

/* A YouTube thumbnail usually wants a frame out of the video. The still lands
   in the session as an ordinary shot, so it grades and exports through the same
   presets as anything off the camera. */
/* The good audio sits beside the video and syncing it by hand is a step every
   single time. A clap gives both tracks the same transient to line up on. */
$("#btnSyncAudio").addEventListener("click", async () => {
  if (!trim.clip) return;
  const takes = state.takes || [];
  if (!takes.length) {
    return flash($("#exportOut"), "no audio takes in this session yet", true);
  }
  // the take recorded closest in time to this clip is the one that belongs
  // A cut just made has no recording time; the newest take stands in, which
  // is what the untyped comparison against `undefined` produced in practice.
  const clipAt = trim.clip ? trim.clip.at : undefined;
  const t = clipAt === undefined ? takes[0] : takes.slice().sort((a, b) =>
    Math.abs(a.at - clipAt) - Math.abs(b.at - clipAt))[0];
  const btn = $("#btnSyncAudio"), out = $("#trimOut");
  btn.disabled = true; btn.textContent = "Finding the clap…";
  const r = await api("/api/sync-audio", {clip: trim.clip.name, take: t.name});
  if (!r.ok) {
    btn.disabled = false; btn.textContent = "Use recorded audio";
    out.textContent = r.error || ""; out.style.color = "var(--tally)"; return;
  }
  out.style.color = "var(--dim)";
  out.textContent = `clap at ${r.camera_clap}s in the clip, ${r.take_clap}s in `
    + `${t.name}, offset ${r.offset > 0 ? "+" : ""}${r.offset}s`;
  btn.textContent = "Muxing…";
  while (true) {
    await new Promise(z => setTimeout(z, 500));
    const j = await fetch(`/api/job?id=${encodeURIComponent(r.job)}`)
      .then(x => x.json()).catch(() => null);
    if (!j || !j.ok) { out.textContent = (j && j.error) || "lost the sync job";
                       out.style.color = "var(--tally)"; break; }
    if (j.state === "running") continue;
    if (j.errors && j.errors.length) {
      out.textContent = j.errors[0].error; out.style.color = "var(--tally)";
    } else {
      const f = (j.results || [])[0] || {};
      out.textContent = `saved ${f.name}  offset ${f.offset}s  `
        + `${((f.size || 0) / 1048576).toFixed(1)} MB`;
      out.style.color = "var(--ok)";
      // Offer it straight away. A synced file you cannot watch is the
      // un-playable trim all over again, and checking the sync is the whole
      // point of making it.
      lastTrim = {name: f.name, duration: trim.clip.duration,
                  w: trim.clip.w, h: trim.clip.h};
      $("#btnPlayTrim").hidden = false;
      $("#btnPlayTrim").textContent = "Play the synced take";
    }
    break;
  }
  btn.disabled = false; btn.textContent = "Use recorded audio";
  refresh();
});

$("#btnPlayTrim").addEventListener("click", () => {
  if (!lastTrim) return;
  playClip(lastTrim);
  $("#btnPlayTrim").hidden = true;
});

$("#btnGrab").addEventListener("click", async () => {
  if (!trim.clip) return;
  const btn = $("#btnGrab"), out = $("#trimOut");
  const at = $("#playerVid").currentTime || 0;
  btn.disabled = true; btn.textContent = "Grabbing…";
  const r = await api("/api/grab", {name: trim.clip.name, at});
  btn.disabled = false; btn.textContent = "Grab frame";
  if (!r.ok) { out.textContent = r.error || ""; out.style.color = "var(--tally)"; return; }
  out.textContent = `saved ${r.name}  ${r.dims[0]}×${r.dims[1]} at ${r.at}s, `
    + `now in the strip`;
  out.style.color = "var(--ok)";
  await refresh();
  select(r.name);
});

$("#btnTrimGo").addEventListener("click", async () => {
  if (!trim.clip) return;
  const btn = $("#btnTrimGo");
  btn.disabled = true; btn.textContent = "Cutting…";
  const out = $("#trimOut");
  const r = await api("/api/trim", {
    name: trim.clip.name, start: trim.in,
    end: trim.out == null ? trim.clip.duration : trim.out,
    mode: $("#trimFast").checked ? "copy" : "precise"});
  if (!r.ok) { out.textContent = r.error || ""; out.style.color = "var(--tally)";
               paintTrim(); return; }
  while (true) {
    await new Promise(z => setTimeout(z, 500));
    const j = await fetch(`/api/job?id=${encodeURIComponent(r.job)}`)
      .then(x => x.json()).catch(() => null);
    if (!j || !j.ok) {
      out.textContent = (j && j.error) || "lost contact with the cut";
      out.style.color = "var(--tally)";
      break;
    }
    if (j.state === "running") {
      out.textContent = "cutting… " + (j.elapsed || "");
      out.style.color = "var(--dim)";
      continue;
    }
    if (j.errors && j.errors.length) {
      out.textContent = j.errors[0].error; out.style.color = "var(--tally)";
    } else {
      const f = (j.results || [])[0] || {};
      out.textContent = `saved ${f.name}  ${f.seconds}s  `
        + `${((f.size || 0) / 1048576).toFixed(1)} MB  in ${j.elapsed}s`;
      out.style.color = "var(--ok, #7fd18b)";
      // Checking the cut before sending an audition tape is the point of
      // making it, so offer it right here rather than in a file manager.
      lastTrim = {name: f.name, duration: f.seconds, w: trim.clip.w,
                  h: trim.clip.h};
      $("#btnPlayTrim").hidden = false;
      $("#btnPlayTrim").textContent = "Play the cut";
    }
    break;
  }
  paintTrim();
  refresh();
});

/* ---- undo ----------------------------------------------------------------
   A grade is a sequence of guesses; without a way back you stop experimenting.
   History is per frame, and switching frames starts a fresh stack. */
let hist: string[] = [], histAt = -1, histFor: string | null = null;

function snapshot(){
  if (!state.sel) return;
  if (histFor !== state.sel) { hist = []; histAt = -1; histFor = state.sel; }
  const st = JSON.stringify(currentEdits());
  if (histAt >= 0 && hist[histAt] === st) return;
  hist = hist.slice(0, histAt + 1);
  hist.push(st);
  if (hist.length > 60) hist.shift();
  histAt = hist.length - 1;
}

function restore(i: number){
  if (i < 0 || i >= hist.length) return;
  histAt = i;
  const e = JSON.parse(hist[i]);
  SLIDERS.forEach(k => { knob(k).value = e.adjust[k] ?? NEUTRAL[k]; syncOut(k); });
  crop = e.crop;
  applyTransform(); drawCrop(); refreshPreview(false); drawHistogram();
  if (state.sel) api("/api/edits", {name: state.sel, adjust: e.adjust, crop: e.crop});
  flash($("#exportOut"), `step ${histAt + 1} of ${hist.length}`);
}
function undo(){ if (histAt > 0) restore(histAt - 1); }
function redo(){ if (histAt < hist.length - 1) restore(histAt + 1); }


/* ---- camera card ---- */
interface CardFile { folder: string; name: string; have: boolean }
let cardFiles: CardFile[] = [];
/* While the Zoom feed is up the app cannot touch a camera setting, because the
   webcam pipeline holds the PTP session. This does the handover: tether, meter,
   hand back. About six seconds of feed interruption, which beats the only
   alternative of reaching for the camera's dials mid-call. */
/* A self-tape with no sound is a wasted take. Diagnosing a dead mic by hand
   took half a dozen shell probes, so the app answers it in one click. */
/* A recorded take was INVISIBLE in the app: shots() lists images and clips()
   lists video, so a .wav appeared nowhere and the only way to hear it was a
   file manager. Same trap as the trim you could not play back. */
function renderTakes(takes: Take[]){
  const box = $("#takes");
  // Rebuild only when the list changed. Polling used to rebuild it every few
  // seconds, which deleted a take's player while it was being listened to.
  const sig = takes.map(t => t.name + ":" + t.seconds).join(",");
  if (box.dataset.sig === sig) return;
  box.dataset.sig = sig;
  if (!takes.length) { box.innerHTML = ""; return; }
  box.innerHTML = `<div class="tk-h">${takes.length} audio take${takes.length===1?"":"s"}</div>`;
  takes.forEach(t => {
    const row = document.createElement("div");
    row.className = "tk-row";
    const secs = t.seconds == null ? "?" : t.seconds.toFixed(1);
    row.innerHTML =
      `<span class="tk-name" title="${esc(t.name)}">${esc(t.name)}</span>`
      + `<span class="tk-len">${secs}s`
      + ` <button class="tk-del" data-n="${esc(t.name)}" title="Move this take to the trash, recoverable">x</button></span>`;
    // A player PER TAKE costs about 48px each, and a session accumulates
    // accidental one-second recordings: four takes, three of them 0.6s, ate
    // 200px of the sidebar and pushed the histogram and the crop below the
    // fold. Build the player only when a take is actually played.
    const play = document.createElement("button");
    play.className = "tk-play";
    play.textContent = "Play";
    play.title = "Listen to this take";
    play.addEventListener("click", () => {
      const playing = row.querySelector("audio");
      if (playing) {
        playing.remove();
        play.textContent = "Play";
        return;
      }
      // One at a time: two takes playing over each other tells you nothing.
      box.querySelectorAll("audio").forEach((a: HTMLAudioElement) => a.remove());
      box.querySelectorAll<HTMLElement>(".tk-play").forEach(b => b.textContent = "Play");
      const au = document.createElement("audio");
      au.controls = true; au.autoplay = true; au.preload = "metadata";
      au.src = "/take/" + encodeURIComponent(t.name);
      row.appendChild(au);
      play.textContent = "Stop";
    });
    row.querySelector(".tk-len")?.insertBefore(
      play, row.querySelector(".tk-del"));
    box.appendChild(row);
  });
  // Takes are ~9 MB a minute, so there has to be a way to clear them.
  box.querySelectorAll<HTMLButtonElement>(".tk-del").forEach(btn =>
    btn.addEventListener("click", async () => {
      const n = btn.dataset.n;
      btn.disabled = true;
      const r = await api("/api/delete", {name: n});
      if (!r.ok) { btn.disabled = false;
                   return flash($("#exportOut"), r.error, true); }
      flash($("#exportOut"), `${n} moved to the trash, press Undo delete to get it back`);
      lastDeleted = r.removed;
      $("#btnUndelete").hidden = false;
      refresh();
    }));
}

/* An in-camera take gets the camera's built-in microphone, the weakest link in
   the chain. Record the good one in parallel and sync in the edit. */
let audioRec = false;
$("#btnRecAudio").addEventListener("click", async () => {
  const btn = $("#btnRecAudio");
  btn.disabled = true;
  toggleEpoch++;
  const r = await api("/api/audio-record", {on: !audioRec});
  toggleEpoch++;
  btn.disabled = false;
  if (!r.ok && !audioRec) {
    return flash($("#exportOut"), r.error || "could not start recording", true);
  }
  audioRec = !!r.recording;
  btn.textContent = audioRec ? "Stop audio" : "Record audio";
  btn.classList.toggle("busy", audioRec);
  if (audioRec) {
    flash($("#exportOut"), `recording audio to ${r.name}\nclap once so you have a sync point`);
  } else {
    flash($("#exportOut"), r.ok
      ? `saved ${r.name}  ${r.seconds}s  ${((r.size||0)/1048576).toFixed(1)} MB`
      : (r.error || "the recording was empty"), !r.ok);
    refresh();
  }
});

$("#btnAudio").addEventListener("click", async () => {
  const btn = $("#btnAudio");
  btn.disabled = true; btn.textContent = "Listening…";
  const a = await api("/api/audio");
  btn.disabled = false; btn.textContent = "Check audio";
  const dev = (a.source || "unknown").replace(/^alsa_input\./, "")
                .replace(/\s*\[Audio\/Source\]$/, "");
  const lines = [dev];
  if (a.signal === true) {
    lines.push(`signal present, peak ${a.level}%`
      + ((a.level ?? 0) > 85 ? "  TOO HOT, turn the gain down" : ""));
  } else if (a.signal === false) {
    lines.push("NO SOUND reaching the computer");
  } else {
    lines.push("could not measure: " + (a.why || "input busy"));
  }
  if (a.muted) lines.push("muted in the system mixer");
  if (a.phantom === false) lines.push("phantom power is off");
  (a.hints || []).forEach((h: string) => lines.push(h));
  flash($("#exportOut"), lines.join("\n"),
        a.signal === false || !!a.muted || (a.level ?? 0) > 85);
});

$("#btnFixExp").addEventListener("click", async () => {
  const btn = $("#btnFixExp");
  const streaming = !!state.webcam;
  btn.disabled = true;
  btn.textContent = streaming ? "Handing over…" : "Metering…";
  const r = await api("/api/exposure-fix", {look: adj()});
  btn.disabled = false; btn.textContent = "Fix exposure";
  if (!r.ok) return flash($("#exportOut"), r.error || "could not meter", true);
  const b = r.before || {}, a = r.after || {};
  flash($("#exportOut"),
    `metered: ${b.expprogram} ${b.aperture} ${b.shutter}`
    + `  ->  ${a.expprogram} ${a.aperture} ${a.shutter}`
    + (streaming ? "\nfeed handed back" : ""));
  loadSettings();
});

// Exports of frames that have since been deleted pile up silently. This was
// cleaned by hand twice in one evening before it became a button.
$("#btnTidy").addEventListener("click", async () => {
  const b = $("#btnTidy");
  b.disabled = true;
  try {
    // A DRY RUN first, always. It decides on a filename, so the list is shown
    // before anything moves.
    const look = await api("/api/tidy-exports", {});
    if (!("orphans" in look)) return flash($("#exportOut"), "the dry run did not come back", true);
    const n = look.orphans.length;
    if (!n) return flash($("#exportOut"),
      `nothing to tidy: ${look.kept} exports, all of frames you still have`);
    const mb = Math.round(look.bytes / 1e6);
    if (!await askConfirm(`Move ${n} export${n === 1 ? "" : "s"} (${mb} MB) to the trash?`,
        `These are exports of frames that are no longer in the session.`
        + `\nThey go to the trash, so Undo delete brings them back.`
        + `\n\n` + look.orphans.slice(0, 8).map(o => o.name).join("\n")
        + (n > 8 ? `\n... and ${n - 8} more` : ""), "Move to trash"))
      return;
    const did = await api("/api/tidy-exports", {confirm: true});
    if (!("moved" in did)) return flash($("#exportOut"), "tidying did not come back", true);
    if (!did.ok) return flash($("#exportOut"),
      `moved ${did.moved.length}, ${did.failed.length} failed: `
      + (did.failed[0] ? did.failed[0].error : ""), true);
    flash($("#exportOut"),
      `${did.moved.length} old exports moved to the trash, recoverable`);
    refresh();
  } finally { b.disabled = false; }
});

$("#btnCardList").addEventListener("click", async () => {
  const btn = $("#btnCardList");
  btn.disabled = true; btn.textContent = "Reading…";
  const r = await api("/api/card");
  btn.disabled = false; btn.textContent = "List card";
  if (!r.ok) { $("#cardNote").textContent = r.error || ""; $("#cardNote").style.color = "var(--tally)"; return; }
  cardFiles = r.files.filter((f: CardFile) => !f.have);
  // A partial card shown as if it were the whole card is the trap here.
  $("#cardNote").style.color = (r.truncated || r.warning)
    ? "var(--amber)" : "var(--mute)";
  $("#cardNote").textContent =
    `${r.files.length}${r.truncated ? "+" : ""} on the card, `
    + `${cardFiles.length} not yet imported`
    + (r.truncated ? "  (stopped early, this is not the whole card)" : "")
    + (r.warning ? `  (${r.warning})` : "");
  $("#btnCardImport").disabled = !cardFiles.length;
});
/* Importing ten 4K takes is minutes of USB, so it runs as a job: a live count
   and a cancel, like the batch export. */
let importJob: string | null = null;

$("#btnCardImport").addEventListener("click", async () => {
  const btn = $("#btnCardImport");
  if (importJob) {                       // second click cancels
    await api("/api/job-cancel", {id: importJob});
    btn.textContent = "Cancelling…";
    return;
  }
  const start = await api("/api/card-import", {files: cardFiles});
  if (!start.ok) { $("#cardNote").textContent = start.error || "";
                   $("#cardNote").style.color = "var(--tally)"; return; }
  importJob = start.job;
  btn.classList.add("busy");
  btn.textContent = `0 / ${start.total}  cancel`;
  $("#cardNote").style.color = "var(--dim)";

  while (true) {
    await new Promise(z => setTimeout(z, 500));
    const j = await fetch(`/api/job?id=${encodeURIComponent(importJob || "")}`)
      .then(x => x.json()).catch(() => null);
    if (!j || !j.ok) {
      $("#cardNote").textContent = (j && j.error) || "lost contact with the import";
      $("#cardNote").style.color = "var(--tally)";
      break;
    }
    if (j.state === "running") {
      btn.textContent = `${j.done} / ${j.total}  cancel`;
      $("#cardNote").textContent = `importing ${j.done} of ${j.total}…`;
      continue;
    }
    const n = (j.results || []).length, e = (j.errors || []).length;
    $("#cardNote").textContent =
      `${j.state === "cancelled" ? "stopped after " : "imported "}${n}`
      + (e ? `, ${e} failed: ${j.errors.map((x: {error: string}) => x.error).join("; ")}` : "");
    $("#cardNote").style.color = e ? "var(--tally)" : "var(--mute)";
    break;
  }
  importJob = null;
  btn.classList.remove("busy");
  btn.textContent = "Import new";
  cardFiles = []; await refresh();
});

/* ---- notice when the server goes away --------------------------------- */
// This poll is also what keeps the page LIVE. Before it, /api/state was read
// at boot and after a button, and this timer fetched it and threw the answer
// away, so a camera that auto-connected, a next-step that advanced, or a frame
// from another tab never reached the screen without a reload.
let offline = false, pollController: AbortController | null = null;
function showOffline(){
  offline = true;
  document.title = "Z8 Studio (offline)";
  $("#connTxt").textContent = "Offline";
  $("#modeTxt").textContent = "Reconnecting to the computer…";
  $("#live").removeAttribute("src");
  (["#btnShoot", "#btnAf", "#btnRec"] as const).forEach(id => $(id).disabled = true);
  flash($("#exportOut"), "Connection lost. Check Wi-Fi and Tailscale, and keep the computer awake. Reconnecting…", true);
}
async function pollState(){
  if (pollController || document.hidden) return;
  const controller = new AbortController();
  pollController = controller;
  const timer = setTimeout(() => controller.abort(), 8000);
  try {
    if (!booted) await boot();
    else await refresh(controller.signal);
    if (pollController !== controller || controller.signal.aborted) return;
    if (offline) {
      offline = false; document.title = "Z8 Studio";
      flash($("#exportOut"), "Reconnected to the computer.");
    }
    if (pendingEdits.size) void flushEditSaves();
  } catch (e) {
    if (pollController === controller && !document.hidden && !offline) showOffline();
  } finally {
    clearTimeout(timer);
    if (pollController === controller) pollController = null;
  }
}
function suspendView(){
  const controller = pollController;
  pollController = null;
  controller?.abort();
  $("#live").removeAttribute("src");
  delete $("#camSettings").dataset.dragging;
  uncmp();
  void flushEditSaves();
}
function resumeView(){
  if (document.hidden) return;
  suspendView();
  void pollState();
}
document.addEventListener("visibilitychange", () => {
  if (document.hidden) suspendView(); else resumeView();
});
window.addEventListener("pagehide", suspendView);
window.addEventListener("pageshow", e => { if (e.persisted) resumeView(); });
window.addEventListener("online", resumeView);
$("#live").addEventListener("error", () => $("#live").removeAttribute("src"));
setInterval(() => { void pollState(); }, 3000);

/* The live view froze on the phone until a reload: an MJPEG stream that
   stalls (a Tailscale or Wi-Fi blip) or ends (the server closes it after
   10 s without a camera) fires no event, so the <img> kept its last frame
   for good. Every 3 s, sample the picture small; a live sensor never gives
   the same pixels twice, so the same picture 6 s running means the stream
   is dead, and it is opened again. */
let liveCanvas: HTMLCanvasElement | null = null, liveSig = "", liveSame = 0;
function liveSample(img: HTMLImageElement): string {
  const c = liveCanvas ??= document.createElement("canvas");
  c.width = 32; c.height = 18;
  const x = c.getContext("2d", {willReadFrequently: true});
  if (!x || !img.naturalWidth) return "blank";
  x.drawImage(img, 0, 0, 32, 18);
  const d = x.getImageData(0, 0, 32, 18).data;
  let h = 2166136261;
  for (let i = 0; i < d.length; i++) h = Math.imul(h ^ d[i], 16777619);
  return String(h >>> 0);
}
function liveWatchdog(){
  const live = $("#live");
  if (document.hidden || live.hidden || !live.src.includes("preview.mjpg")) {
    liveSig = ""; liveSame = 0; return;
  }
  // A picture that cannot be drawn is the iPhone's broken-image state after
  // a frame arrived cut short: that is a dead stream too, not a skip (it
  // was, and the phone showed the broken icon until a reload).
  let sig: string;
  try { sig = liveSample(live); } catch (_) { sig = "broken"; }
  if (sig === liveSig) {
    if (++liveSame >= 2) {
      live.src = "/preview.mjpg?t=" + Date.now();
      liveSig = ""; liveSame = 0;
    }
  } else { liveSig = sig; liveSame = 0; }
}
setInterval(liveWatchdog, 3000);

/* ---- keyboard ---- */
document.addEventListener("keydown", (e: KeyboardEvent) => {
  const tgt = e.target as HTMLElement | null;
  if (tgt && /^(INPUT|SELECT|TEXTAREA)$/.test(tgt.tagName)) return;
  if ($("#dlg").open) return;
  const k = e.key.toLowerCase();
  // With the player open, space means play/pause. It used to fire the
  // SHUTTER behind the overlay while he was reviewing a take.
  if (!$("#player").hidden) {
    if (e.key === " ") {
      e.preventDefault();
      const v = $("#playerVid");
      if (v.paused) v.play().catch(() => {}); else v.pause();
    } else if (k === "i") $("#btnIn").click();
    else if (k === "o") $("#btnOut").click();
    return;
  }
  if (!$("#loupe").hidden) return;
  if (!$("#keys").hidden && k !== "?" && k !== "/") return;
  if (e.altKey) return;
  if ((e.ctrlKey || e.metaKey) && k === "z") {
    e.preventDefault(); return e.shiftKey ? redo() : undo();
  }
  if ((e.ctrlKey || e.metaKey) && k === "y") { e.preventDefault(); return redo(); }
  if (e.key === " ")      { e.preventDefault(); $("#btnShoot").click(); }
  else if (k === "f")     $("#btnAf").click();
  else if (k === "r")     $("#btnRot").click();
  else if (k === "m")     $("#btnMir").click();
  else if (k === "g")     $("#btnGrid").click();
  else if (k === "l")     setReview(!state.review);
  else if (k === "e")     { if (!$("#btnExport").disabled) $("#btnExport").click(); }
  else if (k === "p")     $("#btnPick").click();
  else if (k === "x")     $("#btnReject").click();
  else if (k === "u")     $("#btnClearFlag").click();
  else if (k === "?" || (k === "/" && e.shiftKey)) { e.preventDefault(); toggleKeys(); }
  else if (k === "i" && !$("#player").hidden) $("#btnIn").click();
  else if (k === "o" && !$("#player").hidden) $("#btnOut").click();
  else if (k === "c")     $("#btnCopyEdits").click();
  else if (k === "v")     $("#btnPasteEdits").click();
  else if (e.key === "ArrowLeft" || e.key === "ArrowRight") {
    const list = shownShots();
    if (!list.length) return;
    e.preventDefault();
    const i = list.findIndex(s => s.name === state.sel);
    const n = e.key === "ArrowRight" ? i + 1 : i - 1;
    if (n >= 0 && n < list.length) { select(list[n].name); setReview(true); }
  }
});


/* ---- in-page E2E ----------------------------------------------------------
   Opened with ?e2e=1 this drives the real controls and writes verdicts into
   #e2e, which headless Chrome then reads. Boot-only checks cannot see a button
   that is wired to the wrong handler or a slider that never repaints. */
if (new URLSearchParams(location.search).has("e2e")) {
  const out: string[] = [];
  const ok = (name: string, cond: unknown, detail?: unknown) =>
    out.push(`${cond ? "PASS" : "FAIL"}  ${name}${detail ? "   " + detail : ""}`);
  const wait = (ms: number) => new Promise(r => setTimeout(r, ms));

  // DOMContentLoaded, never load. `load` waits on every pending image, and the
  // preview stream is an MJPEG response that never ends, so with a camera
  // attached `load` never fires at all and the harness silently produced no
  // output. HEADLESS suppresses the stream, but a card thumbnail or a slow
  // export preview can hold `load` open just as well. Diagnosed 2026-09-21:
  // "e2e harness ran   no output, the page may not have booted", with a page
  // that had in fact booted perfectly.
  window.addEventListener("DOMContentLoaded", async () => {
    await wait(1800);                       // let boot finish
    try {
      // Every interaction below needs a frame to act on. Throwing here
      // reported "harness threw   no frames to drive", which reads as a broken
      // harness when it is an empty session: a legitimate state on the first
      // day of a shoot. A skip the suite can recognise, and no teardown work
      // that would touch the camera for nothing.
      if (!state.sel) {
        out.push("SKIP  no frames in this session");
        throw {skipped: true};
      }
      ok("a frame is auto-selected", !!state.sel, state.sel || "none");

      // sliders drive a real re-render
      const before = $("#cropImg").src;
      $("#exposure").value = "1.35";
      $("#exposure").dispatchEvent(new Event("input"));
      ok("slider updates its readout", $("#oExposure").textContent === "1.35",
         $("#oExposure").textContent);
      await wait(900);
      ok("slider triggers a new render", $("#cropImg").src !== before);

      // a look preset moves every knob and resets the unmentioned ones
      $("#lookPreset").value = "moody";
      $("#lookPreset").dispatchEvent(new Event("change"));
      await wait(300);
      ok("a look preset applies its values",
         Math.abs(parseFloat($("#contrast").value) - 1.35) < 0.001,
         "contrast " + $("#contrast").value);
      ok("a look preset resets knobs it does not mention",
         Math.abs(parseFloat($("#exposure").value) - 0.94) < 0.001,
         "exposure " + $("#exposure").value);

      // rotation is applied to the live element
      // Rotation is GLOBAL SERVER state that outlives the page, so restore it
      // BEFORE asserting, not after: an assertion that throws in between used
      // to leave the server on 270deg, which rotates every frame the user sees
      // and made the next run's pixel-exact crop check fail hours later.
      const r0 = state.rotation;
      // Take a LEASE before touching global orientation. This page can be
      // killed between the rotate and the rotate-back, and it twice was, so
      // the server has to be able to undo this without the page's help.
      await api("/api/orient", {rotation: r0, mirror: state.mirror,
                                revert_after: 45});
      $("#btnRot").click();
      await wait(250);
      const rotated = state.rotation;
      // The live view turns by Rotate PLUS the body's own turn (liveTurn).
      const rotOnEl = $("#live").style.transform.includes(
        "rotate(" + ((state.rotation + (state.liveTurn || 0)) % 360) + "deg)");
      $("#btnRot").click(); $("#btnRot").click(); $("#btnRot").click();
      await wait(250);
      ok("rotate advances 90 degrees", rotated === (r0 + 90) % 360,
         rotated + "deg");
      ok("rotation reaches the element", rotOnEl);
      ok("and rotation is put back where it was",
         state.rotation === r0, `${r0} -> ${state.rotation}`);

      // grid cycles
      $("#btnGrid").click(); await wait(80);
      ok("grid cycles on", document.body.dataset.grid === "thirds",
         document.body.dataset.grid);
      $("#btnGrid").click(); $("#btnGrid").click();

      // Culling writes through and marks the cell. Assert on the SELECTED
      // frame, not on the whole session: the buttons act on one shot, and an
      // earlier concurrency check leaves other frames legitimately flagged.
      // Pick ADVANCES to the next frame on purpose, so follow the frame that
      // was judged, not whatever is selected afterwards. Asserting on the
      // session as a whole passed for the wrong reason: another test leaves a
      // frame flagged, so the check went green without the click working.
      const flagOf = (n: string) => (state.shots.find(x => x.name === n) || {pick: null}).pick;
      const judged = state.sel;
      $("#btnPick").click();
      await wait(700);
      ok("pick is recorded on the frame that was judged",
         flagOf(judged) === "pick", String(flagOf(judged)));
      // Only when there IS a next frame. On the last shot of a session it
      // correctly stays put rather than wrapping round to the top.
      ok("pick advances to the next frame",
         state.shots.length < 2 ? true : state.sel !== judged,
         state.shots.length < 2 ? "single frame, nothing to advance to" : state.sel);
      select(judged); await wait(300);
      $("#btnClearFlag").click();
      await wait(700);
      ok("clearing a flag works", !flagOf(judged), String(flagOf(judged)));

      // crop refits when the export aspect changes
      $("#exportPreset").value = "ig-square";
      $("#exportPreset").dispatchEvent(new Event("change"));
      const img = $("#cropImg");
      for (let i = 0; i < 40 && !img.naturalWidth; i++) await wait(100);
      await wait(400);
      const sq = crop ? crop.w / crop.h : 0;
      const srcAspect = img.naturalWidth / img.naturalHeight;
      ok("crop refits to a square preset",
         Math.abs(sq * srcAspect - 1) < 0.05,
         "effective aspect " + (sq * srcAspect).toFixed(3));

      // undo walks back
      const expNow = $("#exposure").value;
      $("#exposure").value = "1.7";
      $("#exposure").dispatchEvent(new Event("input"));
      await wait(900);
      undo();
      await wait(700);
      ok("undo restores the previous grade", $("#exposure").value !== "1.7",
         "exposure " + $("#exposure").value);

      // loupe opens and closes
      openLoupe(0.5, 0.5);
      await wait(600);
      ok("loupe opens", $("#loupe").hidden === false);
      ok("loupe loaded a patch", ($("#loupeImg").naturalWidth || 0) > 100,
         ($("#loupeImg").naturalWidth || 0) + "px");
      $("#loupe").click();
      ok("loupe closes", $("#loupe").hidden === true);

      // a real export through the button
      $("#exportPreset").value = "yt-thumb";
      $("#exportPreset").dispatchEvent(new Event("change"));
      await wait(200);
      $("#btnExport").click();
      await wait(3500);
      ok("export button produces a result",
         /\.jpg/.test($("#exportOut").textContent),
         $("#exportOut").textContent.split("\n")[0].slice(0, 60));

      // The trim controls are real buttons on a real clip. A player that opens
      // proves nothing: the in/out points are what get sent to ffmpeg.
      if (state.clips && state.clips.length) {
        playClip(state.clips[0]);
        await wait(500);
        ok("player opens on a clip", $("#player").hidden === false);
        const v = $("#playerVid");
        v.currentTime = 1.0; await wait(400);
        $("#btnIn").click();
        v.currentTime = 2.5; await wait(400);
        $("#btnOut").click();
        await wait(150);
        ok("in and out points are captured",
           trim.in > 0.5 && trim.out != null && trim.out > trim.in,
           `in ${trim.in.toFixed(2)} out ${(trim.out || 0).toFixed(2)}`);
        ok("save is enabled once the range is valid",
           $("#btnTrimGo").disabled === false, $("#btnTrimGo").textContent);
        $("#btnTrimClear").click();
        await wait(150);
        ok("reset clears the range back to the whole clip",
           trim.in === 0 && trim.out === null);
        // Clicking the bar must NOT dismiss the player, or every in point
        // closes the thing you are trimming.
        $("#playerBar").click();
        await wait(150);
        ok("clicking the controls does not close the player",
           $("#player").hidden === false);
        $("#player").click();
        await wait(150);
        ok("clicking away closes the player", $("#player").hidden === true);
      }

      // The audio controls, driven for real. They work with the camera off,
      // which is most of the time, and none of them had browser coverage.
      $("#btnAudio").click();
      for (let i = 0; i < 80 && $("#btnAudio").textContent !== "Check audio"; i++)
        await wait(250);
      ok("check audio reports something",
         ($("#exportOut").textContent || "").length > 10,
         ($("#exportOut").textContent || "").split("\n")[0].slice(0, 46));

      const takesBefore = (state.takes || []).length;
      $("#btnRecAudio").click();
      for (let i = 0; i < 60 && $("#btnRecAudio").textContent === "Record audio"; i++)
        await wait(250);
      const started = $("#btnRecAudio").textContent === "Stop audio";
      ok("record audio starts and the button becomes Stop", started,
         $("#btnRecAudio").textContent);
      if (started) {
        await wait(2200);
        $("#btnRecAudio").click();
        for (let i = 0; i < 80 && $("#btnRecAudio").textContent === "Stop audio"; i++)
          await wait(250);
        ok("it stops again", $("#btnRecAudio").textContent === "Record audio",
           $("#btnRecAudio").textContent);
        for (let i = 0; i < 40 && (state.takes || []).length <= takesBefore; i++)
          await wait(250);
        ok("the take appears in the session",
           (state.takes || []).length > takesBefore,
           `${takesBefore} -> ${(state.takes || []).length}`);
        // A player is built ON DEMAND now, not one per take. Four takes with
        // an eager player each ate 200px of the sidebar and pushed the
        // histogram below the fold. So check the row and its Play button, and
        // then that pressing it really does produce an audio element.
        const pb = $("#takes").querySelector<HTMLElement>(".tk-play");
        ok("and it is listed with a Play button", !!pb,
           $("#takes").querySelectorAll(".tk-row").length + " rows");
        ok("no take builds a player before it is asked for",
           !$("#takes").querySelector("audio"),
           "one per take cost 200px of sidebar");
        if (pb) {
          pb.click(); await wait(250);
          ok("pressing Play builds the player",
             !!$("#takes").querySelector("audio"), pb.textContent);
          pb.click(); await wait(150);
          ok("and pressing it again takes the player away",
             !$("#takes").querySelector("audio"), pb.textContent);
        } else {
          ok("pressing Play builds the player", false, "no button");
          ok("and pressing it again takes the player away", false, "no button");
        }
      } else {
        ok("it stops again", false, "never started");
        ok("the take appears in the session", false, "never started");
        ok("and it is listed with a Play button", false, "never started");
        ok("no take builds a player before it is asked for", false, "never started");
        ok("pressing Play builds the player", false, "never started");
        ok("and pressing it again takes the player away", false, "never started");
      }

      // the keyboard overlay, built from the live bindings
      toggleKeys();
      await wait(200);
      ok("keyboard help opens", $("#keys").hidden === false);
      const krows = $("#keysTbl").querySelectorAll("tr").length;
      ok("it lists the real bindings", krows >= 10, krows + " rows");
      ok("it names a binding that exists",
         /shoot/i.test($("#keysTbl").textContent));
      $("#keys").click();
      await wait(150);
      ok("keyboard help closes", $("#keys").hidden === true);

      // The newest controls, in a real browser. The whole UI was once dead
      // while 62 API checks passed, so every new button gets driven here.
      ok("the missing-tool banner is hidden when nothing is missing",
         $("#toolWarn").hidden === true || state.toolsMissing.length > 0);

      // Delete and undo, on a frame the harness made for itself. It never
      // touches a real shot: no victim, no test.
      const victim = (state.shots.find(x =>
        x.name.startsWith("selftest-e2e-victim")) || {}).name;
      ok("a throwaway frame is available for the delete test", !!victim,
         victim || "none, delete interactions will report themselves below");
      if (victim) {
        select(victim);
        await wait(300);
        $("#btnDelete").click();
        await wait(900);
        ok("delete removes it from the strip",
           !state.shots.some(x => x.name === victim));
        ok("and the trash count rises", (state.trash || 0) >= 1, state.trash);
        ok("undo delete becomes available", $("#btnUndelete").hidden === false);

        // Empty trash must ARM before it destroys anything, and this has to be
        // checked WHILE the trash has something in it. Testing it after the
        // undo passed for the wrong reason: the button correctly refuses to
        // arm on an empty trash, so it never showed the confirmation at all.
        $("#btnEmptyTrash").click();
        await wait(250);
        ok("empty trash asks before it destroys anything",
           /for good/.test($("#btnEmptyTrash").textContent),
           $("#btnEmptyTrash").textContent);
        delete $("#btnEmptyTrash").dataset.armed;
        $("#btnEmptyTrash").textContent = "Empty trash";

        $("#btnUndelete").click();
        await wait(900);
        ok("undo delete brings it back",
           state.shots.some(x => x.name === victim));
        ok("an empty trash refuses to arm at all",
           (state.trash || 0) === 0
           && !$("#btnEmptyTrash").dataset.armed, String(state.trash));
      } else {
        ok("delete removes it from the strip", false, "no victim frame");
        ok("and the trash count rises", false, "no victim frame");
        ok("undo delete becomes available", false, "no victim frame");
        ok("empty trash asks before it destroys anything", false, "no victim frame");
        ok("undo delete brings it back", false, "no victim frame");
        ok("an empty trash refuses to arm at all", false, "no victim frame");
      }

      // Look to picks: the grade must reach the picked frame and no other.
      if (victim && state.shots.length > 1) {
        const other = (state.shots.find(x => x.name !== victim) || {name: ""}).name;
        // Make the precondition real rather than assumed: an earlier
        // interaction may have left a flag on the other frame, and then
        // "unpicked frames are untouched" passes or fails for the wrong reason.
        select(other); await wait(250);
        $("#btnClearFlag").click(); await wait(600);
        select(victim); await wait(250);
        $("#btnPick").click(); await wait(700);
        // Pick ADVANCES to the next frame, so without re-selecting, the slider
        // change below autosaves onto the wrong frame and this test grades the
        // very frame it is about to assert was left alone.
        select(victim); await wait(300);
        $("#exposure").value = "1.33";
        $("#exposure").dispatchEvent(new Event("input"));
        await wait(900);
        $("#btnApplyPicks").click();
        await wait(900);
        const ed: Record<string, Edit> = state.edits || {};
        const expOf = (n: string) => (ed[n] || {adjust: {}}).adjust.exposure;
        ok("look to picks reaches the picked frame",
           Math.abs((expOf(victim) ?? NaN) - 1.33) < 0.01,
           String(expOf(victim)));
        const otherExp = expOf(other);
        ok("and leaves an unpicked frame alone",
           otherExp === undefined || Math.abs(otherExp - 1.33) >= 0.01,
           `${other} exposure ${otherExp}`);
        select(victim); await wait(250);
        $("#btnClearFlag").click(); await wait(600);
      } else {
        ok("look to picks reaches the picked frame", false, "needs two frames");
        ok("and leaves an unpicked frame alone", false, "needs two frames");
      }

      // Grab frame and Play the cut, driven in the player on a real clip.
      if (state.clips && state.clips.length) {
        playClip(state.clips[0]);
        await wait(500);
        ok("play the cut is hidden until a cut exists",
           $("#btnPlayTrim").hidden === true);
        const before = state.shots.length;
        $("#playerVid").currentTime = 1.0; await wait(400);
        $("#btnGrab").click();
        for (let i = 0; i < 60 && state.shots.length === before; i++) await wait(250);
        ok("grab frame adds a still to the strip",
           state.shots.length > before,
           `${before} -> ${state.shots.length}`);

        $("#btnIn").click();
        $("#playerVid").currentTime = 3.0; await wait(400);
        $("#btnOut").click();
        $("#trimFast").checked = true;          // keep the harness quick
        $("#btnTrimGo").click();
        for (let i = 0; i < 120 && $("#btnPlayTrim").hidden; i++) await wait(250);
        ok("play the cut appears once a cut is made",
           $("#btnPlayTrim").hidden === false,
           $("#trimOut").textContent.slice(0, 50));
        $("#btnPlayTrim").click();
        await wait(600);
        ok("and the cut actually loads in the player",
           ($("#playerVid").readyState || 0) > 0
           || $("#playerVid").src.includes("_trim"),
           $("#playerVid").src.split("/").pop());
        $("#trimFast").checked = false;
        $("#player").click(); await wait(200);
      } else {
        ok("play the cut is hidden until a cut exists", false, "no clip");
        ok("grab frame adds a still to the strip", false, "no clip");
        ok("play the cut appears once a cut is made", false, "no clip");
        ok("and the cut actually loads in the player", false, "no clip");
      }

      // Shift-click multi-select, through the real cells.
      const cells = $$("#strip .cell img");
      if (cells.length >= 3) {
        cells[0].click(); await wait(120);
        ok("a plain click marks nothing", !$("#strip .cell[data-mark]"),
           "one click is a selection of one, not a mark");
        cells[2].dispatchEvent(new MouseEvent("click",
          {shiftKey: true, bubbles: true}));
        await wait(160);
        const n = document.querySelectorAll("#strip .cell[data-mark]").length;
        ok("shift-click marks the whole run between the two", n === 3, n + " marked");
        ok("and the export button says how many",
           /3 selected/.test($("#btnExport").textContent),
           $("#btnExport").textContent);
        ok("a Clear button appears", !$("#btnClearMarks").hidden);
        cells[1].dispatchEvent(new MouseEvent("click",
          {ctrlKey: true, bubbles: true}));
        await wait(140);
        ok("ctrl-click removes one from the run",
           document.querySelectorAll("#strip .cell[data-mark]").length === 2,
           document.querySelectorAll("#strip .cell[data-mark]").length + " left");
        $("#btnClearMarks").click(); await wait(140);
        ok("Clear empties the selection",
           !document.querySelector("#strip .cell[data-mark]")
           && $("#btnClearMarks").hidden);
        cells[0].click(); await wait(120);
      } else {
        for (const t of ["a plain click marks nothing",
                         "shift-click marks the whole run between the two",
                         "and the export button says how many",
                         "a Clear button appears",
                         "ctrl-click removes one from the run",
                         "Clear empties the selection"])
          ok(t, false, "needs three frames");
      }

      // Batch export through the real button, including its live counter.
      $("#exportPreset").value = "casting-profile";
      $("#exportPreset").dispatchEvent(new Event("change"));
      await wait(200);
      // Start a real batch, watch the counter MOVE, then cancel it.
      //
      // This used to wait up to thirty seconds for the batch to finish, which
      // made the check an endurance test of the machine rather than a test of
      // the button: a session of 41 frames at 8256x5504 takes minutes, because
      // the cost is decoding each 45MP file, not the resize. It failed at
      // "8 / 43 cancel" on a real session. Completion is covered at the API
      // level; what only the browser can show is that the counter is live and
      // that Cancel actually stops it.
      const bBtn = $("#btnExportAll");
      bBtn.click();
      await wait(600);
      const firstCount = bBtn.textContent;
      const wasCounting = /\d+ \/ \d+/.test(firstCount) || bBtn.classList.contains("busy");
      ok("it showed a live count while running", wasCounting, firstCount);
      let advanced = false;
      for (let i = 0; i < 40 && !advanced; i++) {
        await wait(250);
        if (bBtn.textContent !== firstCount) advanced = true;
        if (!bBtn.classList.contains("busy")) { advanced = true; break; }
      }
      ok("the count advances, it is not a static label", advanced,
         `${firstCount} -> ${bBtn.textContent}`);
      if (bBtn.classList.contains("busy")) {
        bBtn.click();                                  // the Cancel affordance
        for (let i = 0; i < 40 && bBtn.classList.contains("busy"); i++)
          await wait(250);
      }
      ok("cancel returns the button to idle", bBtn.textContent === "Export all",
         bBtn.textContent);

      // histogram drew something
      const hc = must($("#hist").getContext("2d"), "a 2D context")
        .getImageData(0, 0, $("#hist").width, $("#hist").height).data;
      let lit = 0;
      for (let i = 0; i < hc.length; i += 4)
        if (hc[i] + hc[i+1] + hc[i+2] > 60) lit++;
      ok("histogram paints pixels", lit > 500, lit + " lit pixels");
    } catch (e) {
      if (!(e && (e as {skipped?: boolean}).skipped))
        out.push("FAIL  harness threw   " + (e instanceof Error ? e.message : String(e)));
    } finally {
      // Rotation and mirror are GLOBAL SERVER state that outlive this page.
      // Leaking them rotates every frame the user sees and breaks the next
      // run's geometry checks with no local cause, so put them back whatever
      // happened above.
      try {
        await api("/api/orient", {rotation: 0, mirror: false});
      } catch (_) { /* the server's lease covers this case too */ }
    }
    const el = $("#e2e");
    el.textContent = "E2E-BEGIN\n" + out.join("\n") + "\nE2E-END";
    el.hidden = false;
  });
}

/* boot */
let booted = false;
let booting: Promise<void> | null = null;
function boot(): Promise<void> {
  if (booted) return Promise.resolve();
  if (!booting) booting = initializePage().finally(() => { booting = null; });
  return booting;
}
async function initializePage(){
  await loadPresets();
  const st = await refresh();
  if (!st) return;
  // Orientation is server state. The page used to start at 0 degrees and say
  // "Rotate 0°" while the server rotated every frame 90, until pressed.
  if (typeof st.rotation === "number") {
    state.rotation = st.rotation;
    $("#rotTxt").innerHTML = state.rotation + "&deg;";
  }
  if (typeof st.mirror === "boolean") {
    state.mirror = st.mirror;
    $("#btnMir").setAttribute("aria-pressed", String(state.mirror));
  }
  applyTransform();
  if (st.config) {
    $("#exportDir").value = st.config.export_dir || "";
    $("#nameTpl").value = st.config.name_template || "";
    // The shutter's own behaviour, so it must read from the server on every
    // load rather than default to whatever the markup says.
    $("#afFirst").checked = st.config.autofocus_before_shoot !== false;
    const cl = $("#callLook");
    if (!selectBusy(cl) && cl.options.length
        && st.config.webcam_look_id !== undefined)
      cl.value = callLookSaved = st.config.webcam_look_id;
  }
  if (st.camPresets) {
    // The default button says which preset it restores, and is off when that
    // preset does not exist rather than failing when pressed.
    const def = st.config.camera_default_preset || "Headshot";
    const db = $("#btnCamDefault");
    if (db.textContent !== "...") {
      db.textContent = "Restore default: " + def;
      db.dataset.preset = def;
      db.disabled = !st.camPresets.includes(def);
      db.title = db.disabled ? `No saved preset named "${def}" yet: save one under that name`
                             : `Put every camera setting back to "${def}"`;
    }
    const sel = $("#camPreset");
    if (sel.options.length !== st.camPresets.length
        || [...sel.options].some((o, i) => o.value !== st.camPresets[i]))
      fillCamPresets(st.camPresets);
    $("#cfgNote").textContent = "exports go to " + st.exportDir;
  }
  paintMode(st.connected ? "tether" : (st.webcam ? "webcam" : "off"),
            st.connected, st.webcam, st);
  // A camera that comes back clears a stale failure message on its own.
  if (st.connected) delete $("#idle").dataset.sticky;
  booted = true;
}
void boot().catch(showOffline);
setInterval(() => {
  if (!document.hidden && !offline && body.dataset.conn === "1" && body.dataset.shooting === "0")
    void loadSettings().catch(() => {});
}, 15000);
// The lever moves by hand, so check it often; the server answers null at
// once rather than wait on a camera that is busy shooting.
setInterval(() => { if (body.dataset.shooting === "0") void pollLever(); }, 4000);
void pollLever();
// The Z8 only obeys a lever moved during live view once live view restarts,
// and until then reports the old side. A tap on the lamp restarts it (the
// picture blinks for a moment) and reads the lever again.
$("#leverLamp").addEventListener("click", async () => {
  if (body.dataset.conn !== "1") return;
  $("#leverTxt").textContent = "CHECKING…";
  try {
    const r = await api("/api/lever", {});
    paintLever(r.lever);
  } catch (_) { void pollLever(); }
});
})();
