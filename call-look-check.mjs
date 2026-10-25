// End-to-end: browser -> HTTP -> real keeper -> I420 pixels. All state,
// USB/proc trees, MJPEG input, and output live in a disposable sandbox.
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import http from 'node:http';
import net from 'node:net';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { createRequire } from 'node:module';
import { spawn, execFileSync } from 'node:child_process';
import { once } from 'node:events';
const require = createRequire(import.meta.url);
const { webkit, chromium } = require(process.env.Z8_PLAYWRIGHT || 'playwright');
const repo = path.dirname(fileURLToPath(import.meta.url));
const dir = await fs.mkdtemp('/tmp/z8-call-look-');
const ctrl = path.join(dir, 'ctrl.json'), status = path.join(dir, 'status.json');
const frame = path.join(dir, 'frame.i420');
const sleep = ms => new Promise(r => setTimeout(r, ms));
const json = async p => JSON.parse(await fs.readFile(p, 'utf8'));
const write = async (p, v) => { await fs.writeFile(p + '.test-tmp', JSON.stringify(v)); await fs.rename(p + '.test-tmp', p); };
const children = [], sockets = new Set();
let browser, camera, checks = 0;
function check(name, value) { assert.ok(value, name); checks++; console.log('PASS ' + name); }
async function until(fn, label, timeout = 10000) {
  const end = Date.now() + timeout;
  while (Date.now() < end) { try { const v = await fn(); if (v) return v; } catch {} await sleep(60); }
  throw new Error('Timed out: ' + label);
}
function start(bin, env) {
  const child = spawn(bin, [], {env: {...process.env, ...env}, stdio: ['ignore', 'pipe', 'pipe']});
  child.output = ''; child.stdout.on('data', b => child.output += b); child.stderr.on('data', b => child.output += b);
  children.push(child); return child;
}
async function freePort() {
  const server = net.createServer(); server.listen(0, '127.0.0.1'); await once(server, 'listening');
  const port = server.address().port; await new Promise(r => server.close(r)); return port;
}
try {
  for (const name of ['photos', 'usb', 'proc']) await fs.mkdir(path.join(dir, name));
  await fs.copyFile(path.join(repo, 'presets.json'), path.join(dir, 'presets.json'));
  await write(path.join(dir, 'config.json'), {webcam_look_id: ''});
  await write(ctrl, {live: false, look: null}); await fs.writeFile(frame, '');
  const jpg = execFileSync('python3', ['-c', 'from PIL import Image; import sys; Image.new("RGB", (1024,576), (120,90,70)).save(sys.stdout.buffer, "JPEG")']);
  camera = http.createServer((req, res) => {
    res.writeHead(200, {'Content-Type': 'multipart/x-mixed-replace; boundary=z8test'});
    const send = () => res.write(Buffer.concat([Buffer.from(`--z8test\r\nContent-Type: image/jpeg\r\nContent-Length: ${jpg.length}\r\n\r\n`), jpg, Buffer.from('\r\n')]));
    const timer = setInterval(send, 50); send(); res.on('close', () => clearInterval(timer));
  });
  camera.on('connection', s => { sockets.add(s); s.on('close', () => sockets.delete(s)); });
  camera.listen(0, '127.0.0.1'); await once(camera, 'listening');
  const port = await freePort(), base = `http://127.0.0.1:${port}`;
  const env = {Z8_HOME: dir, Z8_ROOT: path.join(dir, 'photos'), Z8_USB_ROOT: path.join(dir, 'usb'),
    Z8_PROC_ROOT: path.join(dir, 'proc'), Z8_NO_AUTOCONNECT: '1', Z8_PORT: String(port),
    Z8_WEB: process.env.Z8_TEST_WEB || path.join(repo, 'web'), Z8_FEED_CTRL: ctrl, Z8_FEED_STATUS: status};
  const server = start(process.env.Z8_TEST_SERVER || path.join(repo, 'target/release/z8-studio'), env);
  await until(async () => (await fetch(base + '/api/state')).ok, 'isolated server');
  const keeper = start(process.env.Z8_TEST_KEEPER || path.join(repo, 'target/release/z8-feed'), {...env,
    PORT: String(camera.address().port), Z8_FEED_DEV: frame, Z8_FEED_OUTFMT: 'file', Z8_MTPLVCAP: '/does-not-exist'});
  await until(async () => (await json(status)).state === 'idle', 'isolated keeper');
  const post = async body => (await fetch(base + '/api/call-look', {method: 'POST', headers: {'Content-Type': 'application/json'}, body: JSON.stringify(body)})).json();
  const pixels = () => fs.readFile(frame);
  // Set the sandbox's feed live directly: this test never sends mode/camera commands.
  await write(ctrl, {live: true, look: null});
  await until(async () => (await json(status)).state === 'live', 'synthetic stream');
  const neutral = await pixels(), before = await json(status);
  assert.equal(neutral.length, 1024 * 576 * 3 / 2);
  for (const [engine, name, width] of [[webkit, 'iPhone WebKit', 390], [chromium, 'desktop Chromium', 1500]]) {
    browser = await engine.launch();
    const page = await browser.newPage({viewport: {width, height: 950}, isMobile: width < 500, hasTouch: width < 500});
    const errors = [], writes = [];
    page.on('pageerror', e => errors.push(e.message));
    page.on('request', r => { if (r.method() === 'POST') writes.push(new URL(r.url()).pathname); });
    await page.goto(base, {waitUntil: 'networkidle'});
    const select = page.locator('#callLook');
    await until(async () => (await select.locator('option').count()) > 2, 'presets');
    await select.selectOption('moody');
    await until(async () => !await select.isDisabled(), 'look save');
    await until(async () => !(await pixels()).equals(neutral), name + ' changes actual output pixels');
    check(name + ': choosing a look changes the running feed', (await json(ctrl)).look?.exposure !== undefined);
    const graded = await pixels();
    await select.selectOption('');
    await until(async () => !await select.isDisabled(), 'no look save');
    await until(async () => (await pixels()).equals(neutral), 'no look restores exact original pixels');
    check(name + ': no look removes the grade', !graded.equals(neutral) && (await json(ctrl)).look === null);
    await select.selectOption('sliders');
    await until(async () => !await select.isDisabled(), 'slider mode');
    await page.locator('[data-tab="edit"]').click();
    await page.locator('#exposure').evaluate(e => { e.value = '1.5'; e.dispatchEvent(new Event('input', {bubbles: true})); });
    await until(async () => (await json(ctrl)).look?.exposure === 1.5, 'follow sliders control');
    await until(async () => !(await pixels()).equals(neutral), 'follow sliders pixels');
    check(name + ': follow sliders updates the live pixels', true);
    await select.selectOption('');
    await until(async () => !await select.isDisabled(), 'restore no look');
    await until(async () => (await pixels()).equals(neutral), 'restore neutral');
    await page.locator('#exposure').evaluate(e => { e.value = '1.8'; e.dispatchEvent(new Event('input', {bubbles: true})); });
    await sleep(700);
    check(name + ': photo edits do not override no look', (await json(ctrl)).look === null && (await pixels()).equals(neutral));
    check(name + ': no mode switch or hardware write', writes.length > 0 && writes.every(p => p === '/api/call-look'));
    check(name + ': scope is visible and layout fits', (await page.locator('#callLookNote').innerText()).includes('Zoom/OBS') && await page.evaluate(() => document.documentElement.scrollWidth === innerWidth));
    if (process.env.Z8_CALL_SHOTS) {
      await fs.mkdir(process.env.Z8_CALL_SHOTS, {recursive: true});
      await page.screenshot({path: path.join(process.env.Z8_CALL_SHOTS, name.replaceAll(' ', '-') + '.png'), fullPage: true});
    }
    // A refused save must not silently leave the dropdown claiming success.
    await page.route('**/api/call-look', r => r.fulfill({status: 400, contentType: 'application/json', body: JSON.stringify({ok: false, error: 'test refusal'})}));
    await select.selectOption('moody'); await until(async () => !await select.isDisabled(), 'refused save');
    check(name + ': failed save restores selection and reports error', await select.inputValue() === '' && (await page.locator('#exportOut').innerText()).includes('test refusal'));
    check(name + ': no JavaScript errors', errors.length === 0);
    await browser.close(); browser = null;
  }
  const after = await json(status);
  check('keeper process, link and writer survive all look changes', before.pid === after.pid && before.linkRestarts === after.linkRestarts && before.writerRestarts === after.writerRestarts && after.frames > before.frames);
  const marker = await json(path.join(dir, '.last-mode.json'));
  check('restart state keeps no look', marker.mode === 'webcam' && marker.look === null);
  check('unknown looks are refused', (await post({id: 'missing-preset'})).ok === false);
  check('sliders require actual values', (await post({id: 'sliders'})).ok === false);
  await write(ctrl, {live: false, look: null});
  await until(async () => (await json(status)).state === 'idle', 'idle');
  const saved = await post({id: 'moody'});
  check('inactive selection is saved without starting the camera', saved.ok && !saved.live && !(await json(ctrl)).live && (await json(path.join(dir, 'config.json'))).webcam_look_id === 'moody');
  console.log(`${checks} checks passed; synthetic input only.`);
} catch (e) {
  for (const child of children) if (child.exitCode !== null) console.error(child.output);
  throw e;
} finally {
  await browser?.close();
  for (const child of children.reverse()) {
    if (child.exitCode === null) { child.kill('SIGTERM'); await Promise.race([once(child, 'exit'), sleep(7000)]); }
  }
  for (const socket of sockets) socket.destroy();
  camera?.close();
  await fs.rm(dir, {recursive: true, force: true});
}
