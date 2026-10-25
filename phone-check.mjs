// Run against an isolated server. No camera commands or photo writes are sent.
// Z8_PLAYWRIGHT may point to an existing Playwright installation.
// Z8_BASE defaults to the documented sandbox port, never the live app.
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import http from 'node:http';
import { createRequire } from 'node:module';
const require = createRequire(import.meta.url);
const { webkit, chromium } = require(process.env.Z8_PLAYWRIGHT || 'playwright');
const base = process.env.Z8_BASE || 'http://127.0.0.1:8421';
const out = process.env.Z8_PHONE_SHOTS;
if (out) await fs.mkdir(out, { recursive: true });

// An unknown POST route proves the request passed the origin guard without
// invoking a camera action or changing application state.
if (process.env.Z8_REMOTE_ORIGIN) {
  const remote = new URL(process.env.Z8_REMOTE_ORIGIN);
  // fetch/Undici rewrites Host, so use raw HTTP for Host guard coverage.
  const status = (path, method, headers) => new Promise((resolve, reject) => {
    const req = http.request(new URL(path, base), { method, headers }, response => {
      response.resume();
      resolve(response.statusCode);
    });
    req.on('error', reject);
    req.end(method === 'POST' ? '{}' : undefined);
  });
  for (const [origin, expected] of [[remote.origin, 404], ['https://unrelated.invalid', 403], ['null', 403]]) {
    assert.equal(await status('/api/phone-check-does-not-exist', 'POST',
      { Host: remote.host, Origin: origin }), expected, `remote origin ${origin}`);
  }
  assert.equal(await status('/', 'GET', { Host: remote.host }), 200, 'private proxy Host');
  assert.equal(await status('/', 'GET', { Host: 'unrelated.invalid' }), 403, 'foreign Host');
}

const manifestResponse = await fetch(base + '/manifest.webmanifest');
assert.equal(manifestResponse.status, 200);
assert.match(manifestResponse.headers.get('content-type'), /application\/manifest\+json/);
const manifest = await manifestResponse.json();
assert.equal(manifest.display, 'standalone');
for (const path of ['/apple-touch-icon.png', ...manifest.icons.map(i => i.src)]) {
  const response = await fetch(base + path);
  assert.equal(response.status, 200, path);
  assert.equal(response.headers.get('content-type'), 'image/png');
  assert.deepEqual([...new Uint8Array(await response.arrayBuffer()).slice(0, 8)], [137,80,78,71,13,10,26,10]);
}

for (const [engine, name, sizes] of [
  [webkit, 'webkit', [[320, 568], [390, 844], [430, 932], [844, 390]]],
  [chromium, 'chromium', [[1500, 950]]],
]) {
  const browser = await engine.launch({ headless: true });
  try {
    for (const [width, height] of sizes) {
      const context = await browser.newContext({ viewport: { width, height },
        isMobile: name === 'webkit', hasTouch: name === 'webkit', deviceScaleFactor: 1 });
      const page = await context.newPage();
      const errors = [];
      page.on('pageerror', e => errors.push(e.message));
      await page.goto(base, { waitUntil: 'networkidle' });
      for (const tab of ['camera', 'edit', 'export', 'library']) {
        await page.locator(`[data-tab="${tab}"]`).click();
        const metrics = await page.evaluate(() => ({
          width: document.documentElement.clientWidth,
          scrollWidth: document.documentElement.scrollWidth,
          outside: [...document.querySelectorAll('button,select,input')].filter(e => {
            const r = e.getBoundingClientRect();
            return r.width && r.height && (r.left < -1 || r.right > innerWidth + 1);
          }).map(e => e.id || e.tagName),
        }));
        assert.ok(metrics.scrollWidth <= metrics.width + 1, `${name} ${width} ${tab}: ${JSON.stringify(metrics)}`);
        assert.deepEqual(metrics.outside, [], `${name} ${width} ${tab}`);
      }
      await page.locator('[data-tab="camera"]').click();
      await page.locator('#btnCamDefault').waitFor({ state: 'visible' });
      if (width <= 600) {
        assert.ok((await page.locator('#btnShoot').boundingBox()).height >= 44);
        assert.ok((await page.locator('[data-tab="camera"]').boundingBox()).height >= 44);
      }
      assert.deepEqual(errors, [], `${name} ${width}: browser exceptions`);
      if (out) {
        await page.evaluate(() => scrollTo(0, 0));
        await page.screenshot({ path: `${out}/${name}-${width}.png`, fullPage: true });
      }
      console.log(`${name} ${width}x${height}: all four tabs fit, no browser exceptions`);
      await context.close();
    }
  } finally { await browser.close(); }
}
console.log('Phone assets and layout checks passed. Physical iPhone testing remains manual.');
