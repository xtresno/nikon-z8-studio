#!/usr/bin/env python3
"""Real-image throughput in disposable state/photos. Never controls a camera.
Run through the workspace memory guard; provide the source photo root explicitly.
"""
import argparse, hashlib, json, os, pathlib, shutil, socket, statistics, subprocess, tempfile, time, urllib.request
from PIL import Image
p=argparse.ArgumentParser();p.add_argument('source_root',type=pathlib.Path);p.add_argument('--trials',type=int,default=3);p.add_argument('--out',type=pathlib.Path,required=True);p.add_argument('--workers',type=int,nargs='+',default=[1,3]);p.add_argument('--binary',type=pathlib.Path)
a=p.parse_args(); repo=pathlib.Path(__file__).resolve().parent
sources=sorted(f for day in a.source_root.iterdir() if day.is_dir() and len(day.name)==10 for f in day.iterdir() if f.suffix.lower()=='.jpg' and not f.name.endswith('.dev.jpg') and f.stat().st_size>10_000_000)[-20:]
assert len(sources)==20,'Needs 20 full-size JPEG source frames'
with tempfile.TemporaryDirectory(prefix='z8-benchmark-') as tmp:
 d=pathlib.Path(tmp); photos=d/'photos'; session=photos/time.strftime('%Y-%m-%d');session.mkdir(parents=True)
 shapes=[]
 for i,src in enumerate(sources):
  with Image.open(src) as im:shapes.append(im.size)
  shutil.copy2(src,session/f'benchmark-{i:02}.jpg')
 assert all(w*h>40_000_000 for w,h in shapes),'All benchmark frames must exceed 40 MP'
 for n in ['usb','proc']:(d/n).mkdir()
 result={'frames':len(sources),'dimensions':sorted(set(shapes)),'trials':a.trials,'workers':{}};truth={}
 for workers in a.workers:
  home=d/f'home-{workers}';home.mkdir();shutil.copy2(repo/'presets.json',home/'presets.json')
  sock=socket.socket();sock.bind(('127.0.0.1',0));port=sock.getsockname()[1];sock.close()
  env=dict(os.environ,Z8_PORT=str(port),Z8_HOME=str(home),Z8_WEB=str(repo/'web'),Z8_ROOT=str(photos),Z8_USB_ROOT=str(d/'usb'),Z8_PROC_ROOT=str(d/'proc'),Z8_NO_AUTOCONNECT='1',Z8_BATCH_WORKERS=str(workers))
  log=open(d/f'server-{workers}.log','w');proc=subprocess.Popen([str(a.binary or repo/'target/release/z8-studio')],env=env,stdout=log,stderr=log)
  base=f'http://127.0.0.1:{port}'
  def api(url,body=None):
   req=urllib.request.Request(base+url,data=json.dumps(body).encode() if body is not None else None,headers={'Content-Type':'application/json'})
   return json.load(urllib.request.urlopen(req,timeout=120))
  def job(route,body):
   t=time.perf_counter();r=api(route,body);assert r.get('ok'),r
   while True:
    j=api('/api/job?id='+r['job'])
    if j['state']!='running':break
    time.sleep(.025)
   elapsed=time.perf_counter()-t;assert j['state']=='done' and not j['errors'] and j['done']==20,j
   return elapsed,j
  try:
   for _ in range(100):
    try:api('/api/state');break
    except OSError:time.sleep(.1)
   pres=api('/api/presets');preset=next(x for x in pres['export'] if x['id']=='headshot-8x10');look=next(x for x in pres['look'] if x['id']=='clean-headshot')
   measured={k:[] for k in ['export20','rank20','auto20']}
   status=[]
   for _ in range(40):
    t=time.perf_counter();s=api('/api/state');assert len(s['shots'])==20;status.append((time.perf_counter()-t)*1000)
   for trial in range(a.trials):
    (session/'edits.json').unlink(missing_ok=True)
    t,j=job('/api/export-all',{'preset':preset,'adjust':look});measured['export20'].append(t)
    hashes={x['name']:hashlib.sha256(pathlib.Path(x['path']).read_bytes()).hexdigest() for x in j['results']}
    if 'export' not in truth:truth['export']=hashes
    assert hashes==truth['export'],'Export pixels differ between runs/workers'
    t,j=job('/api/rank',{});measured['rank20'].append(t)
    rank=sorted(j.get('ranked',[]),key=lambda x:x['name'])
    if 'rank' not in truth:truth['rank']=rank
    assert rank==truth['rank'],'Rank results changed'
    t,j=job('/api/auto-look-all',{});measured['auto20'].append(t)
    edits=json.loads((session/'edits.json').read_text())
    if 'edits' not in truth:truth['edits']=edits
    assert edits==truth['edits'],'Auto grades changed'
    print(f'workers={workers} trial={trial+1}: '+', '.join(f'{k}={v[-1]:.3f}s' for k,v in measured.items()),flush=True)
   rss={line.split(':')[0]:line.split(':')[1].strip() for line in pathlib.Path(f'/proc/{proc.pid}/status').read_text().splitlines() if line.startswith(('VmRSS:','VmHWM:'))}
   result['workers'][str(workers)]={'seconds':measured,'median_seconds':{k:round(statistics.median(v),3) for k,v in measured.items()},'state_ms_median':round(statistics.median(status),3),'state_ms_p95':round(sorted(status)[37],3),'memory':rss}
  finally:
   proc.terminate()
   try:proc.wait(timeout=10)
   except subprocess.TimeoutExpired:proc.kill();proc.wait()
   log.close()
 result['export_bytes_identical']=True;result['auto_and_rank_identical']=True
 result['export_digest']=hashlib.sha256(json.dumps(truth['export'],sort_keys=True).encode()).hexdigest()
 result['auto_digest']=hashlib.sha256(json.dumps(truth['edits'],sort_keys=True).encode()).hexdigest()
 result['rank_digest']=hashlib.sha256(json.dumps(truth['rank'],sort_keys=True).encode()).hexdigest()
 a.out.write_text(json.dumps(result,indent=2)+'\n');print(json.dumps(result,indent=2))
