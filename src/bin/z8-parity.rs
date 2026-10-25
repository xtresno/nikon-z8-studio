//! Parity harness: run one pipeline step on a file and write raw RGB, so a
//! Python script can compare it against Pillow. Not shipped.
use z8::{app, grade, img, load};
fn main() {
    let a: Vec<String> = std::env::args().collect();
    app::init();
    let (src, op, out) = (&a[1], &a[2], &a[3]);
    let arg = a.get(4).cloned().unwrap_or_default();
    let p = std::path::Path::new(src);
    // Measurements: print JSON instead of writing pixels.
    match op.as_str() {
        "auto" => {
            let im = load::load_editable(p, None).unwrap();
            let (adj, why, meas) = z8::analysis::auto_look(&im);
            println!("{}", serde_json::json!({"adjust": adj, "why": why, "measured": meas}));
            return;
        }
        "light" => {
            let im = load::load_editable(p, None).unwrap();
            println!("{}", z8::analysis::lighting_report(&im));
            return;
        }
        // Face brightness before and after a look: {"before": y, "after": y}.
        "skin" => {
            let im = load::load_editable(p, Some(700)).unwrap();
            let small = img::thumbnail(&im, 420, 100_000);
            let before = z8::analysis::skin_mask_stats(&small).map(|f| f.y);
            let adj: serde_json::Value = serde_json::from_str(if arg.is_empty() { "{}" } else { &arg }).unwrap();
            let graded = img::thumbnail(&grade::apply_edits(&im, &adj, grade::Phase::All), 420, 100_000);
            let after = z8::analysis::skin_mask_stats(&graded).map(|f| f.y);
            println!("{}", serde_json::json!({"before": before, "after": after}));
            return;
        }
        // Where the loudest onset (the clap) sits, in seconds, as the sync
        // route measures it.
        "onset" => {
            println!("{}", serde_json::json!(z8::audio::loudest_onset(p, 20.0)));
            return;
        }
        // PNG compression levels on a full frame: time and size for each.
        "pngbench" => {
            let im = load::load_editable(p, None).unwrap();
            for (label, c) in [("Fast", png::Compression::Fast), ("Balanced", png::Compression::Balanced)] {
                let t = std::time::Instant::now();
                let mut out = Vec::new();
                {
                    let mut enc = png::Encoder::new(&mut out, im.w as u32, im.h as u32);
                    enc.set_color(png::ColorType::Rgb);
                    enc.set_depth(png::BitDepth::Eight);
                    enc.set_compression(c);
                    enc.write_header().unwrap().write_image_data(&im.px).unwrap();
                }
                let back = img::png_decode(&out).unwrap();
                println!("{label}: {:.2}s {:.1} MB lossless={}", t.elapsed().as_secs_f64(),
                         out.len() as f64 / 1e6, back.px == im.px);
            }
            return;
        }
        "quality" => {
            let im = load::load_editable(p, Some(2200)).unwrap();
            println!("{}", z8::analysis::frame_quality(&im).unwrap_or(serde_json::Value::Null));
            return;
        }
        _ => {}
    }
    let im = match op.as_str() {
        "load" => load::load_editable(p, None).unwrap(),
        "draft" => load::load_editable(p, Some(arg.parse().unwrap())).unwrap(),
        "thumb" => { let im = load::load_editable(p, Some(2200)).unwrap(); let n: usize = arg.parse().unwrap(); img::thumbnail(&im, n, n) }
        "grade" => { let im = load::load_editable(p, Some(2200)).unwrap(); let im = img::thumbnail(&im, 1100, 1100);
                     let adj: serde_json::Value = serde_json::from_str(&arg).unwrap(); grade::apply_edits(&im, &adj, grade::Phase::All) }
        "embedded" => { let n: usize = arg.parse().unwrap(); let im = load::raw_embedded(p, Some(n * 3)).expect("no embedded JPEG"); img::thumbnail(&im, n, n * 3) }
        "fit" => { let im = load::load_editable(p, Some(2200)).unwrap(); grade::fit_to(im, 800, 1000, "cover", "face") }
        "straighten" => { let im = load::load_editable(p, Some(2200)).unwrap(); let im = img::thumbnail(&im, 1100, 1100); grade::straighten(im, arg.parse().unwrap()) }
        _ => panic!("op"),
    };
    std::fs::write(out, format!("{} {}\n", im.w, im.h).into_bytes().into_iter().chain(im.px).collect::<Vec<u8>>()).unwrap();
}
