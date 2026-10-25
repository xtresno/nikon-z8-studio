//! FFI smoke test with no camera attached: every libgphoto2 entry point the
//! app uses must fail cleanly, with python-gphoto2's "[code] text" shape.
fn main() {
    for code in [-1, -7, -10, -53, -105, -110] {
        println!("{code:5} -> {}", z8::gp::err_text(code));
    }
    for i in 0..3 {
        match z8::gp::Session::open() {
            Ok(_) => println!("open {i}: a camera answered (unexpected)"),
            Err(e) => println!("open {i}: {e}"),
        }
    }
    println!("no crash across repeated opens");
}
