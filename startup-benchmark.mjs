// Compare compiled bundles on the same isolated server in iPhone WebKit.
// Required: Z8_BEFORE_JS. Optional: Z8_AFTER_JS, Z8_BASE, Z8_PLAYWRIGHT,
// Z8_BENCH_OUT, Z8_STATE_LATENCY_MS (default 150). No camera commands.
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import {createRequire} from 'node:module';
import {fileURLToPath} from 'node:url';
const {webkit}=createRequire(import.meta.url)(process.env.Z8_PLAYWRIGHT || 'playwright');
const base=process.env.Z8_BASE || 'http://127.0.0.1:8471';
const state=await (await fetch(base+'/api/state')).json();
assert(state.dir.startsWith('/tmp/z8-') && !state.connected && !state.webcam,'Use an isolated server');
assert(process.env.Z8_BEFORE_JS,'Set Z8_BEFORE_JS to the previous compiled app.js');
const latency=Number(process.env.Z8_STATE_LATENCY_MS || 150);
const bundles=[['before',process.env.Z8_BEFORE_JS],['after',process.env.Z8_AFTER_JS || fileURLToPath(new URL('web/dist/app.js',import.meta.url))]];
const browser=await webkit.launch();const rows={before:[],after:[],simulatedStatusLatencyMs:latency};
try {
 for(let run=0;run<5;run++)for(const [tag,source]of bundles){
  const context=await browser.newContext({viewport:{width:390,height:844},isMobile:true,hasTouch:true});
  const page=await context.newPage();let count=0;const script=await fs.readFile(source,'utf8');
  await page.route('**/app.js',r=>r.fulfill({contentType:'application/javascript',body:script}));
  await page.route('**/api/state',async r=>{count++;const response=await r.fetch();await new Promise(x=>setTimeout(x,latency));await r.fulfill({response});});
  const t=Date.now();await page.goto(base+'/?layout=1',{waitUntil:'domcontentloaded'});
  await page.waitForFunction(()=>document.getElementById('cfgNote').textContent.startsWith('exports go to'));
  rows[tag].push({ms:Date.now()-t,statusRequests:count});await context.close();
 }
} finally {await browser.close();}
console.log(JSON.stringify(rows,null,2));
if(process.env.Z8_BENCH_OUT)await fs.writeFile(process.env.Z8_BENCH_OUT,JSON.stringify(rows,null,2)+'\n');
