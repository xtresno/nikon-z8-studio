// Run the page's own interaction checks against real server time. Virtual
// browser time can outrun image workers and misreport a working Cancel button.
import {createRequire} from 'node:module';
import os from 'node:os';
import path from 'node:path';
const require = createRequire(import.meta.url);
let playwright;
for (const candidate of [process.env.Z8_PLAYWRIGHT, 'playwright',
  path.join(os.homedir(), '.local/lib/node_modules/playwright')].filter(Boolean)) {
  try { playwright = require(candidate); break; } catch { /* try next */ }
}
if (!playwright) throw new Error('Set Z8_PLAYWRIGHT to the Playwright module directory');
const base = process.env.Z8_BASE || 'http://127.0.0.1:8420';
const state = await (await fetch(base + '/api/state')).json();
if (state.session !== '1998-12-31') throw new Error('Run through selftest.py in its disposable session');
const browser = await playwright.chromium.launch();
try {
  const page = await browser.newPage({viewport: {width: 1500, height: 950}});
  await page.goto(base + '/?e2e=1', {waitUntil: 'domcontentloaded'});
  await page.waitForFunction(() => document.getElementById('e2e')?.textContent.includes('E2E-END'),
    null, {timeout: 240000});
  const output = await page.locator('#e2e').innerText();
  console.log(output);
  if (output.includes('\nFAIL ')) process.exitCode = 1;
} finally { await browser.close(); }
