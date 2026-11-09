//! stub
//!
//! `burn-tch` 0.22.1 is a deprecated LibTorch backend. It is only ever an
//! *optional* dependency of Burn (`burn-core`'s `tch` feature), but Cargo locks
//! optional dependencies too, and its real manifest drags in `tch`/`torch-sys`
//! (which downloads libtorch at build time) and pins `libc ^0.2.189`.
//!
//! This empty, dependency-free crate is wired in through `[patch.crates-io]`
//! so the resolver sees a package with no code and no transitive dependencies.
//! The backend's features are off everywhere in this repository, so nothing
//! from this crate is ever compiled. Do not add dependencies or code here.
