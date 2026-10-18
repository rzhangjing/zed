use gpui::{AsyncApp, actions};

/// URL scheme handled by the `zed://` links this app registers itself for
/// (for example `zed://agent/shared/<session-id>`).
pub const ZED_URL_SCHEME: &str = "zed";

actions!(
    cli,
    [
        /// Registers the zed:// URL scheme handler.
        RegisterZedScheme
    ]
);

pub async fn register_zed_scheme(cx: &AsyncApp) -> anyhow::Result<()> {
    cx.update(|cx| cx.register_url_scheme(ZED_URL_SCHEME)).await
}
