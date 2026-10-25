// The YouTube recorder page (/record): screen + Z8 camera + mic, composed on
// a canvas and streamed to the server in 1 s chunks. Built to replace OBS,
// whose mic capture went silent mid-take: here a silent or ended mic is
// detected within 2 s and re-opened without stopping the recording.
//
// Wrapped in a function: tsconfig has no modules, so every file in src/
// shares one global scope with app.ts.
(() => {
  const W = 1920, H = 1080, FPS = 30;
  const VIDEO_BPS = 12_000_000, AUDIO_BPS = 192_000;

  const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;
  const out = $<HTMLCanvasElement>("out");
  // desynchronized: the preview skips the compositor's extra frame of delay.
  const ctx = out.getContext("2d", { alpha: false, desynchronized: true })!;
  const camSel = $<HTMLSelectElement>("cam"), micSel = $<HTMLSelectElement>("mic");
  const layoutSel = $<HTMLSelectElement>("layout"), cornerSel = $<HTMLSelectElement>("corner");
  const sizeSel = $<HTMLSelectElement>("pipsize");
  const goBtn = $<HTMLButtonElement>("go"), alertBox = $<HTMLDivElement>("alert");
  const meter = $<HTMLDivElement>("meter"), clock = $<HTMLSpanElement>("clock");

  // ---------------------------------------------------------- small helpers
  const store = {
    get(k: string): string | null { try { return localStorage.getItem("rec." + k); } catch { return null; } },
    set(k: string, v: string) { try { localStorage.setItem("rec." + k, v); } catch { /* private window */ } },
  };
  function status(id: string, level: "good" | "warn" | "bad" | "", text: string) {
    const el = $<HTMLDivElement>(id);
    el.className = "status " + level;
    el.innerHTML = "<b></b>";
    el.append(text);
  }
  let alertTimer = 0;
  function alarm(text: string, sticky = false) {
    alertBox.textContent = text;
    alertBox.classList.add("on");
    clearTimeout(alertTimer);
    if (!sticky) alertTimer = window.setTimeout(() => alertBox.classList.remove("on"), 8000);
  }
  async function api(path: string, body: unknown): Promise<any> {
    const r = await fetch(path, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(body) });
    return r.json().catch(() => ({ ok: false, error: `HTTP ${r.status}` }));
  }
  const stamp = () => {
    const d = new Date(), p = (n: number) => String(n).padStart(2, "0");
    return `${d.getFullYear()}${p(d.getMonth() + 1)}${p(d.getDate())}-${p(d.getHours())}${p(d.getMinutes())}${p(d.getSeconds())}`;
  };

  // ---------------------------------------------- latest frame of a track
  // MediaStreamTrackProcessor keeps delivering frames while this page is
  // hidden; a <video> element is the fallback for browsers without it.
  type Src = { draw: (x: number, y: number, w: number, h: number) => void; size: () => [number, number]; stop: () => void; live: () => boolean };
  function frameSource(track: MediaStreamTrack, onFrame?: () => void): Src {
    const P = (window as any).MediaStreamTrackProcessor;
    if (P) {
      let last: any = null, stopped = false;
      const reader = new P({ track }).readable.getReader();
      (async () => {
        while (!stopped) {
          const { value, done } = await reader.read().catch(() => ({ value: null, done: true }));
          if (done) break;
          if (last) last.close();
          last = value;
          onFrame?.();
        }
      })();
      return {
        draw: (x, y, w, h) => { if (last) ctx.drawImage(last, x, y, w, h); },
        size: () => last ? [last.displayWidth, last.displayHeight] : [0, 0],
        stop: () => { stopped = true; reader.cancel().catch(() => {}); if (last) { last.close(); last = null; } track.stop(); },
        live: () => track.readyState === "live",
      };
    }
    const v = document.createElement("video");
    v.muted = true; v.playsInline = true; v.srcObject = new MediaStream([track]); v.play().catch(() => {});
    const vf = (v as any).requestVideoFrameCallback?.bind(v);
    if (vf && onFrame) { const each = () => { onFrame(); vf(each); }; vf(each); }
    return {
      draw: (x, y, w, h) => { if (v.readyState >= 2) ctx.drawImage(v, x, y, w, h); },
      size: () => [v.videoWidth, v.videoHeight],
      stop: () => { track.stop(); v.srcObject = null; },
      live: () => track.readyState === "live",
    };
  }

  // ------------------------------------------------------------- devices
  // The system default mic (what OBS used), named exactly; Chrome's own
  // "Default" entry would follow a default that changes mid-take.
  let sysMic: RegExp | null = null;
  async function listDevices() {
    if (!sysMic) {
      const info = await fetch("/api/rec-info").then(r => r.json()).catch(() => ({}));
      if (info.micLabel) sysMic = new RegExp("^" + info.micLabel.replace(/[.*+?^${}()|[\]\\]/g, "\\$&") + "$");
    }
    const all = await navigator.mediaDevices.enumerateDevices();
    const fill = (sel: HTMLSelectElement, kind: MediaDeviceKind, prefer: RegExp, key: string) => {
      const list = all.filter(d => d.kind === kind && d.deviceId);
      const keep = sel.value || store.get(key) || "";
      sel.innerHTML = "";
      sel.append(new Option("None", ""));
      for (const d of list) sel.append(new Option(d.label || d.deviceId.slice(0, 8), d.deviceId));
      const pick = list.find(d => d.deviceId === keep) || list.find(d => prefer.test(d.label));
      sel.value = pick ? pick.deviceId : (list[0]?.deviceId ?? "");
    };
    fill(camSel, "videoinput", /nikon|z8|video10/i, "cam");
    fill(micSel, "audioinput", sysMic || /^(?!.*monitor).*scarlett/i, "mic");
  }

  // -------------------------------------------------------------- screen
  let screen: Src | null = null;
  $<HTMLButtonElement>("share").onclick = async () => {
    try {
      const s = await navigator.mediaDevices.getDisplayMedia({
        video: { width: { ideal: 1920 }, height: { ideal: 1080 }, frameRate: { ideal: FPS } } as MediaTrackConstraints,
        audio: false,
        // Chrome: offer tabs first and keep this page out of the list.
        ...({ preferCurrentTab: false, selfBrowserSurface: "exclude", surfaceSwitching: "include" } as any),
      });
      const t = s.getVideoTracks()[0];
      screen?.stop();
      screen = frameSource(t);
      const label = t.label || "shared";
      status("st-screen", "good", `Screen: ${label}`);
      // A screen was just shared, so it should be on screen.
      if (layoutSel.value === "cam") layoutSel.value = "pip";
      t.addEventListener("ended", () => {
        status("st-screen", "bad", "Screen: sharing stopped. Press Share screen.");
        if (recording) alarm("Screen sharing stopped. Press Share screen to bring it back. The recording continues.", true);
        screen = null;
      });
      updateGo();
    } catch (e) {
      status("st-screen", "warn", "Screen: not shared (" + (e as Error).name + ")");
    }
  };

  // -------------------------------------------------------------- camera
  let cam: Src | null = null, camWant = "";
  async function openCam() {
    cam?.stop(); cam = null;
    camWant = camSel.value;
    store.set("cam", camWant);
    if (!camWant) { status("st-cam", "", "Camera: off"); return; }
    try {
      const s = await navigator.mediaDevices.getUserMedia({
        video: { deviceId: { exact: camWant }, width: { ideal: 1920 }, height: { ideal: 1080 }, frameRate: { ideal: FPS } },
      });
      const t = s.getVideoTracks()[0];
      cam = frameSource(t, onCamFrame);
      const st = t.getSettings();
      status("st-cam", "good", `Camera: ${t.label} ${st.width}x${st.height}`);
      t.addEventListener("ended", () => { status("st-cam", "warn", "Camera: lost, reconnecting"); cam = null; });
    } catch (e) {
      status("st-cam", "warn", `Camera: cannot open (${(e as Error).name}). Is the Z8 on and in webcam mode?`);
    }
  }
  camSel.onchange = openCam;
  // The Z8 feed restarts when the camera sleeps or the mode changes: retry.
  setInterval(() => { if (camWant && (!cam || !cam.live())) openCam(); }, 3000);

  $<HTMLButtonElement>("z8on").onclick = async () => {
    const r = await api("/api/mode", { mode: "webcam" });
    if (r.ok === false || r.webcam === false) alarm("Z8 webcam did not start: " + (r.error || r.msg || "is the camera on and plugged in?"));
    else { status("st-cam", "warn", "Camera: Z8 starting"); setTimeout(openCam, 2500); }
  };

  // ----------------------------------------------------------------- mic
  // The graph: mic -> analyser + recorder input. Swapping the mic source does
  // not touch the MediaRecorder, so a re-opened mic joins the same take.
  const ac = new AudioContext({ sampleRate: 48000 });
  const mixOut = ac.createMediaStreamDestination();
  const analyser = ac.createAnalyser();
  analyser.fftSize = 2048;
  analyser.connect(mixOut);
  let micNode: MediaStreamAudioSourceNode | null = null, micTrack: MediaStreamTrack | null = null;
  let micWant = "", micOpening = false, lastReopen = 0, reopenCount = 0;

  async function openMic(why = "") {
    if (micOpening) return;
    micOpening = true;
    try {
      micNode?.disconnect(); micTrack?.stop(); micNode = null; micTrack = null;
      micWant = micSel.value;
      store.set("mic", micWant);
      if (!micWant) { status("st-mic", recording ? "bad" : "", "Mic: off"); return; }
      const s = await navigator.mediaDevices.getUserMedia({
        audio: {
          deviceId: { exact: micWant }, channelCount: 1, sampleRate: 48000,
          // Studio mic: no phone-call processing.
          echoCancellation: false, noiseSuppression: false, autoGainControl: false,
        },
      });
      micTrack = s.getAudioTracks()[0];
      micNode = ac.createMediaStreamSource(s);
      micNode.connect(analyser);
      silentSince = 0;
      status("st-mic", "good", `Mic: ${micTrack.label}${why ? " (re-opened: " + why + ")" : ""}`);
      micTrack.addEventListener("ended", () => reopenMic("the device went away"));
      if (ac.state !== "running") ac.resume();
    } catch (e) {
      status("st-mic", "bad", `Mic: cannot open (${(e as Error).name})`);
      if (recording) alarm("The mic cannot be opened. Check the Scarlett USB cable.", true);
    } finally {
      micOpening = false;
    }
  }
  let lastReset = 0;
  async function resetMic() {
    const now = performance.now();
    if (now - lastReset < 8000) return;
    lastReset = now; silentSince = 0;
    status("st-mic", "bad", "Mic: went silent, resetting the Scarlett");
    if (recording) alarm("The mic dropped out. Resetting it now; the gap is about 3 seconds. Repeat that line.", true);
    const r = await api("/api/reset-mic", {}).catch(() => ({ ok: false }));
    // The reset re-enumerates the device; give it a moment, then re-open.
    setTimeout(() => openMic(r.ok ? "reset after a dropout" : "dropout, reset failed"), 2500);
    if (recording) setTimeout(() => alertBox.classList.remove("on"), 9000);
  }
  function reopenMic(why: string) {
    const now = performance.now();
    if (now - lastReopen < 3000) return;
    lastReopen = now;
    reopenCount++;
    status("st-mic", "bad", `Mic: ${why}, re-opening`);
    if (recording) alarm(`Mic problem: ${why}. Re-opened it. Check the backup WAV for this moment.`);
    openMic(why);
  }
  micSel.onchange = () => openMic();
  navigator.mediaDevices.addEventListener("devicechange", async () => {
    await listDevices();
    if (micWant && (!micTrack || micTrack.readyState !== "live")) openMic("device came back");
  });

  // A real mic always hisses a little. Exact zeros for 2 s mean the input is
  // dead (the OBS failure on 2026-09-22), not that the room is quiet.
  let silentSince = 0;
  const buf = new Float32Array(analyser.fftSize);
  setInterval(() => {
    analyser.getFloatTimeDomainData(buf);
    let peak = 0;
    for (let i = 0; i < buf.length; i++) peak = Math.max(peak, Math.abs(buf[i]));
    const db = peak > 0 ? 20 * Math.log10(peak) : -120;
    meter.style.width = Math.max(0, Math.min(100, (db + 60) / 60 * 100)) + "%";
    meter.classList.toggle("hot", db > -1);
    if (!micTrack) return;
    if (peak === 0) {
      silentSince ||= performance.now();
      // 1.5 s of exact zeros: the AMD controller dropped the stream. Re-opening
      // alone never brings it back, so USB-reset the Scarlett first (about 2 s),
      // then the devicechange handler re-opens it.
      if (performance.now() - silentSince > 1500) resetMic();
    } else {
      silentSince = 0;
      if (db > -1 && recording) status("st-mic", "warn", "Mic: clipping, turn the Scarlett gain down");
    }
  }, 100);

  // ----------------------------------------------------------- compositor
  // Ticks come from a worker: its timer is not throttled when this page is
  // in the background, a main-thread timer or rAF is.
  const tickSrc = "let t=setInterval(()=>postMessage(0)," + Math.round(1000 / FPS) + ");";
  const ticker = new Worker(URL.createObjectURL(new Blob([tickSrc], { type: "text/javascript" })));

  function fit(sw: number, sh: number, bw: number, bh: number): [number, number, number, number] {
    if (!sw || !sh) return [0, 0, bw, bh];
    const s = Math.min(bw / sw, bh / sh), w = sw * s, h = sh * s;
    return [(bw - w) / 2, (bh - h) / 2, w, h];
  }
  function rounded(x: number, y: number, w: number, h: number, r: number) {
    ctx.beginPath();
    ctx.moveTo(x + r, y); ctx.arcTo(x + w, y, x + w, y + h, r); ctx.arcTo(x + w, y + h, x, y + h, r);
    ctx.arcTo(x, y + h, x, y, r); ctx.arcTo(x, y, x + w, y, r); ctx.closePath();
  }
  // shadowBlur costs a full blur on every frame; draw the shadow once per
  // size and paste it.
  const SH = 40;
  let shadow: { w: number; h: number; c: HTMLCanvasElement } | null = null;
  function shadowFor(w: number, h: number): HTMLCanvasElement {
    if (shadow && shadow.w === w && shadow.h === h) return shadow.c;
    const c = document.createElement("canvas");
    c.width = w + SH * 2; c.height = h + SH * 2;
    const g = c.getContext("2d")!;
    g.shadowColor = "rgba(0,0,0,.5)"; g.shadowBlur = 24; g.fillStyle = "#000";
    g.beginPath(); g.roundRect(SH, SH, w, h, 18); g.fill();
    shadow = { w, h, c };
    return c;
  }
  function placeholder(text: string) {
    ctx.fillStyle = "#9aa3b2"; ctx.font = "600 40px system-ui"; ctx.textAlign = "center";
    ctx.fillText(text, W / 2, H / 2);
  }
  let composeMs = 0, lastCompose = 0, gaps: number[] = [];
  function compose() {
    const t0 = performance.now();
    if (lastCompose) { gaps.push(t0 - lastCompose); if (gaps.length > 90) gaps.shift(); }
    lastCompose = t0;
    ctx.fillStyle = "#000"; ctx.fillRect(0, 0, W, H);
    const layout = layoutSel.value;
    if (layout === "cam" || (layout === "pip" && !screen)) {
      if (cam) { const [x, y, w, h] = fit(...cam.size(), W, H); cam.draw(x, y, w, h); }
      else placeholder("Camera is off");
    } else {
      if (screen) { const [x, y, w, h] = fit(...screen.size(), W, H); screen.draw(x, y, w, h); }
      else placeholder("Press Share screen");
      if (layout === "pip" && cam) {
        const [cw, ch] = cam.size();
        const pw = Math.round(W * Number(sizeSel.value)), ph = Math.round(pw * (ch && cw ? ch / cw : 9 / 16));
        const m = 32, c = cornerSel.value;
        const x = c.endsWith("r") ? W - pw - m : m, y = c.startsWith("b") ? H - ph - m : m;
        ctx.drawImage(shadowFor(pw, ph), x - SH, y - SH);
        ctx.save();
        rounded(x, y, pw, ph, 18); ctx.fillStyle = "#000"; ctx.fill();
        ctx.clip();
        cam.draw(x, y, pw, ph);
        ctx.restore();
      }
    }
    pushFrame();
    composeMs = composeMs * 0.9 + (performance.now() - t0) * 0.1;
  }
  // The camera's own frames set the pace when it is on screen: a separate
  // 30 Hz timer beating against its 29.x fps repeats or drops a frame about
  // once a second (a visible hitch) and adds up to 33 ms of delay. The worker
  // tick still runs, and composes only when no camera frame came for 45 ms.
  const camShown = () => !!cam && (layoutSel.value !== "screen");
  function onCamFrame() { if (camShown()) compose(); }
  ticker.onmessage = () => { ticks++; if (!camShown() || performance.now() - lastCompose > 45) compose(); };
  setInterval(() => {
    if (gaps.length < 10) return;
    const avg = gaps.reduce((a, b) => a + b, 0) / gaps.length;
    const worst = Math.max(...gaps);
    const fps = 1000 / avg;
    const lvl = fps < 24 || worst > 120 ? "bad" : fps < 28 || worst > 70 ? "warn" : "good";
    status("st-perf", lvl, `Preview: ${fps.toFixed(0)} fps, ${composeMs.toFixed(1)} ms to draw, worst gap ${worst.toFixed(0)} ms` +
      (lvl === "good" ? "" : ". The computer is busy: close heavy tabs or jobs"));
  }, 1000);

  // The canvas becomes a video track. A generator gets an explicit frame per
  // tick; captureStream(0) + requestFrame is the fallback.
  let videoTrack: MediaStreamTrack, pushFrame: () => void, ticks = 0, pushed = 0, skipped = 0;
  const Gen = (window as any).MediaStreamTrackGenerator, VF = (window as any).VideoFrame;
  const useGen = !!(Gen && VF) && !location.search.includes("gen=0");
  if (useGen) {
    const gen = new Gen({ kind: "video" });
    const writer = gen.writable.getWriter();
    let busy = false;
    videoTrack = gen;
    pushFrame = () => {
      if (busy) { skipped++; return; }
      busy = true; pushed++;
      const f = new VF(out, { timestamp: Math.round(performance.now() * 1000) });
      writer.write(f).catch(() => {}).finally(() => { f.close(); busy = false; });
    };
  } else {
    const t = out.captureStream(0).getVideoTracks()[0] as any;
    videoTrack = t;
    pushFrame = () => { pushed++; t.requestFrame(); };
  }

  // ------------------------------------------------------------ recording
  // WebM first: Chrome's MP4 muxer holds the whole take in memory until
  // stop (measured: 1.2 KB delivered in 6 s), so a crash would lose it all.
  // WebM delivers every second. H.264 in WebM is remuxed to .mp4 at the end.
  const TYPES = [
    "video/webm;codecs=h264,opus",
    "video/webm;codecs=vp9,opus",
    "video/webm;codecs=vp8,opus",
    "video/webm",
  ];
  const mime = TYPES.find(t => MediaRecorder.isTypeSupported(t)) || "";
  const ext = mime.startsWith("video/mp4") ? "mp4" : "webm";
  const h264 = /avc1|h264/.test(mime);

  let rec: MediaRecorder | null = null, recording = false, takeId = "", started = 0;
  let queue: Blob[] = [], sent = 0, uploading = false, failures = 0;
  let backupName: string | null = null;

  async function upload() {
    if (uploading) return;
    uploading = true;
    while (queue.length) {
      const b = queue[0];
      try {
        const r = await fetch(`/api/rec-chunk?id=${takeId}&ext=${ext}&off=${sent}`, { method: "POST", body: b });
        const j = await r.json();
        if (!j.ok) throw new Error(j.error || `HTTP ${r.status}`);
        sent = j.size; queue.shift(); failures = 0;
        status("st-save", "good", `Saving: ${(sent / 1048576).toFixed(1)} MB on disk`);
      } catch (e) {
        failures++;
        const held = queue.reduce((n, x) => n + x.size, 0) / 1048576;
        status("st-save", "bad", `Saving: failed (${(e as Error).message}), holding ${held.toFixed(1)} MB, retrying`);
        if (failures === 3) alarm("Saving to disk is failing. Do not close this window. It keeps retrying.", true);
        await new Promise(r => setTimeout(r, Math.min(5000, 500 * failures)));
      }
    }
    uploading = false;
  }

  function updateGo() {
    goBtn.disabled = !recording && !mime;
    goBtn.textContent = recording ? "Stop" : "Record";
    goBtn.classList.toggle("on", recording);
  }

  async function start() {
    if (!mime) { alarm("This browser cannot record video. Use Google Chrome."); return; }
    if (layoutSel.value !== "cam" && !screen) alarm("No screen shared yet. Recording the camera until you share.");
    if (!micTrack) alarm("No mic is open. This take will have no sound.", true);
    await ac.resume();
    takeId = stamp(); queue = []; sent = 0; failures = 0; reopenCount = 0; backupName = null;
    const stream = new MediaStream([videoTrack, ...mixOut.stream.getAudioTracks()]);
    rec = new MediaRecorder(stream, { mimeType: mime, videoBitsPerSecond: VIDEO_BPS, audioBitsPerSecond: AUDIO_BPS });
    rec.ondataavailable = e => { if (e.data.size) { queue.push(e.data); upload(); } };
    rec.onerror = e => alarm("Recorder error: " + ((e as any).error?.message || "unknown"), true);
    rec.start(1000);
    recording = true; started = performance.now();
    $<HTMLSpanElement>("recdot").classList.add("on");
    clock.classList.add("on");
    status("st-save", "good", "Saving: started");
    updateGo();
    if ($<HTMLInputElement>("backup").checked) {
      const r = await api("/api/audio-record", { on: true }).catch(() => ({ ok: false }));
      if (r.ok) { backupName = r.name; status("st-backup", "good", `Backup audio: ${r.name}`); }
      else status("st-backup", "warn", "Backup audio: not running (" + (r.error || "no answer") + ")");
    }
    if ($<HTMLInputElement>("pauto").checked) prompter({ type: "play" });
  }

  async function stop() {
    if (!rec) return;
    const r = rec; rec = null;
    await new Promise<void>(res => { r.onstop = () => res(); r.stop(); });
    recording = false;
    $<HTMLSpanElement>("recdot").classList.remove("on");
    clock.classList.remove("on");
    updateGo();
    prompter({ type: "pause" });
    if (backupName) {
      await api("/api/audio-record", { on: false }).catch(() => null);
      status("st-backup", "good", `Backup audio: saved ${backupName} in today's Z8 session folder`);
    }
    status("st-save", "warn", "Saving: writing the last seconds");
    while (queue.length || uploading) await new Promise(res => setTimeout(res, 200));
    status("st-save", "warn", "Saving: making the final file");
    const clean = $<HTMLInputElement>("clean").checked;
    if (clean) status("st-save", "warn", "Saving: making the final file and the noise-removed copy");
    if (backupName) status("st-save", "warn", "Saving: making the final file and the clean-audio (WAV) copy");
    const f = await api("/api/rec-finish", { id: takeId, ext, h264, clean, wav: backupName || undefined });
    const line = document.createElement("div");
    if (f.ok) {
      const mins = f.seconds ? ` ${Math.floor(f.seconds / 60)}m ${Math.round(f.seconds % 60)}s,` : "";
      // 2026-10-20: on a blind A/C/D listen the browser track sounded best and
      // the backup-WAV copies crackled, so the original is the upload until the
      // WAV path is proven clean again. The -wav copy stays as a fallback.
      line.textContent = `UPLOAD THIS: ${f.path} (${mins} ${((f.size || sent) / 1048576).toFixed(0)} MB)` +
        (f.wav ? ` · backup-audio copy (fallback): ${f.wav}` : "") +
        (f.clean ? ` · noise removed: ${f.clean}` : "") +
        (reopenCount ? ` – mic re-opened ${reopenCount}x, check the backup WAV` : "");
      status("st-save", f.warning ? "warn" : "good", f.warning ? "Saving: done, but " + f.warning : "Saving: done");
    } else {
      line.textContent = `Take ${takeId}: ${f.error}`;
      status("st-save", "bad", "Saving: " + f.error);
    }
    const takes = $<HTMLDivElement>("takes");
    if (takes.textContent === "None yet.") takes.textContent = "";
    takes.prepend(line);
  }

  goBtn.onclick = () => { recording ? stop() : start(); };
  window.addEventListener("beforeunload", e => { if (recording) { e.preventDefault(); e.returnValue = ""; } });
  document.addEventListener("visibilitychange", () => {
    if (document.hidden && recording) console.warn("[rec] page hidden while recording");
  });
  setInterval(() => {
    if (!recording) return;
    const s = Math.floor((performance.now() - started) / 1000);
    clock.textContent = `${String(Math.floor(s / 60)).padStart(2, "0")}:${String(s % 60).padStart(2, "0")}`;
  }, 250);
  $<HTMLButtonElement>("folder").onclick = () => api("/api/rec-open-folder", {});

  // ---------------------------------------------------------- teleprompter
  const chan = new BroadcastChannel("z8-prompter");
  const script = $<HTMLTextAreaElement>("script");
  const pspeed = $<HTMLInputElement>("pspeed"), psize = $<HTMLInputElement>("psize"), pmirror = $<HTMLInputElement>("pmirror");
  const pmargin = $<HTMLInputElement>("pmargin"), pline = $<HTMLInputElement>("pline"), pmark = $<HTMLInputElement>("pmark");
  const palign = $<HTMLSelectElement>("palign");
  function readouts() {
    $<HTMLSpanElement>("psize-v").textContent = `${psize.value} px`;
    $<HTMLSpanElement>("pmargin-v").textContent = `${pmargin.value}%`;
    $<HTMLSpanElement>("pline-v").textContent = `${Number(pline.value).toFixed(2)}x`;
    $<HTMLSpanElement>("pmark-v").textContent = `${pmark.value}%`;
  }
  const pplay = $<HTMLButtonElement>("pplay");
  function prompter(m: Record<string, unknown>) { chan.postMessage(m); }
  function sendAll() {
    prompter({ type: "config", text: script.value, speed: Number(pspeed.value), size: Number(psize.value), mirror: pmirror.checked,
      margin: Number(pmargin.value), line: Number(pline.value), mark: Number(pmark.value), align: palign.value });
    readouts();
  }
  script.value = store.get("script") || "";
  pspeed.value = store.get("pspeed") || pspeed.value;
  // 2026-10-15: new defaults (smaller text, narrower column). Reset the saved
  // values once so the old 64 px / 6% do not keep overriding them.
  if (store.get("pdefaults") !== "2") {
    for (const k of ["psize", "pmargin"]) { try { localStorage.removeItem("rec." + k); } catch { /* private */ } }
    store.set("pdefaults", "2");
  }
  psize.value = store.get("psize") || psize.value;
  pmargin.value = store.get("pmargin") || pmargin.value;
  pline.value = store.get("pline") || pline.value;
  pmark.value = store.get("pmark") || pmark.value;
  palign.value = store.get("palign") || palign.value;
  palign.onchange = () => { store.set("palign", palign.value); sendAll(); };
  readouts();
  pmirror.checked = store.get("pmirror") === "1";
  $<HTMLInputElement>("pauto").checked = store.get("pauto") !== "0";
  // The video pipeline writes each new script to ~/Pictures/z8-studio/scripts/latest.txt.
  // A file newer than the last one loaded replaces the box once, so edits made
  // here are kept until the next script arrives. Never swapped mid-take.
  async function pullScript() {
    if (recording) return;
    const r = await fetch("/api/rec-script").then(x => x.json()).catch(() => null) as
      { text?: string | null; at?: number; name?: string | null;
        charts?: { title?: string; url?: string; note?: string }[] } | null;
    if (!r || !r.text || !r.at) return;
    renderCharts(r.charts || []);
    const label = $<HTMLParagraphElement>("script-src");
    const name = r.name || "latest.txt";
    if (r.at <= Number(store.get("scriptAt") || "0")) { label.textContent = `Script: ${name}`; return; }
    script.value = r.text;
    store.set("script", r.text);
    store.set("scriptAt", String(r.at));
    label.textContent = `Loaded the newest script: ${name}`;
    sendAll();
  }
  function renderCharts(charts: { title?: string; url?: string; note?: string }[]) {
    const list = $<HTMLOListElement>("charts");
    list.replaceChildren();
    if (!charts.length) {
      const li = document.createElement("li");
      li.textContent = "No chart list for this script yet.";
      li.style.color = "var(--dim)";
      list.append(li);
      return;
    }
    for (const c of charts) {
      const li = document.createElement("li");
      const a = document.createElement("a");
      a.href = c.url || "#"; a.target = "_blank"; a.rel = "noopener";
      a.textContent = c.title || c.url || "chart";
      li.append(a);
      if (c.note) {
        const n = document.createElement("span");
        n.textContent = ` (${c.note})`; n.style.color = "var(--dim)";
        li.append(n);
      }
      list.append(li);
    }
  }
  $<HTMLButtonElement>("charts-open").onclick = async () => {
    const b = $<HTMLButtonElement>("charts-open");
    const r = await api("/api/open-charts", {}).catch(() => ({ ok: false, error: "server unreachable" }));
    b.textContent = r.ok ? `Opened ${r.opened} charts` : `Could not open: ${r.error}`;
    setTimeout(() => { b.textContent = "Open all in the browser"; }, 4000);
  };
  type CheckRow = { label: string; said: unknown; status: string; detail: string };
  function renderCheck(c: { checkedAt?: string; results?: CheckRow[] } | null) {
    const list = $<HTMLUListElement>("check-list");
    const sum = $<HTMLParagraphElement>("check-sum");
    list.replaceChildren();
    if (!c || !c.results) { sum.textContent = "No numbers check for this script yet."; return; }
    const n = (s: string) => c.results!.filter(r => r.status === s).length;
    const mins = c.checkedAt ? Math.round((Date.now() - Date.parse(c.checkedAt)) / 60000) : null;
    sum.textContent = `${n("ok")} still right, ${n("moved")} moved, ${n("manual")} to check by hand, ${n("error")} failed`
      + (mins === null ? "" : `. Checked ${mins < 1 ? "just now" : `${mins} min ago`}.`);
    const color: Record<string, string> = { ok: "var(--good, #3fb950)", moved: "var(--bad, #f85149)", error: "var(--bad, #f85149)", manual: "var(--warn, #d29922)" };
    const word: Record<string, string> = { ok: "OK", moved: "MOVED", error: "ERROR", manual: "CHECK" };
    // Problems first, so a moved number is never below the fold.
    const order: Record<string, number> = { moved: 0, error: 1, manual: 2, ok: 3 };
    for (const r of [...c.results].sort((a, b) => (order[a.status] ?? 9) - (order[b.status] ?? 9))) {
      const li = document.createElement("li");
      const tag = document.createElement("b");
      tag.textContent = `${word[r.status] || r.status} `; tag.style.color = color[r.status] || "";
      li.append(tag, `${r.label}: said ${String(r.said)}, ${r.detail}`);
      list.append(li);
    }
  }
  async function pullCheck() {
    const r = await fetch("/api/rec-check").then(x => x.json()).catch(() => null);
    renderCheck(r ? r.check : null);
  }
  $<HTMLButtonElement>("check-run").onclick = async () => {
    const b = $<HTMLButtonElement>("check-run");
    b.disabled = true; b.textContent = "Checking...";
    const r = await api("/api/check-numbers", {}).catch(() => ({ ok: false, error: "server unreachable" }));
    b.disabled = false; b.textContent = "Re-check numbers now";
    if (r.ok) renderCheck(r.check);
    else $<HTMLParagraphElement>("check-sum").textContent = `Check failed: ${r.error}`;
  };
  void pullCheck();
  window.addEventListener("focus", () => { void pullCheck(); });
  // ------------------------------------------------------ studio ready
  // One list of everything that went wrong on a recording night, each with a
  // fix button where a fix can be automated. Runs on load, on focus and every
  // 20 s, never while recording.
  type Row = { name: string; level: "good" | "warn" | "bad"; text: string; fix?: [string, () => unknown] };
  function fromStatus(id: string, name: string, fix?: [string, () => unknown]): Row {
    const el = $<HTMLDivElement>(id);
    const level = el.classList.contains("bad") ? "bad" : el.classList.contains("good") ? "good" : "warn";
    return { name, level, text: el.textContent || "", fix: level === "good" ? undefined : fix };
  }
  function pingPrompter(): Promise<{ w: number; h: number; full: boolean } | null> {
    return new Promise(res => {
      const t = setTimeout(() => { chan.removeEventListener("message", on); res(null); }, 600);
      const on = (e: MessageEvent) => {
        if (e.data?.type !== "pong") return;
        clearTimeout(t); chan.removeEventListener("message", on); res(e.data);
      };
      chan.addEventListener("message", on);
      chan.postMessage({ type: "ping" });
    });
  }
  async function studioReady() {
    if (recording) return;
    const [srv, pong] = await Promise.all([
      fetch("/api/studio-ready").then(r => r.json()).catch(() => null), pingPrompter()]);
    const rows: Row[] = [];
    const sc = srv?.screens || {};
    if (!srv) rows.push({ name: "Server", level: "bad", text: "The Z8 Studio server is not answering." });
    else if (!sc.ok) rows.push({ name: "Screens", level: "warn", text: `Could not read the screens: ${sc.error || "unknown"}` });
    else if (!sc.elgatoUsb) rows.push({ name: "Elgato", level: "bad", text: "Not on USB. Plug its cable into a port on the back of the PC, not the USB-C dock." });
    else if (!sc.elgatoScreen) rows.push({ name: "Elgato", level: "bad", text: "Plugged in but not a screen yet. Log out and back in once (fixed for good after the next reboot)." });
    else {
      rows.push({ name: "Elgato", level: "good", text: "Connected as a screen." });
      rows.push(sc.layoutOk
        ? { name: "Screens", level: "good", text: "Monitors side by side, Elgato below the middle." }
        : { name: "Screens", level: "warn", text: "Layout is off, so the mouse gets stuck reaching the Elgato.",
            fix: ["Fix layout", async () => { await api("/api/fix-layout", {}); }] });
    }
    if (!pong) rows.push({ name: "Prompter", level: "bad", text: "The prompter window is not open.", fix: ["Open prompter", () => $<HTMLButtonElement>("popen").click()] });
    else if (!(pong.w === 1024 && pong.h === 600)) rows.push({ name: "Prompter", level: "warn", text: `Open, but on a ${pong.w}x${pong.h} screen. Drag it down onto the Elgato.` });
    else if (!pong.full) rows.push({ name: "Prompter", level: "warn", text: "On the Elgato. Click it and press F for full screen." });
    else rows.push({ name: "Prompter", level: "good", text: "Full screen on the Elgato." });
    rows.push(fromStatus("st-screen", "Screen share", ["Share screen", () => $<HTMLButtonElement>("share").click()]));
    rows.push(fromStatus("st-cam", "Camera", ["Start Z8 webcam", () => $<HTMLButtonElement>("z8on").click()]));
    rows.push(layoutSel.value === "pip"
      ? { name: "Layout", level: "good", text: "Screen + camera in corner." }
      : { name: "Layout", level: "warn", text: `Set to ${layoutSel.options[layoutSel.selectedIndex].text}.`,
          fix: ["Screen + camera", () => { layoutSel.value = "pip"; layoutSel.dispatchEvent(new Event("change")); }] });
    if (srv?.micPort) rows.push({ name: "Mic port", level: srv.micPort.level, text: `${srv.micPort.why}${srv.micPort.controller ? ` (${srv.micPort.controller})` : ""}` });
    if (srv && !srv.micDevice) rows.push({ name: "Mic", level: "bad", text: "The Scarlett is not detected. Check its USB cable." });
    else rows.push(fromStatus("st-mic", "Mic"));
    if (srv?.script) {
      const sp = srv.script;
      rows.push(sp.loaded
        ? { name: "Script", level: "good", text: `${sp.name || "loaded"}, ${sp.charts} charts.` }
        : { name: "Script", level: "bad", text: "No script in the recorder yet." });
      if (sp.charts) rows.push({ name: "Charts", level: "warn", text: `Open the ${sp.charts} charts before you start.`, fix: ["Open all", () => $<HTMLButtonElement>("charts-open").click()] });
      const res: { status: string }[] = srv.check?.results || [];
      const moved = res.filter(r => r.status === "moved" || r.status === "error").length;
      const manual = res.filter(r => r.status === "manual").length;
      rows.push(!res.length ? { name: "Numbers", level: "warn", text: "Not checked yet.", fix: ["Check numbers", () => $<HTMLButtonElement>("check-run").click()] }
        : moved ? { name: "Numbers", level: "bad", text: `${moved} quoted number(s) moved. See Numbers check below.`, fix: ["Re-check", () => $<HTMLButtonElement>("check-run").click()] }
        : { name: "Numbers", level: manual ? "warn" : "good", text: manual ? `All pulled numbers still right; ${manual} to check by hand.` : "All still right." });
    }
    if (srv && typeof srv.diskFreeGb === "number") {
      const g = srv.diskFreeGb;
      rows.push({ name: "Disk", level: g < 5 ? "bad" : g < 20 ? "warn" : "good", text: `${g.toFixed(0)} GB free for recordings.` });
    }
    if (srv && typeof srv.load === "number") {
      const busy = srv.load / srv.cpus;
      const top = (srv.topCpu || []).filter((t: { cpu: number }) => t.cpu > 50)
        .map((t: { cpu: number; name: string }) => `${t.name} ${Math.round(t.cpu)}%`).join(", ");
      rows.push(busy > 0.8
        ? { name: "Computer", level: "warn", text: `Busy (load ${srv.load.toFixed(1)} on ${srv.cpus} cores)${top ? `: ${top}` : ""}. The preview may stutter.` }
        : { name: "Computer", level: "good", text: "Not overloaded." });
    }
    const list = $<HTMLUListElement>("ready-list");
    list.replaceChildren();
    const order = { bad: 0, warn: 1, good: 2 };
    const dot = { bad: "#f85149", warn: "#d29922", good: "#3fb950" };
    for (const r of rows.sort((a, b) => order[a.level] - order[b.level])) {
      const li = document.createElement("li");
      const d = document.createElement("span");
      d.textContent = "\u25CF "; d.style.color = dot[r.level];
      const n = document.createElement("b");
      n.textContent = `${r.name}: `;
      li.append(d, n, r.text);
      if (r.fix) {
        const b = document.createElement("button");
        b.textContent = r.fix[0]; b.style.marginLeft = "8px";
        const f = r.fix[1];
        b.onclick = async () => { await f(); setTimeout(() => void studioReady(), 1500); };
        li.append(b);
      }
      list.append(li);
    }
    const bad = rows.filter(r => r.level === "bad").length, warn = rows.filter(r => r.level === "warn").length;
    $<HTMLParagraphElement>("ready-sum").textContent = !bad && !warn ? "All set. Ready to record."
      : `${bad ? `${bad} to fix` : ""}${bad && warn ? ", " : ""}${warn ? `${warn} to look at` : ""}.`;
  }
  $<HTMLButtonElement>("ready-run").onclick = () => void studioReady();
  setTimeout(() => void studioReady(), 3000);
  setInterval(() => void studioReady(), 20000);
  window.addEventListener("focus", () => setTimeout(() => void studioReady(), 500));
  void pullScript();
  window.addEventListener("focus", () => { void pullScript(); });
  const cleanBox = $<HTMLInputElement>("clean");
  cleanBox.checked = store.get("clean") !== "0";
  cleanBox.onchange = () => store.set("clean", cleanBox.checked ? "1" : "0");
  script.oninput = () => { store.set("script", script.value); sendAll(); };
  pspeed.oninput = () => { store.set("pspeed", pspeed.value); sendAll(); };
  psize.oninput = () => { store.set("psize", psize.value); sendAll(); };
  for (const [el, k] of [[pmargin, "pmargin"], [pline, "pline"], [pmark, "pmark"]] as const)
    el.oninput = () => { store.set(k, el.value); sendAll(); };
  pmirror.onchange = () => { store.set("pmirror", pmirror.checked ? "1" : "0"); sendAll(); };
  $<HTMLInputElement>("pauto").onchange = e => store.set("pauto", (e.target as HTMLInputElement).checked ? "1" : "0");
  pplay.onclick = () => prompter({ type: "toggle" });
  $<HTMLButtonElement>("preset").onclick = () => prompter({ type: "reset" });
  $<HTMLButtonElement>("popen").onclick = () => {
    window.open("/prompter", "z8-prompter", "popup,width=1024,height=600");
  };
  chan.onmessage = e => {
    const m = e.data || {};
    if (m.type === "hello") sendAll();
    if (m.type === "state") {
      pplay.textContent = m.playing ? "Pause" : "Play";
      pspeed.value = String(m.speed); store.set("pspeed", pspeed.value);
      // Keys pressed in the prompter window (+/- and [ ]) come back here.
      if (m.size) { psize.value = String(m.size); store.set("psize", psize.value); }
      if (typeof m.margin === "number") { pmargin.value = String(m.margin); store.set("pmargin", pmargin.value); }
      if (m.align) { palign.value = m.align; store.set("palign", palign.value); }
      readouts();
    }
  };

  // ------------------------------------------------------------------ keys
  document.addEventListener("keydown", e => {
    const t = e.target as HTMLElement;
    if (t.tagName === "TEXTAREA" || t.tagName === "INPUT" || t.tagName === "SELECT") return;
    if (e.key === "1") layoutSel.value = "pip";
    else if (e.key === "2") layoutSel.value = "cam";
    else if (e.key === "3") layoutSel.value = "screen";
    else return;
    store.set("layout", layoutSel.value);
  });
  // The layout is NOT restored: every session starts on screen + camera in the
  // corner, because a camera-only layout left over from last time recorded a
  // take with no screen (2026-10-15). Corner and size are still remembered.
  layoutSel.value = "pip";
  for (const [sel, key] of [[cornerSel, "corner"], [sizeSel, "pipsize"]] as const) {
    const v = store.get(key);
    if (v) sel.value = v;
    sel.addEventListener("change", () => store.set(key, sel.value));
  }

  // ----------------------------------------------------------------- boot
  (async () => {
    // Labels are hidden until the page has permission once.
    try {
      const s = await navigator.mediaDevices.getUserMedia({ audio: true, video: true }).catch(() =>
        navigator.mediaDevices.getUserMedia({ audio: true }));
      s.getTracks().forEach(t => t.stop());
    } catch { /* the pickers will show what they can */ }
    await listDevices();
    await Promise.all([openCam(), openMic()]);
    updateGo();
    if (!mime) alarm("This browser cannot record video. Use Google Chrome.", true);
    (window as any).__rec = { mime, ext, h264, state: () => ({ composeMs: +composeMs.toFixed(2), gapAvg: gaps.length ? +(gaps.reduce((a, b) => a + b, 0) / gaps.length).toFixed(1) : null,
      gapMax: gaps.length ? +Math.max(...gaps).toFixed(1) : null, useGen, ticks, pushed, skipped, recording, sent, queued: queue.length, takeId, reopenCount,
      mic: micTrack?.label, cam: cam ? cam.size() : null, screen: screen ? screen.size() : null }) };
  })();
})();
