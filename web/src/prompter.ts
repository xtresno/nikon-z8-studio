// The teleprompter window (/prompter): drag it onto the Elgato Prompter's
// screen and press F. The recorder page sends the script and settings over a
// BroadcastChannel; this window owns the scrolling so it stays smooth.
(() => {
  const view = document.getElementById("view") as HTMLDivElement;
  const text = document.getElementById("text") as HTMLDivElement;
  const hint = document.getElementById("hint") as HTMLDivElement;
  const chan = new BroadcastChannel("z8-prompter");
  // speed is pixels per second at 64 px text; it scales with the text size.
  let y = 0, speed = 60, size = 48, playing = false, last = 0;
  let margin = 22, line = 1.35, mark = 33, align = "center";

  const startY = () => view.clientHeight * mark / 100;
  function layout() {
    text.style.fontSize = size + "px";
    text.style.lineHeight = String(line);
    text.style.textAlign = align;
    text.style.left = text.style.right = margin + "%";
    (document.getElementById("mark") as HTMLDivElement).style.top = mark + "%";
    render();
  }
  function render() { text.style.transform = `translateY(${Math.round(startY() - y)}px)`; }
  function tell() { chan.postMessage({ type: "state", playing, speed, y, size, margin, align }); }
  function setPlaying(p: boolean) { playing = p; last = performance.now(); hint.style.display = p ? "none" : ""; tell(); }

  function frame(now: number) {
    if (playing) {
      y += speed * (size / 64) * (now - last) / 1000;
      const end = text.scrollHeight;
      if (y > end) { y = end; setPlaying(false); }
      render();
    }
    last = now;
    requestAnimationFrame(frame);
  }
  requestAnimationFrame(frame);

  chan.onmessage = e => {
    const m = e.data || {};
    if (m.type === "config") {
      if (typeof m.text === "string") text.textContent = m.text || "Paste your script in the Z8 Recorder page.";
      if (m.speed) speed = m.speed;
      if (m.size) size = m.size;
      if (typeof m.margin === "number") margin = m.margin;
      if (m.line) line = m.line;
      if (m.mark) mark = m.mark;
      if (m.align === "left" || m.align === "center") align = m.align;
      layout();
      view.classList.toggle("mirror", !!m.mirror);
      render();
    }
    if (m.type === "play") setPlaying(true);
    if (m.type === "pause") setPlaying(false);
    if (m.type === "toggle") setPlaying(!playing);
    if (m.type === "reset") { y = 0; setPlaying(false); render(); }
    // The recorder's Studio ready card asks where this window is.
    if (m.type === "ping") chan.postMessage({ type: "pong", w: screen.width, h: screen.height, full: !!document.fullscreenElement });
  };

  document.addEventListener("keydown", e => {
    const line = size * 1.35;
    if (e.key === " ") setPlaying(!playing);
    else if (e.key === "ArrowUp") { speed = Math.min(400, speed + 5); tell(); }
    else if (e.key === "ArrowDown") { speed = Math.max(5, speed - 5); tell(); }
    else if (e.key === "ArrowRight" || e.key === "PageDown") { y += line * 3; render(); }
    else if (e.key === "ArrowLeft" || e.key === "PageUp") { y = Math.max(0, y - line * 3); render(); }
    else if (e.key === "Home") { y = 0; render(); }
    else if (e.key === "+" || e.key === "=") { size = Math.min(200, size + 4); layout(); tell(); }
    else if (e.key === "-" || e.key === "_") { size = Math.max(24, size - 4); layout(); tell(); }
    else if (e.key === "c" || e.key === "C") { align = align === "center" ? "left" : "center"; layout(); tell(); }
    else if (e.key === "]") { margin = Math.min(40, margin + 1); layout(); tell(); }
    else if (e.key === "[") { margin = Math.max(0, margin - 1); layout(); tell(); }
    else if (e.key === "f" || e.key === "F") {
      if (document.fullscreenElement) document.exitFullscreen(); else document.documentElement.requestFullscreen();
    } else return;
    e.preventDefault();
  });
  // Scroll wheel nudges, for fixing your place by hand.
  window.addEventListener("wheel", e => { y = Math.max(0, y + e.deltaY); render(); }, { passive: true });
  window.addEventListener("resize", render);
  // Self-update: when prompter.js changes on the server, reload so new settings
  // work. On 2026-10-15 an old prompter window silently ignored the new margin
  // and spacing sliders. Checked every 15 s while paused; never mid-read.
  let etag: string | null = null;
  async function codeTag() {
    return fetch("/prompter.js", { method: "HEAD", cache: "no-store" }).then(r => r.headers.get("etag")).catch(() => null);
  }
  void codeTag().then(t => { etag = t; });
  setInterval(async () => {
    if (playing) return;
    const t = await codeTag();
    if (t && etag && t !== etag) location.reload();
  }, 15000);
  render();
  chan.postMessage({ type: "hello" });
})();
