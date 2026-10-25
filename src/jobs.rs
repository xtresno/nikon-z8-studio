//! Background jobs: exports, Auto, ranking, trims, imports. A job ALWAYS
//! reaches a terminal state, ids never collide, and a running job is never
//! evicted from under the page polling it.
use std::collections::HashMap;

use parking_lot::Mutex;
use serde_json::{json, Map, Value};

use crate::app::{now, round};

pub struct Job {
    pub id: String,
    pub started: f64,
    pub cancel: bool,
    pub body: Map<String, Value>,
}

struct Reg {
    seq: u64,
    jobs: HashMap<String, Job>,
}

static REG: Mutex<Option<Reg>> = Mutex::new(None);

fn with<T>(f: impl FnOnce(&mut Reg) -> T) -> T {
    let mut g = REG.lock();
    let r = g.get_or_insert_with(|| Reg { seq: 0, jobs: HashMap::new() });
    f(r)
}

pub fn running(kind: &str) -> Vec<String> {
    with(|r| r.jobs.values().filter(|j| j.body["state"] == "running" && j.id.starts_with(&format!("{kind}-")))
        .map(|j| j.id.clone()).collect())
}

pub fn new(kind: &str, total: usize) -> String {
    with(|r| {
        r.seq += 1;
        let t = now();
        let id = format!("{kind}-{}-{}", (t * 1000.0) as u64, r.seq);
        // Keep 40 finished, by START time; never a running one.
        let mut finished: Vec<(f64, String)> = r.jobs.values().filter(|j| j.body["state"] != "running")
            .map(|j| (j.started, j.id.clone())).collect();
        finished.sort_by(|a, b| a.0.total_cmp(&b.0));
        if finished.len() > 40 {
            for (_, old) in &finished[..finished.len() - 40] {
                r.jobs.remove(old);
            }
        }
        let mut body = Map::new();
        for (k, v) in [("id", json!(id)), ("kind", json!(kind)), ("total", json!(total)), ("done", json!(0)),
                       ("state", json!("running")), ("results", json!([])), ("errors", json!([])),
                       ("started", json!(t)), ("cancel", json!(false))] {
            body.insert(k.into(), v);
        }
        r.jobs.insert(id.clone(), Job { id: id.clone(), started: t, cancel: false, body });
        id
    })
}

/// How many finished jobs the registry still holds (bounded by pruning).
pub fn finished_count() -> usize {
    with(|r| r.jobs.values().filter(|j| j.body["state"] != "running").count())
}

pub fn get(id: &str) -> Option<Map<String, Value>> {
    with(|r| r.jobs.get(id).map(|j| {
        let mut b = j.body.clone();
        b.insert("cancel".into(), json!(j.cancel));
        b
    }))
}

/// Record one item; false once the job has been cancelled.
pub fn step(id: &str, ok: bool, payload: Value) -> bool {
    with(|r| match r.jobs.get_mut(id) {
        None => false,
        Some(j) => {
            let d = j.body["done"].as_u64().unwrap_or(0) + 1;
            j.body.insert("done".into(), json!(d));
            let key = if ok { "results" } else { "errors" };
            if let Some(Value::Array(a)) = j.body.get_mut(key) {
                a.push(payload);
            }
            !j.cancel
        }
    })
}

pub fn finish(id: &str, state: &str) {
    with(|r| {
        if let Some(j) = r.jobs.get_mut(id) {
            j.body.insert("state".into(), json!(state));
            j.body.insert("elapsed".into(), json!(round(now() - j.started, 1)));
        }
    })
}

pub fn set(id: &str, key: &str, v: Value) {
    with(|r| {
        if let Some(j) = r.jobs.get_mut(id) {
            j.body.insert(key.into(), v);
        }
    })
}

pub fn cancel(id: &str) -> bool {
    with(|r| match r.jobs.get_mut(id) {
        Some(j) => {
            j.cancel = true;
            true
        }
        None => false,
    })
}

/// Run a job body on its own thread; whatever it does, the job ends.
pub fn spawn(id: String, f: impl FnOnce() + Send + 'static) {
    std::thread::spawn(move || {
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
        if let Err(e) = r {
            let msg = e.downcast_ref::<String>().cloned()
                .or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string()))
                .unwrap_or_else(|| "panic".into());
            eprintln!("job {id} failed: {msg}");
            step(&id, false, json!({"name": "", "error": format!("Panic: {msg}")}));
        }
        if get(&id).map(|j| j["state"] == "running").unwrap_or(false) {
            finish(&id, "failed");
        }
        crate::app::release_memory();
    });
}

/// How many frames a batch works on at once. Each 45 MP frame in flight is
/// about 265 MB at its peak, and this machine is shared, so a quarter of the
/// cores, never more than three.
pub fn batch_workers() -> usize {
    // Z8_BATCH_WORKERS=1 is the old one-at-a-time behaviour, for measuring.
    if let Some(n) = std::env::var("Z8_BATCH_WORKERS").ok().and_then(|v| v.parse::<usize>().ok()) {
        return n.clamp(1, 16);
    }
    let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
    (cores / 4).clamp(1, 3)
}

/// Run `work` over `items` on up to `workers` threads and hand each result to
/// `done` on THIS thread, in the order they finish. Everything with a shared
/// side effect (a job step, a write to edits.json) belongs in `done`, never in
/// `work`. `done` returns false to stop: no new item starts, and the ones
/// already running finish and are dropped. Returns false if it was stopped.
pub fn par_each<T: Sync, R: Send>(items: &[T], workers: usize, work: impl Fn(&T) -> R + Sync,
                                  mut done: impl FnMut(&T, R) -> bool) -> bool {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::Relaxed};
    let (next, stop) = (AtomicUsize::new(0), AtomicBool::new(false));
    // A hand-off, not a queue: a worker waits until its result is TAKEN
    // before it starts another item, so a stop is seen within one item per
    // worker. With an unbounded queue a busy job thread let 9 start after a
    // stop at 5 (measured in a_batch_runs_in_parallel_and_stops_cleanly).
    let (tx, rx) = std::sync::mpsc::sync_channel::<(usize, R)>(0);
    std::thread::scope(|sc| {
        for _ in 0..workers.clamp(1, items.len().max(1)) {
            let tx = tx.clone();
            let (next, stop, work) = (&next, &stop, &work);
            sc.spawn(move || loop {
                if stop.load(Relaxed) {
                    break;
                }
                let i = next.fetch_add(1, Relaxed);
                if i >= items.len() || tx.send((i, work(&items[i]))).is_err() {
                    break;
                }
            });
        }
        drop(tx);
        let mut going = true;
        for (i, r) in rx {
            if going && !done(&items[i], r) {
                going = false;
                stop.store(true, Relaxed);
            }
        }
        going
    })
}
