// Real WebKit interactions against a disposable server with six synthetic
// mobile-test-*.jpg fixtures. Refuses the live photo directory. Camera reads
// are mocked for lifecycle checks; camera commands are never sent.
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import { createRequire } from 'node:module';
import { execFileSync } from 'node:child_process';
const { webkit } = createRequire(import.meta.url)(process.env.Z8_PLAYWRIGHT || 'playwright');
const base = process.env.Z8_BASE || 'http://127.0.0.1:8422';
const api = async (path, body) => (await fetch(base + path, body ? {
  method: 'POST', headers: {'Content-Type':'application/json'}, body: JSON.stringify(body),
} : {})).json();
let initial = await api('/api/state');
assert.ok(initial.dir.startsWith('/tmp/z8-mobile-'), 'Use a disposable /tmp/z8-mobile-* photo root');
assert.equal(initial.connected, false, 'The test server must have no camera');
if (!initial.shots.length) {
  execFileSync('python3', ['-c', `
from PIL import Image, ImageDraw
from pathlib import Path
import sys
folder=Path(sys.argv[1])
for i in range(6):
    image=Image.new('RGB',(900,1350),(30+i*8,65,100))
    draw=ImageDraw.Draw(image)
    for x,c in enumerate(['#f07167','#fed9b7','#fdfcdc','#00afb9','#0081a7']):
        draw.rectangle((x*180,450,(x+1)*180,900),fill=c)
    draw.ellipse((270,160,630,520),fill=(190,140,110))
    draw.text((40,40),f'MOBILE TEST {i+1}',fill='white')
    image.save(folder/f'mobile-test-{i+1}.jpg',quality=92)
`, initial.dir]);
  initial = await api('/api/state');
}
const names = initial.shots.map(s => s.name);
assert.ok(names.length >= 3 && names.every(n => /^mobile-test-\d+\.jpg$/.test(n)), 'Only synthetic mobile fixtures are allowed');
for (const name of names) await api('/api/edits', {name, adjust:{exposure:1}, crop:null});
let passed = 0;
function check(name, value = true) { assert.ok(value, name); console.log('PASS ' + name); passed++; }
async function until(fn, timeout = 10000) {
  const end = Date.now() + timeout;
  while (Date.now() < end) { if (await fn()) return; await new Promise(r => setTimeout(r, 80)); }
  throw new Error('Timed out waiting for mobile workflow state');
}
const browser = await webkit.launch();
try {
  const context = await browser.newContext({viewport:{width:390,height:844},isMobile:true,hasTouch:true});
  const page = await context.newPage();
  const errors = [];
  page.on('pageerror', e => errors.push(e.message));
  await page.goto(base, {waitUntil:'networkidle'});
  await page.locator('#still[data-name]').waitFor();
  check('portrait photo loads in the phone stage');
  await page.locator('[data-tab="edit"]').tap();
  const first = await page.evaluate(() => {
    const imgs = [...document.querySelectorAll('#strip img')];
    imgs[0].click();
    const slider = document.querySelector('#exposure');
    slider.value='1.27'; slider.dispatchEvent(new Event('input'));
    imgs[1].click();
    return imgs[0].title;
  });
  await until(async () => (await api('/api/state')).edits[first]?.adjust?.exposure === 1.27);
  check('an immediate photo switch saves the edit to the original photo');
  await page.locator('#strip img').first().tap();
  await page.locator('[data-tab="edit"]').tap();
  await page.locator('[data-reset="exposure"]').tap();
  assert.equal(await page.locator('#exposure').inputValue(), '1');
  check('a slider value can be reset with one tap');
  await page.locator('#btnFocusCheck').tap();
  await page.waitForFunction(() => document.querySelector('#loupeImg').naturalWidth > 0);
  check('focus inspection opens with a tap');
  await page.locator('#loupeBar').tap();
  check('focus inspection closes with a tap', await page.locator('#loupe').isHidden());
  await page.locator('#btnCompare').dispatchEvent('touchstart');
  check('hold to compare shows the original', await page.locator('#stageBadge').isVisible());
  await page.locator('#btnCompare').dispatchEvent('touchcancel');
  check('an interrupted touch restores the edited image', await page.locator('#stageBadge').isHidden());

  await page.locator('#cropWrap').scrollIntoViewIfNeeded();
  const before = await page.locator('#cropInfo').innerText();
  const corner = await page.locator('#cropBox [data-h="se"]').boundingBox();
  await page.mouse.move(corner.x + corner.width / 2, corner.y + corner.height / 2);
  await page.mouse.down();
  await page.mouse.move(corner.x - 25, corner.y - 25, {steps:6});
  await page.mouse.up();
  check('crop handles resize the photo', (await page.locator('#cropInfo').innerText()) !== before);
  await until(async () => !!(await api('/api/state')).edits[first]?.crop);
  check('the resized crop is saved');
  await page.locator('[data-tab="export"]').tap();
  await page.locator('#exportPreset').selectOption('yt-thumb');
  await page.locator('#btnExport').tap();
  await page.locator('#downloadLinks a').first().waitFor();
  const downloading = page.waitForEvent('download');
  await page.locator('#downloadLinks a').first().tap();
  const download = await downloading;
  assert.equal(await download.failure(), null);
  const bytes = await fs.readFile(await download.path());
  check('a single export downloads in WebKit', bytes[0] === 255 && bytes[1] === 216);

  await page.locator('#btnSelectFrames').tap();
  await page.locator('#strip img').nth(0).tap();
  await page.locator('#strip img').nth(1).tap();
  await page.locator('[data-tab="export"]').tap();
  check('two frames can be selected without a keyboard', /2 selected/.test(await page.locator('#btnExport').innerText()));
  await page.locator('#btnExport').tap();
  await until(async () => await page.locator('#downloadLinks a').count() === 2);
  for (const link of await page.locator('#downloadLinks a').all()) {
    const r = await fetch(base + await link.getAttribute('href'));
    assert.equal(r.status, 200);
  }
  check('every selected export has a working phone download link');

  const selected = await page.locator('#strip img[data-on="1"]').getAttribute('title');
  await context.setOffline(true);
  await page.locator('[data-tab="edit"]').tap();
  await page.evaluate(() => {
    const slider=document.querySelector('#exposure');
    slider.value='1.34'; slider.dispatchEvent(new Event('input'));
  });
  await until(async () => (await page.title()).includes('offline'));
  check('Wi-Fi loss shows a useful reconnect message', /Wi-Fi and Tailscale/.test(await page.locator('#exportOut').innerText()));
  await context.setOffline(false);
  await page.evaluate(() => window.dispatchEvent(new Event('online')));
  await until(async () => await page.title() === 'Z8 Studio');
  await until(async () => (await api('/api/state')).edits[selected]?.adjust?.exposure === 1.34);
  check('edits made offline are saved after reconnecting');
  check('reconnection preserves the selected photo', await page.locator('#strip img[data-on="1"]').getAttribute('title') === selected);
  await page.locator('[data-tab="edit"]').tap();
  await page.locator('#btnCompare').scrollIntoViewIfNeeded();
  check('panel tabs stay reachable while editing', (await page.locator('.tabs').boundingBox()).y >= -1);

  await page.evaluate(() => {
    const slider = document.querySelector('#exposure');
    slider.value='1.42'; slider.dispatchEvent(new Event('input'));
    Object.defineProperty(document, 'hidden', {configurable:true, value:true});
    document.dispatchEvent(new Event('visibilitychange'));
  });
  await until(async () => (await api('/api/state')).edits[selected]?.adjust?.exposure === 1.42);
  check('switching apps flushes pending edits immediately');
  await page.evaluate(() => {
    Object.defineProperty(document, 'hidden', {configurable:true, value:false});
    document.dispatchEvent(new Event('visibilitychange'));
  });
  for (const [width,height] of [[320,568],[390,844],[430,932],[844,390]]) {
    await page.setViewportSize({width,height});
    for (const tab of ['camera','edit','export','library']) {
      await page.locator(`[data-tab="${tab}"]`).tap();
      const geometry = await page.evaluate(() => ({w:innerWidth,sw:document.documentElement.scrollWidth,
        short:[...document.querySelectorAll('button,select')].filter(e=>{const r=e.getBoundingClientRect();return r.width&&r.height&&r.height<43}).map(e=>e.id)}));
      assert.ok(geometry.sw <= geometry.w+1, JSON.stringify({width,tab,...geometry}));
      assert.deepEqual(geometry.short, [], `touch targets at ${width}, ${tab}`);
    }
    check(`populated panels fit at ${width}x${height}, with 44px tap controls`);
  }
  assert.deepEqual(errors, []);
  check('photo workflows have no browser exceptions');
  await context.close();

  // Camera/lifecycle simulation is confined to this context. The camera
  // stays disconnected on the server; no control command can escape it.
  const cameraContext = await browser.newContext({viewport:{width:390,height:844},isMobile:true,hasTouch:true});
  let previews = 0, hangNext = false, releaseHung;
  await cameraContext.route('**/api/state', async route => {
    if (hangNext) {
      hangNext=false;
      await new Promise(resolve => { releaseHung=resolve; });
      await route.abort().catch(()=>{}); return;
    }
    const response = await route.fetch();
    const state = await response.json();
    await route.fulfill({json:{...state,connected:true,present:true,manualOff:false,nextStep:null}});
  });
  await cameraContext.route('**/api/settings', route => route.fulfill({json:{
    iso:{value:'320',choices:['100','320','640'],readonly:false},
    whitebalance:{value:'Automatic',choices:['Automatic','Daylight'],readonly:false},
    battery:{value:'75%',choices:[],readonly:true},
  }}));
  await cameraContext.route('**/preview.mjpg?*', async route => {
    previews++;
    await route.fulfill({contentType:'image/jpeg',body:await fs.readFile(initial.dir+'/'+names[0])});
  });
  const cameraPage = await cameraContext.newPage();
  cameraPage.on('pageerror', e => errors.push(e.message));
  await cameraPage.goto(base,{waitUntil:'networkidle'});
  await cameraPage.locator('#camSettings input').first().waitFor();
  await cameraPage.locator('#camSettings input').first().dispatchEvent('pointerdown');
  await cameraPage.locator('#camSettings input').first().dispatchEvent('pointercancel');
  check('cancelled camera-slider touch releases its refresh lock', !(await cameraPage.locator('#camSettings').getAttribute('data-dragging')));
  const opened = previews;
  await cameraPage.evaluate(() => {
    Object.defineProperty(document,'hidden',{configurable:true,value:true});
    document.dispatchEvent(new Event('visibilitychange'));
  });
  check('live-view connection closes when the phone app is hidden', !(await cameraPage.locator('#live').getAttribute('src')));
  await cameraPage.evaluate(() => {
    Object.defineProperty(document,'hidden',{configurable:true,value:false});
    document.dispatchEvent(new Event('visibilitychange'));
  });
  await until(() => previews > opened);
  check('returning to the phone app opens a fresh live-view connection');
  const resumed = previews;
  await cameraPage.evaluate(() => dispatchEvent(new PageTransitionEvent('pageshow',{persisted:true})));
  await until(() => previews > resumed);
  check('restoring a suspended page restarts live view');
  hangNext = true;
  await cameraPage.evaluate(() => dispatchEvent(new Event('online')));
  await until(() => !!releaseHung);
  await until(async () => (await cameraPage.title()).includes('offline'), 11000);
  check('a stalled status request times out instead of freezing polling');
  releaseHung();
  await until(async () => await cameraPage.title() === 'Z8 Studio');
  check('status polling recovers after a stalled request');
  assert.deepEqual(errors, []);
  await cameraContext.close();
  console.log(`${passed} mobile workflow checks passed. Physical iOS lock-screen and Photos saving still need device testing.`);
} finally { await browser.close(); }
