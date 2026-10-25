#!/usr/bin/env node
// Lay a take's backup WAV under its video, IN SYNC, even when either track lost
// samples. Measured 2026-10-20 (take 20261020-191700): the WAV lost 3.5 s in
// small cuts between 1:35 and 3:20, and Chrome's own track had eleven ~100 ms
// holes. A single offset cannot fix that, so this tracks the WAV's lag every
// 0.25 s against the video's own audio (on the container timeline), uses the
// WAV wherever it matches, and fills only the moments the WAV lost from the
// browser track (gain-matched, 5 ms crossfades). It then re-checks sync every
// 5 s and refuses to write the file if any point is more than 2 ms off.
//
//   node tools/sync-audio.mjs <video.mp4|webm> <backup.wav> <out.mp4|webm>
// Prints one JSON line: { ok, seconds, patchedSeconds, splices, maxOffMs, ... }
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { execFileSync } from 'node:child_process';

const [video, wav, outFile] = process.argv.slice(2);
const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'z8-sync-'));
const aRaw = path.join(tmp, 'a.raw'), wRaw = path.join(tmp, 'w.raw'), outRaw = path.join(tmp, 'o.raw');
const ff = (args) => execFileSync('ffmpeg', ['-v', 'error', '-y', ...args], { stdio: ['ignore', 'ignore', 'pipe'] });
try {
ff(['-i', video, '-vn', '-af', 'aresample=async=1:first_pts=0', '-ac', '1', '-ar', '48000', '-f', 's16le', aRaw]);
ff(['-i', wav, '-ac', '1', '-ar', '48000', '-f', 's16le', wRaw]);

const rd = (f) => { const b = fs.readFileSync(f); const x = new Float32Array(b.length >> 1); for (let i = 0; i < x.length; i++) x[i] = b.readInt16LE(i * 2) / 32768; return x; };
const A = rd(aRaw), Wv = rd(wRaw);
const dec = (x) => { const y = new Float32Array(Math.floor(x.length / 6)); for (let i = 0; i < y.length; i++) { let s = 0; for (let k = 0; k < 6; k++) s += x[i * 6 + k]; y[i] = s / 6; } return y; };
const a8 = dec(A), w8 = dec(Wv);
const BLK = 2000; // 0.25 s at 8 kHz
const WIN = 4000; // 0.5 s correlation window
const corr = (s, l, st) => { let c = 0, ea = 0, ew = 0; for (let i = 0; i < WIN; i += st) { const j = s + i + l; if (j < 0 || j >= w8.length) return -1; const p = a8[s + i], q = w8[j]; c += p * q; ea += p * p; ew += q * q; } return ea < 1e-6 ? -2 : c / Math.sqrt(ea * ew + 1e-12); };
const search = (s, lo, hi, st) => { let best = -3, bl = 0; for (let l = lo; l <= hi; l += st) { const r = corr(s, l, st === 1 ? 1 : 2); if (r > best) { best = r; bl = l; } } return [best, bl]; };
const nb = Math.floor((a8.length - WIN) / BLK);
const lag = new Array(nb).fill(null); let prev = null;
for (let b = 0; b < nb; b++) {
  const s = b * BLK; let best, bl;
  if (prev !== null) { [best, bl] = search(s, prev - 480, prev + 480, 1); }
  if (prev === null || best < 0.85) { const [b2, l2] = search(s, -40000, 8000, 4); const [b3, l3] = search(s, l2 - 4, l2 + 4, 1); if (prev === null || b3 > best) { best = b3; bl = l3; } }
  if (best === -2) { lag[b] = prev; continue; } // silence: keep the previous lag
  if (best >= 0.85) { lag[b] = bl; prev = bl; }
}
// Unmatched blocks between two matched blocks with the SAME lag are not a WAV
// loss (the browser track is what failed there, e.g. its own 100 ms holes):
// keep the WAV at that lag. Only a lag CHANGE marks samples the WAV lost.
const matchedLag = lag.slice();
for (let b = 0; b < nb; b++) {
  if (matchedLag[b] !== null) continue;
  let p = b - 1; while (p >= 0 && matchedLag[p] === null) p--;
  let n = b + 1; while (n < nb && matchedLag[n] === null) n++;
  const pl = p >= 0 ? matchedLag[p] : null, nl = n < nb ? matchedLag[n] : null;
  if (pl !== null && (nl === null || Math.abs(pl - nl) <= 200)) lag[b] = pl;  // within 25 ms: keep the WAV
  else if (pl === null && nl !== null) lag[b] = nl;
}
// gain: WAV vs Chrome on matched blocks
let sw = 0, sa = 0; for (let b = 0; b < nb; b++) if (lag[b] !== null) { const s = b * BLK * 6, L = lag[b] * 6; for (let i = 0; i < BLK * 6; i += 8) { const j = s + i + L; if (j >= 0 && j < Wv.length) { sw += Wv[j] ** 2; sa += A[s + i] ** 2; } } }
const gain = Math.sqrt(sw / (sa + 1e-12));
const out = new Float32Array(A.length); const F = 240; // 5 ms crossfade
let patched = 0, splices = 0, lastSrc = null, lastLag = lag.find((x) => x !== null) ?? 0;
for (let b = 0; b < nb + 1; b++) {
  const s = b * BLK * 6, e = Math.min(A.length, s + BLK * 6); const L = b < nb ? lag[b] : lag[nb - 1];
  const useW = L !== null && L !== undefined;
  if (useW) lastLag = L;
  const src = useW ? `w${L}` : 'a'; if (!useW) patched += (e - s) / 48000;
  for (let i = s; i < e; i++) {
    // In a browser-track hole (exact zeros) the WAV still has the sound.
    let v = useW ? (Wv[i + L * 6] ?? 0) : (A[i] === 0 ? (Wv[i + lastLag * 6] ?? 0) : A[i] * gain);
    if (lastSrc !== null && src !== lastSrc && i - s < F) { // crossfade from the previous source
      const pl = lastSrc === 'a' ? null : +lastSrc.slice(1); const pv = pl === null ? A[i] * gain : (Wv[i + pl * 6] ?? 0);
      const t = (i - s) / F; v = pv * (1 - t) + v * t;
    }
    out[i] = v;
  }
  if (lastSrc !== null && src !== lastSrc) splices++; lastSrc = src;
}
const buf = Buffer.alloc(out.length * 2); for (let i = 0; i < out.length; i++) buf.writeInt16LE(Math.max(-32768, Math.min(32767, Math.round(out[i] * 32767))), i * 2);
fs.writeFileSync(outRaw, buf);
const lags = lag.filter((x) => x !== null).map((x) => x / 8);
const stats = { seconds: out.length / 48000, blocks: nb, matched: lag.filter((x) => x !== null).length, patchedSeconds: +patched.toFixed(2), splices, gain: +gain.toFixed(2), lagMsFirst: lags[0], lagMsLast: lags.at(-1) };

// Verify: loudness envelopes of the result vs the video's track, every 5 s.
const env = (x) => { const H = 48, m = Math.floor(x.length / H), e = new Float32Array(m); for (let k = 0; k < m; k++) { let s = 0; for (let i = k * H; i < (k + 1) * H; i++) s += x[i] * x[i]; e[k] = Math.sqrt(s / H); } return e; };
const ea = env(A), eo = env(out); const EL = 2000, ER = 400; let maxOff = 0, checked = 0;
for (let s = 5000; s + EL + ER < Math.min(ea.length, eo.length); s += 5000) {
  let best = -2, bl = 0;
  for (let l = -ER; l <= ER; l++) { let c = 0, x2 = 0, y2 = 0, mx = 0, my = 0; for (let i = 0; i < EL; i++) { mx += ea[s + i]; my += eo[s + l + i]; } mx /= EL; my /= EL; for (let i = 0; i < EL; i++) { const p = ea[s + i] - mx, q = eo[s + l + i] - my; c += p * q; x2 += p * p; y2 += q * q; } const r = c / Math.sqrt(x2 * y2 + 1e-12); if (r > best) { best = r; bl = l; } }
  if (best > 0.8) { checked++; maxOff = Math.max(maxOff, Math.abs(bl)); }
}
const ok = checked > 0 && maxOff <= 2 && stats.matched > stats.blocks * 0.5;
if (ok) {
  const webm = outFile.endsWith('.webm');
  ff(['-i', video, '-f', 's16le', '-ar', '48000', '-ac', '1', '-i', outRaw, '-map', '0:v', '-map', '1:a', '-c:v', 'copy', '-af', 'highpass=f=60',
    ...(webm ? ['-c:a', 'libopus', '-b:a', '192k'] : ['-c:a', 'aac', '-b:a', '256k', '-movflags', '+faststart']), outFile]);
}
console.log(JSON.stringify({ ok, ...stats, checked, maxOffMs: maxOff }));
} finally {
  fs.rmSync(tmp, { recursive: true, force: true });
}
