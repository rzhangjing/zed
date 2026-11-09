//! stub
//!
//! `burn-candle` is only ever an *optional* backend of Burn, but Cargo resolves
//! optional dependencies into `Cargo.lock` regardless of the selected features.
//! This empty, dependency-free crate replaces it through `[patch.crates-io]` so
//! the resolver never has to fetch the Candle/Hugging Face tree.
//!
//! The backend's features are off everywhere in this repository, so nothing
//! from this crate is ever compiled. Do not add dependencies or code here.
