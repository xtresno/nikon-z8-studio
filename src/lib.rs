// Two lints are off on purpose. Pixel loops index several buffers by the same
// x, which reads more plainly than zipped iterators, and `chunks_exact(3)` over
// RGB is clearer here than `as_chunks`. Everything else stays at the default.
#![allow(clippy::needless_range_loop, clippy::chunks_exact_to_as_chunks)]
pub mod advice;
pub mod alloc;
pub mod analysis;
pub mod app;
pub mod audio;
pub mod camera;
pub mod export;
pub mod feedctl;
pub mod frame;
pub mod gp;
pub mod http;
pub mod grade;
pub mod img;
pub mod jobs;
pub mod load;
pub mod recorder;
pub mod store;
pub mod tools;
