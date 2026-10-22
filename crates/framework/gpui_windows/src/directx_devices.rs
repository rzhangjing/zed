//! Direct3D 11 device creation for GPUI.
//!
//! The hardware path enumerates the DXGI adapters and creates a device with
//! [`D3D_DRIVER_TYPE_UNKNOWN`]. When that fails - no usable GPU, a broken
//! driver, a virtual machine without graphics support - we fall back to the
//! WARP software rasterizer so that GPUI can still start.
//!
//! The Direct3D/DXGI debug layer is opt-in in every profile: set
//! [`DEBUG_LAYER_ENV_VAR`] to request it. Debug builds no longer ask for it by
//! default, so they no longer require the Windows "Graphics Tools" optional
//! feature to be installed.

use anyhow::{Context, Result};
use gpui_util::ResultExt;
use itertools::Itertools;
use windows::Win32::{
    Foundation::HMODULE,
    Graphics::{
        Direct3D::{
            D3D_DRIVER_TYPE, D3D_DRIVER_TYPE_UNKNOWN, D3D_DRIVER_TYPE_WARP, D3D_FEATURE_LEVEL,
            D3D_FEATURE_LEVEL_10_1, D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_11_1,
        },
        Direct3D11::{
            D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_CREATE_DEVICE_DEBUG,
            D3D11_FEATURE_D3D10_X_HARDWARE_OPTIONS, D3D11_FEATURE_DATA_D3D10_X_HARDWARE_OPTIONS,
            D3D11_SDK_VERSION, D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext,
        },
        Dxgi::{
            CreateDXGIFactory2, DXGI_CREATE_FACTORY_DEBUG, DXGI_CREATE_FACTORY_FLAGS, IDXGIAdapter,
            IDXGIAdapter1, IDXGIDevice, IDXGIFactory6,
        },
    },
};
use windows::core::Interface;

/// Environment variable that opts into the Direct3D 11 / DXGI debug layer.
///
/// Both debug and release builds default to "no debug layer", so a debug build
/// no longer fails on machines without the Windows "Graphics Tools" optional
/// feature.
const DEBUG_LAYER_ENV_VAR: &str = "GPUI_ENABLE_DEBUG_LAYER";

/// Environment variable that skips hardware adapter detection and creates the
/// device with WARP (software rasterizer), e.g. for testing or for users who
/// want to force software rendering.
const FORCE_WARP_ENV_VAR: &str = "GPUI_FORCE_WARP";

/// Feature levels we ask for, highest first. 4x MSAA is required for Direct3D
/// Feature Level 10.1 or better.
const FEATURE_LEVELS: &[D3D_FEATURE_LEVEL] = &[
    D3D_FEATURE_LEVEL_11_1,
    D3D_FEATURE_LEVEL_11_0,
    D3D_FEATURE_LEVEL_10_1,
];

/// Retried with `11_1` removed when the first attempt fails: not every Direct3D
/// 11 runtime knows the 11.1 feature level, and some drivers reject the whole
/// call instead of skipping the levels they do not support.
const FALLBACK_FEATURE_LEVELS: &[D3D_FEATURE_LEVEL] =
    &[D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_10_1];

pub(crate) fn try_to_recover_from_device_lost<T>(mut f: impl FnMut() -> Result<T>) -> Result<T> {
    (0..5)
        .map(|i| {
            if i > 0 {
                // Add a small delay before retrying
                std::thread::sleep(std::time::Duration::from_millis(100 + i * 10));
            }
            f()
        })
        .find_or_last(Result::is_ok)
        .unwrap()
        .context("DirectXRenderer failed to recover from lost device after multiple attempts")
}

#[derive(Clone)]
pub(crate) struct DirectXDevices {
    pub(crate) adapter: IDXGIAdapter1,
    pub(crate) dxgi_factory: IDXGIFactory6,
    pub(crate) device: ID3D11Device,
    pub(crate) device_context: ID3D11DeviceContext,
}

/// Which path [`DirectXDevices::new`] takes to create the devices.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DevicePlan {
    /// Enumerate the DXGI adapters and use the first one that yields a usable
    /// device, falling back to WARP when none does.
    HardwareThenWarp,
    /// Skip hardware adapter detection entirely and render with WARP.
    WarpOnly,
}

/// Interpret a boolean-ish environment variable value: it counts as "on" when
/// the variable exists and its trimmed value is not empty, `0`, `false` or
/// `no` (case insensitive).
fn env_flag_truthy(env_value: Option<&str>) -> bool {
    let Some(value) = env_value else {
        return false;
    };
    let value = value.trim();
    !value.is_empty()
        && !value.eq_ignore_ascii_case("0")
        && !value.eq_ignore_ascii_case("false")
        && !value.eq_ignore_ascii_case("no")
}

/// Whether the Direct3D 11 / DXGI debug layer should be requested at all.
fn debug_layer_requested(env_value: Option<&str>) -> bool {
    env_flag_truthy(env_value)
}

/// Which device creation path the [`FORCE_WARP_ENV_VAR`] environment variable
/// asks for. The hardware path is tried first unless WARP is explicitly forced,
/// and always falls back to WARP when it fails.
fn device_plan(force_warp_env_value: Option<&str>) -> DevicePlan {
    if env_flag_truthy(force_warp_env_value) {
        DevicePlan::WarpOnly
    } else {
        DevicePlan::HardwareThenWarp
    }
}

/// Returns whether the debug layer was requested *and* can actually be
/// obtained.
///
/// This never fails: when the debug layer was not requested it is not touched
/// at all, and when it was requested but is unavailable we keep going without
/// it instead of failing to start.
fn debug_layer_available() -> bool {
    let env_value = std::env::var(DEBUG_LAYER_ENV_VAR).ok();
    if !debug_layer_requested(env_value.as_deref()) {
        return false;
    }

    if probe_debug_layer() {
        log::info!("{DEBUG_LAYER_ENV_VAR} is set and the Direct3D/DXGI debug layer is available.");
        true
    } else {
        log::warn!(
            "{DEBUG_LAYER_ENV_VAR} is set, but the DXGI debug interface could not be obtained \
             (is the Windows \"Graphics Tools\" optional feature installed?). Continuing without \
             the Direct3D/DXGI debug layer."
        );
        false
    }
}

/// Asks DXGI for the debug info queue, which is what the debug layer would use.
/// Fails when the debug layer is not installed.
fn probe_debug_layer() -> bool {
    use windows::Win32::Graphics::Dxgi::{DXGIGetDebugInterface1, IDXGIInfoQueue};

    unsafe { DXGIGetDebugInterface1::<IDXGIInfoQueue>(0) }
        .warn_on_err()
        .is_some()
}

impl DirectXDevices {
    pub(crate) fn new() -> Result<Self> {
        let debug_layer_available = debug_layer_available();

        match device_plan(std::env::var(FORCE_WARP_ENV_VAR).ok().as_deref()) {
            DevicePlan::WarpOnly => {
                log::info!("{FORCE_WARP_ENV_VAR} is set: skipping hardware adapter detection.");
                directx_devices_from_warp(debug_layer_available)
                    .context("Creating a WARP (software) Direct3D 11 device")
            }
            DevicePlan::HardwareThenWarp => {
                match directx_devices_from_hardware(debug_layer_available) {
                    Ok(devices) => Ok(devices),
                    Err(hardware_error) => {
                        log::warn!(
                            "Failed to create a hardware Direct3D 11 device: {hardware_error:#}"
                        );
                        log::warn!("Falling back to WARP (software) rendering.");
                        directx_devices_from_warp(debug_layer_available).map_err(|warp_error| {
                            device_creation_failed(hardware_error, warp_error)
                        })
                    }
                }
            }
        }
    }
}

/// The error returned when neither the hardware path nor the WARP fallback
/// produced a usable device. It carries both failure reasons.
fn device_creation_failed(
    hardware_error: anyhow::Error,
    warp_error: anyhow::Error,
) -> anyhow::Error {
    anyhow::anyhow!(
        "Failed to create a Direct3D 11 device: the hardware path failed ({hardware_error:#}) \
         and the WARP (software) fallback failed ({warp_error:#})"
    )
}

/// Enumerates the DXGI adapters and creates a device on the first one that
/// works, logging the adapter name it inspects and keeping every adapter's
/// failure reason for the returned error.
fn directx_devices_from_hardware(debug_layer_available: bool) -> Result<DirectXDevices> {
    let dxgi_factory = get_dxgi_factory(debug_layer_available).context("Creating DXGI factory")?;
    let mut failures = Vec::new();

    for adapter_index in 0.. {
        let adapter: IDXGIAdapter1 = match unsafe { dxgi_factory.EnumAdapters(adapter_index) } {
            Ok(adapter) => adapter
                .cast::<IDXGIAdapter1>()
                .context("Casting IDXGIAdapter to IDXGIAdapter1")?,
            Err(error) => {
                if adapter_index == 0 {
                    failures.push(format!("enumerating DXGI adapters failed: {error}"));
                }
                break;
            }
        };

        let gpu_name = adapter_description(&adapter);
        log::info!("Using GPU: {}", gpu_name);

        // Check to see whether this adapter supports Direct3D 11 and create
        // the device if it does, otherwise keep looking.
        match create_device(
            Some(&adapter),
            D3D_DRIVER_TYPE_UNKNOWN,
            debug_layer_available,
        ) {
            Ok((device, device_context, feature_level)) => {
                log_feature_level(feature_level);
                return Ok(DirectXDevices {
                    adapter,
                    dxgi_factory,
                    device,
                    device_context,
                });
            }
            Err(error) => failures.push(format!("{gpu_name} (adapter {adapter_index}): {error:#}")),
        }
    }

    if failures.is_empty() {
        failures.push("no DXGI adapter was enumerated".to_string());
    }

    Err(anyhow::anyhow!(
        "no usable Direct3D 11 hardware adapter: {}",
        failures.join("; ")
    ))
}

/// Creates a device with the WARP (software) rasterizer, which works without a
/// usable GPU or GPU driver.
fn directx_devices_from_warp(debug_layer_available: bool) -> Result<DirectXDevices> {
    // WARP does not need the debug factory flag; the device flags below still
    // carry `D3D11_CREATE_DEVICE_DEBUG` when the debug layer was requested.
    let dxgi_factory: IDXGIFactory6 =
        unsafe { CreateDXGIFactory2(DXGI_CREATE_FACTORY_FLAGS::default()) }
            .context("Creating DXGI factory")?;
    let (device, device_context, feature_level) =
        create_device(None, D3D_DRIVER_TYPE_WARP, debug_layer_available)
            .context("Creating a WARP Direct3D 11 device")?;

    // A device created with `D3D_DRIVER_TYPE_WARP` is not bound to an adapter at
    // creation time, so ask the device for the (software) adapter it runs on.
    let dxgi_device: IDXGIDevice = device
        .cast()
        .context("Getting IDXGIDevice from the WARP device")?;
    let adapter: IDXGIAdapter1 = unsafe { dxgi_device.GetAdapter() }
        .context("Getting the adapter of the WARP device")?
        .cast()
        .context("Casting the WARP IDXGIAdapter to IDXGIAdapter1")?;

    log::info!("Using WARP (software) adapter");
    log_feature_level(feature_level);
    log::warn!(
        "Rendering with the WARP software rasterizer: performance will be significantly lower \
         than with a hardware GPU."
    );

    Ok(DirectXDevices {
        adapter,
        dxgi_factory,
        device,
        device_context,
    })
}

/// Creates a Direct3D 11 device for `driver_type` and checks that it supports
/// the features GPUI needs.
///
/// `adapter` must be `None` for [`D3D_DRIVER_TYPE_WARP`], which takes no
/// adapter; hardware devices are created with [`D3D_DRIVER_TYPE_UNKNOWN`] and
/// the adapter enumerated from DXGI.
fn create_device(
    adapter: Option<&IDXGIAdapter1>,
    driver_type: D3D_DRIVER_TYPE,
    debug_layer_available: bool,
) -> Result<(ID3D11Device, ID3D11DeviceContext, D3D_FEATURE_LEVEL)> {
    // `D3D11CreateDevice` takes the base `IDXGIAdapter`, the adapters we
    // enumerate are `IDXGIAdapter1`.
    let adapter: Option<IDXGIAdapter> = adapter
        .map(|adapter| adapter.cast())
        .transpose()
        .context("Casting IDXGIAdapter1 to IDXGIAdapter")?;
    let device_flags = if debug_layer_available {
        D3D11_CREATE_DEVICE_BGRA_SUPPORT | D3D11_CREATE_DEVICE_DEBUG
    } else {
        D3D11_CREATE_DEVICE_BGRA_SUPPORT
    };

    let mut device = None;
    let mut device_context = None;
    let mut feature_level = D3D_FEATURE_LEVEL::default();
    let mut last_error = None;
    for (attempt, feature_levels) in [FEATURE_LEVELS, FALLBACK_FEATURE_LEVELS]
        .into_iter()
        .enumerate()
    {
        device = None;
        device_context = None;
        match unsafe {
            D3D11CreateDevice(
                adapter.as_ref(),
                driver_type,
                HMODULE::default(),
                device_flags,
                Some(feature_levels),
                D3D11_SDK_VERSION,
                Some(&mut device),
                Some(&mut feature_level),
                Some(&mut device_context),
            )
        } {
            Ok(()) => {
                if attempt > 0 {
                    log::info!(
                        "Created the device after retrying with the Direct3D 11.0/10.1 feature levels."
                    );
                }
                last_error = None;
                break;
            }
            Err(error) => {
                if attempt == 0 {
                    log::info!(
                        "Creating the device with the Direct3D 11.1 feature level in the list \
                         failed ({error}), retrying with the Direct3D 11.0/10.1 feature levels."
                    );
                }
                last_error = Some(error);
            }
        }
    }

    if let Some(error) = last_error {
        return Err(error).context("Creating the Direct3D 11 device");
    }
    let device = device
        .ok_or_else(|| anyhow::anyhow!("D3D11CreateDevice succeeded but returned no device"))?;
    let device_context = device_context.ok_or_else(|| {
        anyhow::anyhow!("D3D11CreateDevice succeeded but returned no immediate context")
    })?;

    let mut data = D3D11_FEATURE_DATA_D3D10_X_HARDWARE_OPTIONS::default();
    unsafe {
        device
            .CheckFeatureSupport(
                D3D11_FEATURE_D3D10_X_HARDWARE_OPTIONS,
                &mut data as *mut _ as _,
                std::mem::size_of::<D3D11_FEATURE_DATA_D3D10_X_HARDWARE_OPTIONS>() as u32,
            )
            .context("Checking GPU device feature support")?;
    }
    if data
        .ComputeShaders_Plus_RawAndStructuredBuffers_Via_Shader_4_x
        .as_bool()
    {
        Ok((device, device_context, feature_level))
    } else {
        Err(anyhow::anyhow!(
            "Required feature StructuredBuffer is not supported by GPU/driver"
        ))
    }
}

/// Best-effort human readable adapter name for logs and error messages. Never
/// fails: an adapter whose description cannot be read is still usable.
fn adapter_description(adapter: &IDXGIAdapter1) -> String {
    match unsafe { adapter.GetDesc1() } {
        Ok(desc) => String::from_utf16_lossy(&desc.Description)
            .trim_matches(char::from(0))
            .to_string(),
        Err(_) => "<unnamed adapter>".to_string(),
    }
}

#[inline]
fn log_feature_level(feature_level: D3D_FEATURE_LEVEL) {
    match feature_level {
        D3D_FEATURE_LEVEL_11_1 => {
            log::info!("Created device with Direct3D 11.1 feature level.")
        }
        D3D_FEATURE_LEVEL_11_0 => {
            log::info!("Created device with Direct3D 11.0 feature level.")
        }
        D3D_FEATURE_LEVEL_10_1 => {
            log::info!("Created device with Direct3D 10.1 feature level.")
        }
        level => log::info!("Created device with an unexpected Direct3D feature level {level:?}."),
    }
}

#[inline]
fn get_dxgi_factory(debug_layer_available: bool) -> Result<IDXGIFactory6> {
    let factory_flag = if debug_layer_available {
        DXGI_CREATE_FACTORY_DEBUG
    } else {
        DXGI_CREATE_FACTORY_FLAGS::default()
    };
    unsafe { Ok(CreateDXGIFactory2(factory_flag)?) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_layer_is_off_by_default_and_ignores_falsy_values() {
        assert!(!debug_layer_requested(None));
        for value in ["", " ", "0", "false", "FALSE", "False", "no", "NO", " no "] {
            assert!(
                !debug_layer_requested(Some(value)),
                "{DEBUG_LAYER_ENV_VAR}={value:?} must not request the debug layer"
            );
        }
    }

    #[test]
    fn debug_layer_is_requested_by_any_other_value() {
        for value in ["1", "true", "TRUE", "yes", "on", "garbage", " 1 "] {
            assert!(
                debug_layer_requested(Some(value)),
                "{DEBUG_LAYER_ENV_VAR}={value:?} must request the debug layer"
            );
        }
    }

    #[test]
    fn hardware_path_runs_first_unless_warp_is_forced() {
        // Unset: try the hardware adapters, and fall back to WARP if that fails.
        assert_eq!(device_plan(None), DevicePlan::HardwareThenWarp);
        for value in ["", "0", "false", "no"] {
            assert_eq!(
                device_plan(Some(value)),
                DevicePlan::HardwareThenWarp,
                "{FORCE_WARP_ENV_VAR}={value:?} must not force WARP"
            );
        }

        for value in ["1", "true", "YES", "on"] {
            assert_eq!(
                device_plan(Some(value)),
                DevicePlan::WarpOnly,
                "{FORCE_WARP_ENV_VAR}={value:?} must force WARP"
            );
        }
    }

    #[test]
    fn both_failure_reasons_are_reported_when_warp_fails_too() {
        let error = device_creation_failed(
            anyhow::anyhow!("hardware path exploded (DXGI_ERROR_NOT_FOUND)"),
            anyhow::anyhow!("WARP path exploded (E_INVALIDARG)"),
        );
        let message = format!("{error:#}");
        assert!(
            message.contains("hardware path exploded (DXGI_ERROR_NOT_FOUND)"),
            "hardware failure reason missing from: {message}"
        );
        assert!(
            message.contains("WARP path exploded (E_INVALIDARG)"),
            "WARP failure reason missing from: {message}"
        );
    }
}
