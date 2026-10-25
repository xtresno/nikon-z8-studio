//! Z8 Studio: tethered capture, editing and export for a Nikon Z8 on Linux.
use std::time::Duration;

use z8::{app, audio, camera, feedctl, http, load, store};

#[global_allocator]
static ALLOC: z8::alloc::HugeAlloc = z8::alloc::HugeAlloc;

/// Reconnect without waiting for a browser, drop a handle a feed has taken
/// over, and put back an orientation lease nobody confirmed.
fn presence_watch() {
    loop {
        let r = std::panic::catch_unwind(|| {
            if camera::cam().is_open() && feedctl::webcam_running() {
                camera::cam().disconnect();
                camera::cam().a.lock().manual_off = false;
            }
            camera::cam().check_still_attached();
            feedctl::maybe_autoconnect(camera::camera_present());
        });
        if r.is_err() {
            eprintln!("presence watch pass failed");
        }
        std::thread::sleep(Duration::from_secs(6));
    }
}

#[tokio::main]
async fn main() {
    // Every image buffer of 1 MB or more comes straight from the kernel and
    // goes straight back when freed. glibc otherwise raises this threshold
    // after the first big free, and later 34-136 MB decodes land in arenas
    // that fragment and are never returned: measured 249 MB still resident
    // after a load test even with periodic trimming (30 MB with this). A
    // fixed value also switches that dynamic adjustment off.
    unsafe {
        libc::mallopt(libc::M_MMAP_THRESHOLD, 1 << 20);
        libc::mallopt(libc::M_ARENA_MAX, 4);
    }
    let a = app::init();
    let _ = std::fs::create_dir_all(store::today_dir().join("exports"));
    let gone = store::sweep_staging(&[store::export_dir(None), store::today_dir()]);
    if !gone.is_empty() {
        println!("swept {} staging file(s) from an interrupted run", gone.len());
    }
    load::start_convert_worker();
    std::thread::spawn(audio::watchdog);
    // The orientation lease runs on its own clock. It used to ride on the
    // presence watch, so a server started without auto-connect (every test
    // copy) never put a leased rotation back.
    std::thread::spawn(|| loop {
        http::orient_lease_expire();
        std::thread::sleep(Duration::from_secs(1));
    });
    // Freed memory back to the OS every 20 s, as the Python app trimmed after
    // batches: request threads keep what they freed otherwise.
    std::thread::spawn(|| loop {
        std::thread::sleep(Duration::from_secs(20));
        z8::export::full_base_expire();
        app::release_memory();
    });
    if !a.no_autoconnect {
        std::thread::spawn(presence_watch);
        feedctl::restore_mode_if_interrupted();
    }
    let router = axum::Router::new().fallback(http::handle);
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], a.port));
    let listener = match tokio::net::TcpListener::bind(addr).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("cannot listen on {addr}: {e}");
            std::process::exit(1);
        }
    };
    println!("Z8 Studio (rust) on http://127.0.0.1:{}", a.port);
    println!("session dir: {}", store::session_dir(None).display());
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
    let server = tokio::spawn(async move {
        let _ = axum::serve(listener, router)
            .with_graceful_shutdown(async { let _ = stop_rx.await; })
            .await;
    });
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).expect("signal");
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {},
        _ = term.recv() => {},
    }
    // Stop taking requests, give ordinary ones two seconds, and go. A page
    // showing live view holds an MJPEG stream that never ends, and waiting
    // for it kept the service "deactivating" until systemd killed it, which
    // skipped closing the camera: the hard exit that wedges a Z8's USB
    // session. The camera is released on every exit path.
    let _ = stop_tx.send(());
    let _ = tokio::time::timeout(Duration::from_secs(2), server).await;
    camera::cam().disconnect();
    std::process::exit(0);
}
