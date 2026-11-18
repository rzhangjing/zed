mod _admin;
pub mod login;
mod logout;
mod theme;

use topcoat::{
    Result,
    asset::{AssetBundle, RouterBuilderAssetExt},
    context::Cx,
    cookie::RouterBuilderCookieExt,
    font::{Font, fontsource::fontsource_font},
    router::{RouterBuilder, RouterBuilderDiscoverExt, Slot, layout, module_router},
    runtime::RouterBuilderRuntimeExt,
    session::{RouterBuilderSessionExt, SessionConfig},
    tailwind,
    view::{View, view},
};

use crate::features::theme::theme;

const GEIST: Font =
    fontsource_font!(GEIST, weight: [400, 500, 600, 700], style: Normal, host: Asset);

pub fn router() -> RouterBuilder {
    module_router!()
        .discover()
        .assets(assets())
        .cookies()
        .sessions(SessionConfig::default())
        // Register the runtime after the application layers so rerenders use them too.
        .runtime()
}

/// The bundled assets.
///
/// `AssetBundle::load` looks for an `assets` directory next to the executable,
/// which `cargo topcoat asset bundle` writes (`cargo topcoat dev` does it on
/// every build). A host that embeds this crate has its own executable — Zed's
/// `Help > Admin Demo` runs the demo in `zed.exe` — so a bundle has to be
/// generated from a binary built with this crate; `ADMIN_ASSETS_DIR` points at
/// one directly.
///
/// A missing bundle is a hard error, not a degraded mode: the views call into
/// the asset configuration while rendering (`tailwind::stylesheet!`, fonts,
/// icons), so serving without one answers 500 instead of rendering the demo.
fn assets() -> AssetBundle {
    if let Some(dir) = std::env::var_os("ADMIN_ASSETS_DIR").filter(|value| !value.is_empty()) {
        return AssetBundle::load_dir(dir).unwrap_or_else(|error| {
            panic!("ADMIN_ASSETS_DIR does not hold an asset bundle: {error}")
        });
    }

    AssetBundle::load().unwrap_or_else(|error| {
        panic!(
            "{error}; generate the bundle from a binary built with this crate, e.g. \
             `cargo run -p admin --features bundle-tool --bin admin-bundle-assets -- \
             <binary> <its directory>/assets`"
        )
    })
}

#[layout]
async fn root(cx: &Cx, slot: Slot<'_>) -> Result<impl View> {
    let theme = theme(cx);

    Ok(view! {
        <!DOCTYPE html>
        <html lang="en" class=(theme.class()) style=(theme.color_scheme())>
            <head>
                <meta charset="utf-8">
                <meta name="viewport" content="width=device-width, initial-scale=1">
                <title>"Backroom"</title>
                <link rel="stylesheet" href=(tailwind::stylesheet!())>
                topcoat::font::link(font: GEIST)
                topcoat::runtime::script()
                topcoat::dev::script()
            </head>
            <body>(slot)</body>
        </html>
    })
}
