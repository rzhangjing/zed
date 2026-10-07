use crate::TelemetrySettings;
use gpui::App;
use parking_lot::Mutex;
#[cfg(target_os = "macos")]
use regex::Regex;
use release_channel::ReleaseChannel;
use settings::Settings as _;
use std::sync::LazyLock;
use std::{env, sync::Arc};

pub static MINIDUMP_ENDPOINT: LazyLock<Option<String>> = LazyLock::new(|| {
    option_env!("ZED_MINIDUMP_ENDPOINT")
        .map(str::to_string)
        .or_else(|| env::var("ZED_MINIDUMP_ENDPOINT").ok())
});

pub fn should_install_crash_handler(channel: ReleaseChannel) -> bool {
    matches!(
        env::var("ZED_GENERATE_MINIDUMPS").as_deref(),
        Ok("true" | "1")
    ) || (channel != ReleaseChannel::Dev && MINIDUMP_ENDPOINT.is_some())
}

pub fn os_name() -> String {
    #[cfg(target_os = "macos")]
    {
        "macOS".to_string()
    }
    #[cfg(target_os = "linux")]
    {
        format!("Linux {}", gpui::guess_compositor())
    }
    #[cfg(target_os = "freebsd")]
    {
        format!("FreeBSD {}", gpui::guess_compositor())
    }

    #[cfg(target_os = "windows")]
    {
        "Windows".to_string()
    }
}

/// Note: This might do blocking IO! Only call from background threads
pub fn os_version() -> String {
    cfg_select! {
       feature = "test-support" => {
           // MacOS branch in particular is quite slow, hence we ought to "avoid" it in tests.
           "test binary".to_owned()
       }
       target_os = "macos" => {
           static MACOS_VERSION_REGEX: LazyLock<Regex> = LazyLock::new(|| {
               Regex::new(r"(\s*\(Build [^)]*[0-9]\))").unwrap()
           });
           use objc2_foundation::NSProcessInfo;
           let process_info = NSProcessInfo::processInfo();
           let version_nsstring = process_info.operatingSystemVersionString();
           // "Version 15.6.1 (Build 24G90)" -> "15.6.1 (Build 24G90)"
           let version_string = version_nsstring.to_string().replace("Version ", "");
           // "15.6.1 (Build 24G90)" -> "15.6.1"
           // "26.0.0 (Build 25A5349a)" -> unchanged (Beta or Rapid Security Response; ends with letter)
           MACOS_VERSION_REGEX
               .replace_all(&version_string, "")
               .to_string()
       }
       any(target_os = "linux", target_os = "freebsd") => {
           use std::path::Path;

           let content = if let Ok(file) = std::fs::read_to_string(&Path::new("/etc/os-release")) {
               file
           } else if let Ok(file) = std::fs::read_to_string(&Path::new("/usr/lib/os-release")) {
               file
           } else if let Ok(file) = std::fs::read_to_string(&Path::new("/var/run/os-release")) {
               file
           } else {
               log::error!(
                   "Failed to load /etc/os-release, /usr/lib/os-release, or /var/run/os-release"
               );
               "".to_string()
           };
           util::parse_os_release(&content).unwrap_or_else(|| "unknown".to_string())
       }
       target_os = "windows" => {
           let mut info = unsafe { std::mem::zeroed() };
           let status = unsafe { windows::Wdk::System::SystemServices::RtlGetVersion(&mut info) };
           if status.is_ok() {
               semver::Version::new(
                   info.dwMajorVersion as _,
                   info.dwMinorVersion as _,
                   info.dwBuildNumber as _,
               )
               .to_string()
           } else {
               "unknown".to_string()
           }
       }
    }
}

/// The local identifiers Zed keeps for crash reporting.
///
/// This used to also drive the analytics uploader, which has been removed from
/// this fork; only the locally-known ids and the diagnostics preference remain.
pub struct Telemetry {
    state: Mutex<TelemetryState>,
}

struct TelemetryState {
    diagnostics: bool,
    system_id: Option<Arc<str>>,
    installation_id: Option<Arc<str>>,
}

impl Telemetry {
    pub fn new(cx: &App) -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(TelemetryState {
                diagnostics: TelemetrySettings::get_global(cx).diagnostics,
                system_id: None,
                installation_id: None,
            }),
        })
    }

    pub fn start(
        self: &Arc<Self>,
        system_id: Option<String>,
        installation_id: Option<String>,
        _session_id: String,
        _cx: &mut App,
    ) {
        let mut state = self.state.lock();
        state.system_id = system_id.map(|id| id.into());
        state.installation_id = installation_id.map(|id| id.into());
    }

    pub fn diagnostics_enabled(&self) -> bool {
        self.state.lock().diagnostics
    }

    pub fn system_id(&self) -> Option<Arc<str>> {
        self.state.lock().system_id.clone()
    }

    pub fn installation_id(&self) -> Option<Arc<str>> {
        self.state.lock().installation_id.clone()
    }
}
