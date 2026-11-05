#![allow(clippy::disallowed_methods, reason = "build scripts are exempt")]
use std::process::Command;

fn main() {
    if cfg!(target_os = "macos") {
        println!("cargo:rustc-env=MACOSX_DEPLOYMENT_TARGET=10.15.7");

        // Weakly link ReplayKit to ensure Zed can be used on macOS 10.15+.
        println!("cargo:rustc-link-arg=-Wl,-weak_framework,ReplayKit");

        // Seems to be required to enable Swift concurrency
        println!("cargo:rustc-link-arg=-Wl,-rpath,/usr/lib/swift");

        // Register exported Objective-C selectors, protocols, etc
        println!("cargo:rustc-link-arg=-Wl,-ObjC");

        // weak link to support Catalina
        println!("cargo:rustc-link-arg=-Wl,-weak_framework,ScreenCaptureKit");
    }

    // Populate git sha environment variable if git is available
    println!("cargo:rerun-if-changed=../../../.git/logs/HEAD");
    println!(
        "cargo:rustc-env=TARGET={}",
        std::env::var("TARGET").unwrap()
    );

    let git_sha = match std::env::var("ZED_COMMIT_SHA").ok() {
        Some(git_sha) => {
            // In deterministic build environments such as Nix, we inject the commit sha into the build script.
            Some(git_sha)
        }
        None => {
            if let Some(output) = Command::new("git")
                .args(["rev-parse", "HEAD"])
                .output()
                .ok()
                && output.status.success()
            {
                let git_sha = String::from_utf8_lossy(&output.stdout);
                Some(git_sha.trim().to_string())
            } else {
                None
            }
        }
    };

    if let Some(git_sha) = git_sha {
        println!("cargo:rustc-env=ZED_COMMIT_SHA={git_sha}");

        if let Some(build_identifier) = option_env!("GITHUB_RUN_NUMBER") {
            println!("cargo:rustc-env=ZED_BUILD_ID={build_identifier}");
        }

        if let Ok(build_profile) = std::env::var("PROFILE")
            && build_profile == "release"
        {
            // This is currently the best way to make `cargo build ...`'s build script
            // to print something to stdout without extra verbosity.
            println!("cargo::warning=Info: using '{git_sha}' hash for ZED_COMMIT_SHA env var");
        }
    }

    if cfg!(windows) {
        if cfg!(target_env = "msvc") {
            // todo(windows): This is to avoid stack overflow. Remove it when solved.
            println!("cargo:rustc-link-arg=/stack:{}", 8 * 1024 * 1024);
            println!("cargo:rustc-link-arg=/DELAYLOAD:windowsperformancerecordercontrol");
            println!("cargo:rustc-link-lib=delayimp");
        } else {
            // 与上面 MSVC 分支等价的 8MB 栈，GNU ld 的写法不同；delay-load 未在 GNU 下复刻。
            println!("cargo:rustc-link-arg=-Wl,--stack,{}", 8 * 1024 * 1024);

            // Windows 加载器拒绝 `SizeOfImage >= 0x78000000`（1.875 GiB）的映像：这样的
            // exe 启动时报「此应用无法在你的电脑上运行」（ERROR_BAD_EXE_FORMAT），
            // 实测 1.850 GiB 可以加载、1.875 GiB 直接被拒（改头字段即可复现）。
            // 合并 Octos 之后，dev 构建把整棵依赖树的 DWARF 一起链进来，已越过该线
            // （0x7EA24000 = 1.978 GiB）。dev 配置下丢掉调试段把映像压回阈值以内；
            // COFF 符号表保留，所以 panic backtrace 仍能解析出符号名。
            // 想要源码级调试信息：改用 `[profile.dev] debug = 1`（只留行表）或 release 配置。
            if std::env::var("PROFILE").as_deref() == Ok("debug") {
                println!("cargo:rustc-link-arg=-Wl,--strip-debug");
            }
        }

        if cfg!(target_arch = "x86_64") || cfg!(target_arch = "aarch64") {
            let out_dir = std::env::var("OUT_DIR").unwrap();
            let out_dir: &std::path::Path = out_dir.as_ref();
            let target_dir = std::path::Path::new(&out_dir)
                .parent()
                .and_then(|p| p.parent())
                .and_then(|p| p.parent())
                .expect("Failed to find target directory");

            let conpty_dll_target = target_dir.join("conpty.dll");
            let open_console_target = target_dir.join("OpenConsole.exe");

            // `conpty.dll` and `OpenConsole.exe` are runtime files: they only
            // have to sit next to the built executable. When both are already
            // there, this build has nothing to fetch and must not touch the
            // network at all -- a stalled github.com request has held a build
            // on this machine for twenty minutes.
            //
            // Assumption: the pinned `conpty_url` below is the only source of
            // these two files, so whatever this script last copied into
            // `target/<profile>` still matches the pin. Bumping the version
            // edits this script and reruns it, but the files already on disk
            // then have to be deleted by hand to pick up the new release. That
            // is the deliberate trade-off: never re-download a binary that is
            // already in place.
            let conpty_staged =
                is_usable_file(&conpty_dll_target) && is_usable_file(&open_console_target);

            if !conpty_staged {
                let conpty_url = "https://github.com/microsoft/terminal/releases/download/v1.24.10621.0/Microsoft.Windows.Console.ConPTY.1.24.260303001.nupkg";
                let nupkg_name = format!(
                    "{}.zip",
                    conpty_url.rsplit('/').next().unwrap_or("conpty.nupkg")
                );
                let extract_dir = out_dir.join("conpty");

                // Keep the archive in `<target>/conpty-cache`, beside the
                // profile directory, instead of in this build's `OUT_DIR`: a
                // second profile, a `cargo clean -p zed` or a deleted
                // `conpty.dll` then restages ConPTY without a download.
                let nupkg_cache_dir = target_dir
                    .parent()
                    .unwrap_or(target_dir)
                    .join("conpty-cache");
                let cached_nupkg = nupkg_cache_dir.join(&nupkg_name);

                // Watch the cache directory only once it exists: naming an
                // absent path would rerun this script on every build.
                if nupkg_cache_dir.is_dir() {
                    println!("cargo:rerun-if-changed={}", nupkg_cache_dir.display());
                }

                // A cached archive is reused as it is. Without one, download
                // into this build's `OUT_DIR` again: a failed attempt leaves a
                // partial file there, which must not pass for an archive.
                let cached_nupkg_hit = is_usable_file(&cached_nupkg);
                let nupkg_path = if cached_nupkg_hit {
                    cached_nupkg.clone()
                } else {
                    out_dir.join("conpty.nupkg.zip")
                };
                let mut have_archive = cached_nupkg_hit;

                if !have_archive {
                    let download_script = format!(
                        "$ProgressPreference = 'SilentlyContinue'; [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12; Invoke-WebRequest -Uri '{}' -OutFile '{}'",
                        conpty_url,
                        nupkg_path.display()
                    );

                    let download_result = Command::new("powershell")
                        .args([
                            "-NoProfile",
                            "-NonInteractive",
                            "-Command",
                            &download_script,
                        ])
                        .output();

                    match download_result {
                        Ok(output) if output.status.success() => {
                            println!("Downloaded conpty nupkg successfully");
                            have_archive = true;

                            // Best effort: a failure to cache the archive only
                            // costs a download on a later build.
                            let cached = std::fs::create_dir_all(&nupkg_cache_dir)
                                .and_then(|()| std::fs::copy(&nupkg_path, &cached_nupkg));
                            if let Err(e) = cached {
                                println!(
                                    "cargo::warning=Failed to cache conpty nupkg at {}: {}",
                                    cached_nupkg.display(),
                                    e
                                );
                            }
                        }
                        Ok(output) => {
                            println!(
                                "cargo::warning=Failed to download conpty nupkg: {}",
                                String::from_utf8_lossy(&output.stderr)
                            );
                        }
                        Err(e) => {
                            println!(
                                "cargo::warning=Failed to run PowerShell for download: {}",
                                e
                            );
                        }
                    }
                }

                // Extract only what is actually available: a failed download
                // leaves nothing to unpack, and keeps the warning above as the
                // only report.
                if have_archive {
                    let extract_script = format!(
                        "$ProgressPreference = 'SilentlyContinue'; Expand-Archive -Path '{}' -DestinationPath '{}' -Force",
                        nupkg_path.display(),
                        extract_dir.display()
                    );

                    let extract_result = Command::new("powershell")
                        .args(["-NoProfile", "-NonInteractive", "-Command", &extract_script])
                        .output();

                    match extract_result {
                        Ok(output) if output.status.success() => {
                            let (conpty_dll_source, open_console_source) =
                                if cfg!(target_arch = "x86_64") {
                                    (
                                        extract_dir.join("runtimes/win-x64/native/conpty.dll"),
                                        extract_dir
                                            .join("build/native/runtimes/x64/OpenConsole.exe"),
                                    )
                                } else {
                                    (
                                        extract_dir.join("runtimes/win-arm64/native/conpty.dll"),
                                        extract_dir
                                            .join("build/native/runtimes/arm64/OpenConsole.exe"),
                                    )
                                };

                            match std::fs::copy(&conpty_dll_source, &conpty_dll_target) {
                                Ok(_) => {
                                    println!("Copied conpty.dll to {}", conpty_dll_target.display())
                                }
                                Err(e) => println!(
                                    "cargo::warning=Failed to copy conpty.dll from {}: {}",
                                    conpty_dll_source.display(),
                                    e
                                ),
                            }

                            match std::fs::copy(&open_console_source, &open_console_target) {
                                Ok(_) => println!(
                                    "Copied OpenConsole.exe to {}",
                                    open_console_target.display()
                                ),
                                Err(e) => println!(
                                    "cargo::warning=Failed to copy OpenConsole.exe from {}: {}",
                                    open_console_source.display(),
                                    e
                                ),
                            }
                        }
                        Ok(output) => {
                            println!(
                                "cargo::warning=Failed to extract conpty nupkg: {}",
                                String::from_utf8_lossy(&output.stderr)
                            );
                        }
                        Err(e) => {
                            println!(
                                "cargo::warning=Failed to run PowerShell for extraction: {}",
                                e
                            );
                        }
                    }
                }
            }
        }

        println!("cargo:rerun-if-env-changed=RELEASE_CHANNEL");
        println!("cargo:rerun-if-env-changed=GITHUB_RUN_NUMBER");

        #[cfg(windows)]
        {
            windows_resources::compile(false).expect("failed to compile Windows resources");
        }
    }

    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    prepare_app_icon_x11();
}

/// Whether `path` is a file with content, that is, a file a previous build
/// finished writing. Used to decide that ConPTY needs no download.
fn is_usable_file(path: &std::path::Path) -> bool {
    std::fs::metadata(path).is_ok_and(|metadata| metadata.is_file() && metadata.len() > 0)
}

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
fn icon_path() -> std::path::PathBuf {
    use std::str::FromStr;

    let release_channel = option_env!("RELEASE_CHANNEL").unwrap_or("dev");
    let channel = match release_channel {
        "stable" => "",
        "preview" => "-preview",
        "nightly" => "-nightly",
        "dev" => "-dev",
        _ => "-dev",
    };

    #[cfg(windows)]
    let icon = format!("resources/windows/app-icon{}.ico", channel);
    #[cfg(not(windows))]
    let icon = format!("resources/app-icon{}.png", channel);

    std::path::PathBuf::from_str(&icon).unwrap()
}

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
fn prepare_app_icon_x11() {
    use image::{ImageReader, imageops};
    use std::env;
    use std::path::Path;

    let out_dir = env::var("OUT_DIR").unwrap();

    let resized_image = ImageReader::open(icon_path())
        .unwrap()
        .decode()
        .unwrap()
        .resize(256, 256, imageops::FilterType::Lanczos3);

    // name should match include_bytes! call in src/zed.rs
    let icon_out_path = Path::new(&out_dir).join("app_icon.png");
    resized_image.save(&icon_out_path).expect("saving app icon");

    println!("cargo:rerun-if-env-changed=RELEASE_CHANNEL");
    println!("cargo:rerun-if-changed={}", icon_path().to_string_lossy());
}
