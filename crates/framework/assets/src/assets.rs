// This crate was essentially pulled out verbatim from main `zed` crate to avoid having to run RustEmbed macro whenever zed has to be rebuilt. It saves a second or two on an incremental build.

use anyhow::Context as _;
use gpui::{App, AssetSource, Result, SharedString};

// Release builds embed the assets; dev builds read them from the checkout at
// runtime so edits show up on the next launch without a rebuild and no
// build-time path is baked in (which corgi's sandbox rejects). See
// `util::fs_embed!`.
util::fs_embed! {
    pub struct Assets,
    crate_relative = "../../../assets",
    root_relative = "assets",
    include = [
        "fonts/**/*",
        "icons/**/*",
        "images/**/*",
        "themes/**/*",
        "sounds/**/*",
        "prompts/**/*",
        "*.md",
    ],
    exclude = ["themes/src/*", "*.DS_Store"],
}

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<std::borrow::Cow<'static, [u8]>>> {
        Self::get(path)
            .map(|f| Some(f.data))
            .with_context(|| format!("loading asset at path {path:?}"))
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        Ok(Self::iter()
            .filter_map(|p| {
                if p.starts_with(path) {
                    Some(p.into())
                } else {
                    None
                }
            })
            .collect())
    }
}

impl Assets {
    /// Populate the [`TextSystem`] of the given [`AppContext`] with all `.ttf` fonts in the `fonts` directory.
    pub fn load_fonts(&self, cx: &App) -> anyhow::Result<()> {
        let font_paths = self.list("fonts")?;
        let mut embedded_fonts = Vec::new();
        for font_path in font_paths {
            if font_path.ends_with(".ttf") {
                let font_bytes = cx
                    .asset_source()
                    .load(&font_path)?
                    .expect("Assets should never return None");
                embedded_fonts.push(font_bytes);
            }
        }

        cx.text_system().add_fonts(embedded_fonts)
    }

    pub fn load_test_fonts(&self, cx: &App) {
        cx.text_system()
            .add_fonts(vec![
                self.load("fonts/lilex/Lilex-Regular.ttf").unwrap().unwrap(),
            ])
            .unwrap()
    }
}

/// The asset source the running application registers: this crate's [`Assets`]
/// first, GPUI Kit's icon catalog second.
///
/// GPUI has a single asset source per application — [`gpui::App::with_assets`]
/// overwrites it, with no multi-source or fallback mechanism — so anything that
/// adds a catalog has to compose it here.
///
/// Zed is consulted **first**, for every path. That is what keeps adding GPUI
/// Kit invisible to the rest of the application:
///
/// * Zed's 395 icons and GPUI Kit's 1830 overlap on 59 names, and the bytes
///   differ for all 59, so "last one wins" would silently restyle 59 existing
///   Zed icons (for example `icons/info.svg`, `icons/settings.svg`,
///   `icons/terminal.svg`).
/// * Zed's icon names are snake_case (`icons/arrow_up.svg`) while GPUI Kit's
///   are kebab-case (`icons/arrow-up.svg`), so the two catalogs mostly address
///   different paths and the ordering rarely matters at all.
///
/// A path only GPUI Kit has — the ones its components name through
/// `gpui_kit_assets::IconName`, such as `icons/arrow-up.svg` — falls through to
/// [`gpui_kit_assets::AllAssets`], the full 1830-icon catalog that
/// `gpui-component-story` also mounts.
#[derive(Clone, Copy, Debug, Default)]
pub struct AppAssets;

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> Result<Option<std::borrow::Cow<'static, [u8]>>> {
        match Assets.load(path) {
            Ok(Some(bytes)) => Ok(Some(bytes)),
            // Neither bundled source signals a miss with `Ok(None)`: both are
            // rust-embed generated and turn its `None` into an error, so a
            // failure here means "Zed does not have this path".
            Ok(None) | Err(_) => gpui_kit_assets::AllAssets.load(path),
        }
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut paths = Assets.list(path)?;
        paths.extend(gpui_kit_assets::AllAssets.list(path)?);
        paths.sort();
        paths.dedup();
        Ok(paths)
    }
}

#[cfg(test)]
mod tests {
    use super::{AppAssets, Assets};
    use gpui::{AssetSource as _, SharedString};

    /// Zed's own assets keep answering from Zed: a font, and an icon whose name
    /// both catalogs happen to use.
    #[test]
    fn zed_paths_come_from_zed() {
        for path in ["fonts/lilex/Lilex-Regular.ttf", "icons/info.svg"] {
            let zed = Assets.load(path).unwrap().unwrap();
            assert_eq!(AppAssets.load(path).unwrap().unwrap(), zed, "{path}");
        }
    }

    /// `icons/info.svg` exists in both catalogs with different bytes. Zed's
    /// copy must win, or registering the composite would restyle the paths the
    /// editor already draws.
    #[test]
    fn overlapping_icon_prefers_zed_bytes() {
        let zed = Assets.load("icons/info.svg").unwrap().unwrap();
        let kit = gpui_kit_assets::AllAssets
            .load("icons/info.svg")
            .unwrap()
            .unwrap();

        assert_ne!(zed, kit, "the two catalogs are expected to differ here");
        assert_eq!(AppAssets.load("icons/info.svg").unwrap().unwrap(), zed);
    }

    /// Every icon both catalogs ship must still come from Zed, and the two
    /// copies must really differ — otherwise the test above would pass for the
    /// wrong reason.
    #[test]
    fn every_overlapping_icon_keeps_zed_bytes() {
        let kit: std::collections::HashSet<SharedString> = gpui_kit_assets::AllAssets
            .list("icons/")
            .unwrap()
            .into_iter()
            .collect();

        let mut overlapping = 0;
        for path in Assets.list("icons/").unwrap() {
            if !kit.contains(&path) {
                continue;
            }
            overlapping += 1;

            let zed = Assets.load(&path).unwrap().unwrap();
            let kit_bytes = gpui_kit_assets::AllAssets.load(&path).unwrap().unwrap();
            assert_eq!(AppAssets.load(&path).unwrap().unwrap(), zed, "{path}");
            assert_ne!(zed, kit_bytes, "{path} is expected to differ");
        }

        assert_eq!(
            overlapping, 59,
            "Zed and GPUI Kit are expected to share 59 icon names"
        );
    }

    /// A path only GPUI Kit has must come from the fallback.
    #[test]
    fn kit_only_icon_falls_back_to_kit() {
        assert!(
            Assets.load("icons/arrow-up.svg").is_err(),
            "this icon is meant to be GPUI Kit-only"
        );

        let kit = gpui_kit_assets::AllAssets
            .load("icons/arrow-up.svg")
            .unwrap()
            .unwrap();
        assert_eq!(AppAssets.load("icons/arrow-up.svg").unwrap().unwrap(), kit);
    }

    /// A path nobody has is still an error, so a real miss is not swallowed.
    #[test]
    fn missing_path_is_an_error() {
        assert!(Assets.load("icons/not-a-real-icon.svg").is_err());
        assert!(AppAssets.load("icons/not-a-real-icon.svg").is_err());
    }

    /// `list` is the union of both catalogs, sorted and deduplicated.
    #[test]
    fn list_merges_both_catalogs_and_dedups() {
        let zed = Assets.list("icons/").unwrap();
        let kit = gpui_kit_assets::AllAssets.list("icons/").unwrap();
        let merged: Vec<SharedString> = AppAssets.list("icons/").unwrap();

        let mut expected: Vec<SharedString> = zed.iter().chain(kit.iter()).cloned().collect();
        expected.sort();
        expected.dedup();
        assert_eq!(merged, expected);

        // The catalogs really do overlap, so merging has to have removed
        // something for this test to mean anything.
        assert!(
            merged.len() < zed.len() + kit.len(),
            "expected the two catalogs to share names"
        );
        assert!(merged.contains(&SharedString::from("icons/arrow-up.svg")));
        assert!(merged.contains(&SharedString::from("icons/magnifying_glass.svg")));
    }
}
