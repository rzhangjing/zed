//! What this build can call, layer by layer.
//!
//! The RustGIS tree is layered L0-L5 and `crates/rustgis-zed` links **all twelve** of its library
//! crates, so every layer is compiled in. This module says, for each one, what a host can actually
//! do with it right now: a measured count where that is cheap and honest (`L2: 4 layers`,
//! `L4: 213 commands`, `L5: 8 provider presets`), and [`Probe::Linked`] where the crate is
//! reachable but only a real use can prove more — a keychain needs a user session, a GeoLibre tool
//! needs the `geolibre` program, the assistant needs provider credentials.
//!
//! It is deliberately side-effect free: no network, no credential store, no font scan, no file
//! access. `Keychain::default()` is documented upstream as "never touches D-Bus or triggers a
//! prompt", which is why constructing one here is allowed.

use crate::MapEngine;

/// How a layer's availability was established.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Probe {
    /// The crate is linked into this build; nothing was exercised at runtime.
    Linked,
    /// A count the host can show, measured on the live session or on embedded data.
    Measured(usize),
}

/// One RustGIS layer, as this build sees it.
#[derive(Clone, Debug, PartialEq)]
pub struct Capability {
    /// `L0` … `L5`.
    pub layer: &'static str,
    /// The upstream crate name.
    pub krate: &'static str,
    /// What a host can do with it today.
    pub detail: String,
    pub probe: Probe,
}

impl Capability {
    /// Whether the layer is usable at all: everything listed here is compiled and linked in.
    pub fn available(&self) -> bool {
        match self.probe {
            Probe::Linked | Probe::Measured(_) => true,
        }
    }
}

/// Every layer's availability without a session. The counts that need one are filled in by
/// [`of_engine`]; this is what a host with no session yet can show.
pub fn all() -> Vec<Capability> {
    let embedded_sample_data = rustgis_render::sample_data::COUNTRIES.len()
        + rustgis_render::sample_data::CITIES.len()
        + rustgis_render::sample_data::RIVERS.len()
        + rustgis_render::sample_data::LAKES.len();
    // `external_tools()` is empty until a suite registers (the engine registers the ones built in,
    // and `rustgis-geolibre`'s tools only when the engine's `geolibre` feature is on); counting it
    // is the cheapest honest way to reach into L2 analysis.
    let external_tools = rustgis_analysis::external_tools().len();
    // Constructing the hook is documented as side-effect free (no D-Bus, no prompt); the store
    // itself opens on first use, which this never does.
    let _ = rustgis_keychain::Keychain::default();
    let presets = rustgis_assistant::provider::PRESETS.len();

    vec![
        Capability {
            layer: "L0",
            krate: "rustgis-geom",
            detail: "coordinates, Web Mercator, geodesic measurement (camera centre in lon/lat)"
                .to_string(),
            probe: Probe::Linked,
        },
        Capability {
            layer: "L1",
            krate: "rustgis-data",
            detail: "features, typed attributes, GeoJSON/CSV/Shapefile/KML/GPX I/O, GeoTIFF \
                     rasters (exercised through load_geojson)"
                .to_string(),
            probe: Probe::Linked,
        },
        Capability {
            layer: "L2",
            krate: "rustgis-map",
            detail: "layers, symbology, labels, camera, basemaps".to_string(),
            probe: Probe::Linked,
        },
        Capability {
            layer: "L2",
            krate: "rustgis-analysis",
            detail: format!(
                "geoprocessing tools, through Session::run (`gp.*`); {external_tools} external \
                 suites registered"
            ),
            probe: Probe::Measured(external_tools),
        },
        Capability {
            layer: "L2",
            krate: "rustgis-tiles",
            detail: "XYZ/TMS/WMS rasters, MVT, PMTiles/MBTiles, a MapLibre style subset".to_string(),
            probe: Probe::Linked,
        },
        Capability {
            layer: "L2",
            krate: "rustgis-keychain",
            detail: "system secret store (Windows Credential Manager): linked, the store opens on \
                     first use and needs a user session to prove"
                .to_string(),
            probe: Probe::Linked,
        },
        Capability {
            layer: "L3",
            krate: "rustgis-render",
            detail: format!(
                "tiny-skia renderer, label placement, PNG export; {embedded_sample_data} bytes of \
                 embedded Natural Earth sample data"
            ),
            probe: Probe::Measured(embedded_sample_data),
        },
        Capability {
            layer: "L3",
            krate: "rustgis-plugins",
            detail: format!(
                "sandboxed WebAssembly plugins (wasmi) under `{}`: linked, none installed",
                rustgis_plugins::PREFIX
            ),
            probe: Probe::Linked,
        },
        Capability {
            layer: "L3",
            krate: "rustgis-geolibre",
            detail: format!(
                "GeoLibre/Whitebox tool suite under `{}` (source `{}`): linked; its tools register \
                 with the engine's `geolibre` feature, and running one needs the `geolibre` program",
                rustgis_geolibre::PREFIX,
                rustgis_geolibre::SOURCE
            ),
            probe: Probe::Linked,
        },
        Capability {
            layer: "L4",
            krate: "rustgis-engine",
            detail: "Session, undo, command registry".to_string(),
            probe: Probe::Linked,
        },
        Capability {
            layer: "L5",
            krate: "rustgis-assistant",
            detail: format!(
                "AI assistant agent loop with {presets} provider presets: linked, needs provider \
                 credentials to run"
            ),
            probe: Probe::Measured(presets),
        },
    ]
}

/// [`all`], with the counts a live session can measure filled in.
pub fn of_engine(engine: &MapEngine) -> Vec<Capability> {
    let layers = engine.layer_count();
    let commands = engine.commands().len();
    let tiles = engine.tile_layers().len();
    let geoprocessing = engine.commands_with_prefix("gp.").len();
    let plugin_commands = engine.commands_with_prefix(rustgis_plugins::PREFIX).len();
    all()
        .into_iter()
        .map(|mut capability| match capability.krate {
            "rustgis-map" => {
                capability.detail = format!(
                    "layers, symbology, labels, camera, basemaps: {layers} layers loaded"
                );
                capability.probe = Probe::Measured(layers);
                capability
            }
            "rustgis-tiles" => {
                capability.detail = format!(
                    "XYZ/TMS/WMS rasters, MVT, PMTiles/MBTiles: {tiles} tile layers loaded"
                );
                capability.probe = Probe::Measured(tiles);
                capability
            }
            "rustgis-analysis" => {
                capability.detail = format!(
                    "geoprocessing tools, through Session::run: {geoprocessing} `gp.*` commands \
                     registered"
                );
                capability.probe = Probe::Measured(geoprocessing);
                capability
            }
            "rustgis-plugins" => {
                capability.detail = format!(
                    "sandboxed WebAssembly plugins (wasmi): {plugin_commands} `{}` commands \
                     registered",
                    rustgis_plugins::PREFIX
                );
                capability.probe = Probe::Measured(plugin_commands);
                capability
            }
            "rustgis-engine" => {
                capability.detail = format!(
                    "Session, undo, command registry: {commands} commands registered"
                );
                capability.probe = Probe::Measured(commands);
                capability
            }
            _ => capability,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_layer_from_l0_to_l5_is_reported() {
        let capabilities = all();
        for layer in ["L0", "L1", "L2", "L3", "L4", "L5"] {
            assert!(
                capabilities.iter().any(|c| c.layer == layer),
                "{layer} is not reported"
            );
        }
        assert!(capabilities.iter().all(Capability::available));
    }

    #[test]
    fn every_library_crate_of_the_copied_tree_is_named() {
        let capabilities = all();
        for krate in [
            "rustgis-geom",
            "rustgis-data",
            "rustgis-map",
            "rustgis-analysis",
            "rustgis-tiles",
            "rustgis-keychain",
            "rustgis-render",
            "rustgis-plugins",
            "rustgis-geolibre",
            "rustgis-engine",
            "rustgis-assistant",
        ] {
            assert!(
                capabilities.iter().any(|c| c.krate == krate),
                "{krate} is not reachable from the adapter"
            );
        }
    }

    #[test]
    fn a_live_session_measures_layers_and_commands() {
        let engine = MapEngine::new();
        let capabilities = of_engine(&engine);
        let map = capabilities
            .iter()
            .find(|c| c.krate == "rustgis-map")
            .expect("rustgis-map is reported");
        assert_eq!(
            map.probe,
            Probe::Measured(4),
            "the sample project has 4 layers"
        );
        let engine = capabilities
            .iter()
            .find(|c| c.krate == "rustgis-engine")
            .expect("rustgis-engine is reported");
        match engine.probe {
            Probe::Measured(count) => assert!(count > 100, "{count} commands is too few"),
            Probe::Linked => panic!("the engine count should be measured"),
        }
    }
}
