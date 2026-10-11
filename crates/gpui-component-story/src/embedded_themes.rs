#[cfg(target_family = "wasm")]
use std::collections::HashMap;

#[cfg(target_family = "wasm")]
pub fn embedded_themes() -> HashMap<&'static str, &'static str> {
    let mut themes = HashMap::new();

    themes.insert("adventure", include_str!("../../../gpui-kit/themes/adventure.json"));
    themes.insert("alduin", include_str!("../../../gpui-kit/themes/alduin.json"));
    themes.insert("asciinema", include_str!("../../../gpui-kit/themes/asciinema.json"));
    themes.insert("aurora", include_str!("../../../gpui-kit/themes/aurora.json"));
    themes.insert("ayu", include_str!("../../../gpui-kit/themes/ayu.json"));
    themes.insert(
        "catppuccin",
        include_str!("../../../gpui-kit/themes/catppuccin.json"),
    );
    themes.insert(
        "everforest",
        include_str!("../../../gpui-kit/themes/everforest.json"),
    );
    themes.insert(
        "fahrenheit",
        include_str!("../../../gpui-kit/themes/fahrenheit.json"),
    );
    themes.insert("flexoki", include_str!("../../../gpui-kit/themes/flexoki.json"));
    themes.insert("gruvbox", include_str!("../../../gpui-kit/themes/gruvbox.json"));
    themes.insert("harper", include_str!("../../../gpui-kit/themes/harper.json"));
    themes.insert("hybrid", include_str!("../../../gpui-kit/themes/hybrid.json"));
    themes.insert(
        "jellybeans",
        include_str!("../../../gpui-kit/themes/jellybeans.json"),
    );
    themes.insert("kibble", include_str!("../../../gpui-kit/themes/kibble.json"));
    themes.insert(
        "macos-classic",
        include_str!("../../../gpui-kit/themes/macos-classic.json"),
    );
    themes.insert(
        "mellifluous",
        include_str!("../../../gpui-kit/themes/mellifluous.json"),
    );
    themes.insert("molokai", include_str!("../../../gpui-kit/themes/molokai.json"));
    themes.insert("solarized", include_str!("../../../gpui-kit/themes/solarized.json"));
    themes.insert("spaceduck", include_str!("../../../gpui-kit/themes/spaceduck.json"));
    themes.insert(
        "tokyonight",
        include_str!("../../../gpui-kit/themes/tokyonight.json"),
    );
    themes.insert("twilight", include_str!("../../../gpui-kit/themes/twilight.json"));

    themes
}
