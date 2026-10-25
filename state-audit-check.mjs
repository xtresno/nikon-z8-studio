// Browser and HTTP regressions in a disposable photo root; no camera commands.
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import path from 'node:path';
import {createRequire} from 'node:module';
import {execFileSync} from 'node:child_process';
const {webkit}=createRequire(import.meta.url)(process.env.Z8_PLAYWRIGHT || 'playwright');
const base=process.env.Z8_BASE || 'http://127.0.0.1:8471';
const api=async(p,b)=>(await fetch(base+p,b ? {method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify(b)} : {})).json();
const initial=await api('/api/state');
assert(initial.dir.startsWith('/tmp/z8-mobile-') && !initial.connected && !initial.webcam);
const root=path.dirname(initial.dir), dates=['1999-04-01','1999-04-02','1999-04-03'];
const sleep=ms=>new Promise(r=>setTimeout(r,ms));
async function until(fn,ms=5000){const end=Date.now()+ms;while(Date.now()<end){if(await fn())return true;await sleep(80);}return false;}
let failed=0,passed=0;
function check(name,ok){console.log((ok?'PASS ':'FAIL ')+name);ok?passed++:failed++;}
for(const date of dates){
 await fs.mkdir(path.join(root,date),{recursive:true});
 execFileSync('python3',['-c','from PIL import Image; import sys; Image.new("RGB",(1000,700),(110,85,65)).save(sys.argv[1],"JPEG")',path.join(root,date,'state-test.jpg')]);
}
const edits=async date=>{try{return JSON.parse(await fs.readFile(path.join(root,date,'edits.json'),'utf8'));}catch{return {};}};
const browser=await webkit.launch();
try{
 await api('/api/session',{date:dates[0]});
 await api('/api/call-look',{id:''});
 const context=await browser.newContext({viewport:{width:390,height:844},isMobile:true,hasTouch:true});
 const one=await context.newPage(),two=await context.newPage(),errors=[];
 for(const page of [one,two]){page.on('pageerror',e=>errors.push(e.message));await page.goto(base,{waitUntil:'networkidle'});}
 await one.locator('[data-tab="edit"]').click();
 await one.locator('#lookPreset').selectOption('moody');
 const wanted=Number(await one.locator('#exposure').inputValue());
 check('choosing a photo Look persists without moving a slider',await until(async()=>(await edits(dates[0]))['state-test.jpg']?.adjust?.exposure===wanted));
 await one.locator('#callLook').selectOption('moody');
 await until(async()=>!(await one.locator('#callLook').isDisabled()));
 check('the other page follows the selected Call look',await until(async()=>await two.locator('#callLook').inputValue()==='moody'));
 const result=await api('/api/call-look',{id:'sliders',follow:true,look:{exposure:1.8}});
 check('a stale slider-following page cannot override a named Call look',result.ok && (await api('/api/state')).config.webcam_look_id==='moody');
 // Keep a failed edit queued while another client changes the shared session.
 await one.locator('[data-tab="edit"]').click();
 let attempted=false;
 await one.route('**/api/edits',r=>{attempted=true;return r.abort('failed');});
 await one.locator('#exposure').evaluate(e=>{e.value='1.42';e.dispatchEvent(new Event('input',{bubbles:true}));});
 assert(await until(()=>attempted));
 await api('/api/session',{date:dates[1]});
 await one.unroute('**/api/edits');
 await one.evaluate(()=>window.dispatchEvent(new Event('online')));
 check('a queued edit stays with its original session after another device switches days',await until(async()=>(await edits(dates[0]))['state-test.jpg']?.adjust?.exposure===1.42));
 check('the other session does not receive the queued edit',!(await edits(dates[1]))['state-test.jpg']);
 // Losing an iPhone page must not lose an edit that was waiting for Wi-Fi.
 await api('/api/session',{date:dates[0]});await one.reload({waitUntil:'networkidle'});
 await one.locator('[data-tab="edit"]').click();
 await one.route('**/api/edits',r=>r.abort('failed'));
 await one.locator('#exposure').evaluate(e=>{e.value='1.57';e.dispatchEvent(new Event('input',{bubbles:true}));});
 await sleep(800);await one.reload({waitUntil:'networkidle'});
 check('an unsaved edit is recovered after reloading the page',Number(await one.locator('#exposure').inputValue())===1.57);
 await one.unroute('**/api/edits');await one.evaluate(()=>window.dispatchEvent(new Event('online')));
 check('the recovered edit saves on reconnection',await until(async()=>(await edits(dates[0]))['state-test.jpg']?.adjust?.exposure===1.57));
 // Failure to rename onto a directory deterministically simulates a sidecar
 // write failure even when the test user can bypass chmod permissions.
 const bad=path.join(root,dates[2],'edits.json');await fs.mkdir(bad);
 await api('/api/session',{date:dates[2]});
 const response=await fetch(base+'/api/edits',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({session:dates[2],name:'state-test.jpg',adjust:{exposure:1.3}})});
 const body=await response.json();
 check('a failed edit write returns an error instead of claiming it saved',response.status>=400 && body.ok===false && !!body.error);
 check('failed saves leave no temporary sidecars',(await fs.readdir(path.join(root,dates[2]))).every(n=>!n.endsWith('.tmp')));
 check('invalid session paths are refused',(await api('/api/edits',{session:'../escape',name:'state-test.jpg',adjust:{exposure:1}})).ok===false);
 // Every slider's tap-to-reset value must have a working handler.
 await api('/api/session',{date:dates[0]});await one.reload({waitUntil:'networkidle'});
 await one.locator('[data-tab="edit"]').click();
 for(const knob of ['exposure','shadows','highlights','contrast','saturation','warmth','tint','vignette','sharpness','straighten']){
  const neutral=['exposure','contrast','saturation','sharpness'].includes(knob)?1:0;
  await one.locator('#'+knob).evaluate(e=>{e.value=String(Number(e.value)+.1);e.dispatchEvent(new Event('input',{bubbles:true}));});
  await one.locator(`[data-reset="${knob}"]`).click();
  check(knob+' tap-to-reset works',Number(await one.locator('#'+knob).inputValue())===neutral);
 }
 check('both pages have no JavaScript exceptions',!errors.length);
}finally{
 await browser.close();
 await api('/api/session',{date:initial.session});
 await api('/api/call-look',{id:initial.config.webcam_look_id});
 for(const date of dates)await fs.rm(path.join(root,date),{recursive:true,force:true});
}
console.log(`${passed} passed, ${failed} failed`);process.exitCode=failed?1:0;
