//! Test-only access to the built-in grammar assets.
//!
//! The `grammars` crate package is gone, but its `src/` directory remains the
//! single source of truth for grammar configs and Tree-sitter queries: tests
//! `include_str!` files from it and the debug watcher in the `zed` binary
//! reloads it at runtime. This module reproduces the crate's loaders so test
//! helpers can keep constructing languages without a `grammars` dependency.

use std::borrow::Cow;
use std::sync::Arc;

use anyhow::Context as _;
use language_core::{LanguageConfig, LanguageQueries, QueryFile, QueryFileContents};

use crate::Language;

// Dev builds read the checkout's query files at runtime instead of embedding
// them; see the `assets` crate for the rationale.
util::fs_embed! {
    struct GrammarDir,
    crate_relative = "../grammars/src",
    root_relative = "crates/framework/grammars/src",
    exclude = ["*.rs"],
}

/// Get a raw embedded file by path (relative to the grammar assets directory).
pub fn get_file(path: &str) -> Option<util::__rust_embed::EmbeddedFile> {
    GrammarDir::get(path)
}

/// Load and parse the `config.toml` for a given language name.
pub fn load_config(name: &str) -> LanguageConfig {
    let config_toml = String::from_utf8(
        get_file(&format!("{}/config.toml", name))
            .unwrap_or_else(|| panic!("missing config for language {:?}", name))
            .data
            .to_vec(),
    )
    .unwrap();

    LanguageConfig::from_toml(&config_toml)
        .with_context(|| format!("failed to load config.toml for language {name:?}"))
        .unwrap()
}

/// Load all Tree-sitter query files for a given language name.
pub fn load_queries(name: &str) -> LanguageQueries {
    LanguageQueries::from_files(GrammarDir::iter().filter_map(|path| {
        let file_name = path.strip_prefix(name)?.strip_prefix('/')?;
        let query_file = file_name.parse::<QueryFile>().ok()?;
        let contents = match GrammarDir::get(path.as_ref())?.data {
            Cow::Borrowed(bytes) => Cow::Borrowed(std::str::from_utf8(bytes).ok()?),
            Cow::Owned(bytes) => Cow::Owned(String::from_utf8(bytes).ok()?),
        };
        Some(QueryFileContents::new(query_file, contents))
    }))
}

#[doc(hidden)]
pub fn rust_lang() -> Arc<Language> {
    test_language("rust", tree_sitter_rust::LANGUAGE.into())
}

#[doc(hidden)]
pub fn json_lang() -> Arc<Language> {
    test_language("json", tree_sitter_json::LANGUAGE.into())
}

#[doc(hidden)]
pub fn markdown_lang() -> Arc<Language> {
    test_language("markdown", tree_sitter_md::LANGUAGE.into())
}

#[doc(hidden)]
pub fn language(name: &str, grammar: tree_sitter::Language) -> Arc<Language> {
    test_language(name, grammar)
}

fn test_language(name: &str, grammar: tree_sitter::Language) -> Arc<Language> {
    Arc::new(
        Language::new(load_config(name), Some(grammar))
            .with_queries(load_queries(name))
            .unwrap_or_else(|error| panic!("could not parse queries for language {name}: {error}")),
    )
}
