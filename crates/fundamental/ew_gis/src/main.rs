//! EW GIS stage 2: the standalone `winit` host.
//!
//! The scene, the cameras, the input state and the renderer live in the
//! library's [`ew_gis::viewer`]; this binary only parses the command line and
//! the environment (via [`ew_gis::app::Config`]) and runs the `winit` event
//! loop ([`ew_gis::app::run`]). Embedding hosts (the Zed GPUI window) link the
//! library and never reach this file.

fn main() {
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("ew_gis=info,wgpu_core=warn,wgpu_hal=warn"),
    )
    .format_timestamp_millis()
    .init();

    let config = ew_gis::app::Config::from_args_and_env();
    if let Err(error) = ew_gis::app::run(config) {
        log::error!("ew_gis exited with an error: {error}");
        eprintln!("ew_gis: {error}");
        std::process::exit(1);
    }
}
