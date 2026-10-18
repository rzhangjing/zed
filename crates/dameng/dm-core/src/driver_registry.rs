//! ODBC 驱动注册探测。
//!
//! 对应 `docs/设计方案.md` §5.5 连接建立序列的**步骤 ②**与 §6.2 的
//! `DriverNotFound { tried, hint }`。
//!
//! # 为什么要单独做一次探测
//!
//! 直接用连接串 `DRIVER={DM8 ODBC DRIVER}` 去连时，驱动不存在只会得到一个
//! `IM002 未发现数据源名称并且未指定默认驱动程序`——这句话对排查毫无帮助
//! （本机实测：`spike/odbc-smoke` 用 `DSN=DM8` 连接即得此错）。
//! 探测一次注册表/`odbcinst.ini`，就能把「驱动没装」与「连不上服务端」区分开，
//! 并给出可行动的三条建议。
//!
//! # 依赖约束
//!
//! 本模块属于 `dm-core`，**不得依赖 `odbc-api`**（§5.3 CI 强校验）。
//! 因此：
//!
//! * Windows 走 `reg.exe query`（系统自带，无需 `windows-sys` 之类的绑定 crate）；
//! * Linux/麒麟 直接读 unixODBC 的 `odbcinst.ini`；
//! * 两个平台的**解析逻辑都是纯函数**，可在任意平台上用单测覆盖（见本文件 `tests`）。
//!
//! 进程外调用只发生在 `probe()` 里、每次进程至多一次，且带上 `CREATE_NO_WINDOW`
//! 以免在 GUI 进程里闪出控制台窗口。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use dm_error::{DmError, DmErrorKind, Stage};

/// 探测失败时给出的建议（逐字取自设计方案 §6.2）。
pub const DRIVER_NOT_FOUND_HINT: &str = "未找到达梦 ODBC 驱动。\n\
     · Windows：确认已安装达梦客户端（注册表应有 DM8 ODBC DRIVER 项）。\n\
     · 麒麟/Linux：需安装 unixODBC 并在 /etc/odbcinst.ini 注册 DM8 ODBC DRIVER。\n\
     · 也可以用无 DSN 连接串直接指定 DRIVER={DM8 ODBC DRIVER}。";

/// 条目来自哪里。保留来源是为了让「同一个驱动名在两处注册了不同路径」这类
/// 配置事故可见。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Origin {
    /// Windows 注册表键。
    WindowsRegistry {
        /// 例如 `HKLM\SOFTWARE\ODBC\ODBCINST.INI`。
        key: String,
    },
    /// unixODBC 的 `odbcinst.ini`。
    OdbcInstIni {
        /// 实际读到的文件路径。
        path: PathBuf,
    },
}

impl std::fmt::Display for Origin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Origin::WindowsRegistry { key } => write!(f, "注册表 {key}"),
            Origin::OdbcInstIni { path } => write!(f, "{}", path.display()),
        }
    }
}

/// 一条已注册的 ODBC 驱动。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriverInfo {
    /// 注册名，如 `DM8 ODBC DRIVER`。比较时不区分大小写。
    pub name: String,
    /// `Driver` 值：真正被加载的驱动库。
    pub driver_path: Option<String>,
    /// `Setup` 值：配置对话框用的库。
    pub setup_path: Option<String>,
    /// 该条目从哪里读来。
    pub origin: Origin,
}

/// 一次探测的完整结果：找到了什么 + 找过哪里。
///
/// `tried` 是给用户看的，所以它包含「查了哪个键/哪个文件」以及
/// 「为什么没读到」（文件不存在 / 命令不可用），而不是只记成功项。
#[derive(Debug, Clone, Default)]
pub struct DriverRegistry {
    entries: Vec<DriverInfo>,
    tried: Vec<String>,
}

impl DriverRegistry {
    /// 探测本机的 ODBC 驱动注册情况。
    ///
    /// 不会失败：读不到任何东西也是一种结果（`entries` 为空、`tried` 记录原因），
    /// 由 [`DriverRegistry::require`] 决定要不要报错。
    #[must_use]
    pub fn probe() -> Self {
        let mut reg = Self::default();
        #[cfg(windows)]
        reg.probe_windows();
        #[cfg(not(windows))]
        reg.probe_unix();
        reg
    }

    /// 用**预先准备好的文本**构造注册表：纯逻辑、可跨平台单测。
    ///
    /// * `reg_outputs` —— `(来源标签, reg query 输出)`
    /// * `ini_files` —— `(路径, 文件内容)`
    ///
    /// 同名驱动以**先出现的来源为准**（调用方按「注册表在前、ini 在后」传入）。
    #[must_use]
    pub fn from_texts(reg_outputs: &[(String, String)], ini_files: &[(PathBuf, String)]) -> Self {
        let mut reg = Self::default();
        for (label, text) in reg_outputs {
            reg.tried.push(format!("注册表 {label}"));
            reg.absorb(parse_reg_query_output(text), |_| Origin::WindowsRegistry {
                key: label.clone(),
            });
        }
        for (path, text) in ini_files {
            reg.tried.push(path.display().to_string());
            reg.absorb(parse_odbcinst_ini(text), |_| Origin::OdbcInstIni {
                path: path.clone(),
            });
        }
        reg.finish();
        reg
    }

    fn absorb(
        &mut self,
        parsed: Vec<(String, BTreeMap<String, String>)>,
        origin_of: impl Fn(&str) -> Origin,
    ) {
        for (name, kv) in parsed {
            if self.find(&name).is_some() {
                continue; // 先出现的来源优先
            }
            let pick = |key: &str| {
                kv.iter()
                    .find(|(k, _)| k.eq_ignore_ascii_case(key))
                    .map(|(_, v)| v.clone())
            };
            self.entries.push(DriverInfo {
                name: name.clone(),
                driver_path: pick("Driver"),
                setup_path: pick("Setup"),
                origin: origin_of(&name),
            });
        }
    }

    fn finish(&mut self) {
        self.entries.sort_by(|a, b| {
            a.name
                .to_ascii_uppercase()
                .cmp(&b.name.to_ascii_uppercase())
        });
        self.tried.dedup();
    }

    /// 已注册的驱动条目。
    #[must_use]
    pub fn entries(&self) -> &[DriverInfo] {
        &self.entries
    }

    /// 本次探测查过的地方（含失败原因）。
    #[must_use]
    pub fn tried(&self) -> &[String] {
        &self.tried
    }

    /// 按名查找，**不区分大小写**。
    #[must_use]
    pub fn find(&self, name: &str) -> Option<&DriverInfo> {
        let want = name.trim();
        self.entries
            .iter()
            .find(|e| e.name.eq_ignore_ascii_case(want))
    }

    /// 已注册的驱动名列表，用于错误信息里「你是不是想用其中之一」。
    #[must_use]
    pub fn names(&self) -> Vec<&str> {
        self.entries.iter().map(|e| e.name.as_str()).collect()
    }

    /// 要求指定驱动已注册，否则给出结构化错误。
    ///
    /// `tried` 里会带上**本次真实查过的地方**以及「本机已有的驱动名」，
    /// 这比只回一句「没找到」有用得多。
    pub fn require(&self, name: &str) -> Result<&DriverInfo, DmError> {
        if name.trim().is_empty() {
            return Err(DmError::at(
                DmErrorKind::Config("ODBC 驱动名为空".to_string()),
                Stage::Config,
            ));
        }
        if let Some(found) = self.find(name) {
            return Ok(found);
        }

        let mut tried = self.tried.clone();
        if self.entries.is_empty() {
            tried.push("本机未注册任何 ODBC 驱动".to_string());
        } else {
            tried.push(format!("本机已注册：{}", self.names().join(", ")));
        }
        tried.push(format!("请求的驱动名：{name}"));
        Err(DmError::at(
            DmErrorKind::DriverNotFound {
                tried,
                hint: DRIVER_NOT_FOUND_HINT.to_string(),
            },
            Stage::DriverLoad,
        ))
    }

    // ---------------------------------------------------------------- 平台探测

    /// Windows：查注册表的 64 位与 32 位视图。
    ///
    /// 两个视图都要查：达梦客户端若是 32 位安装，只会写进 `WOW6432Node`。
    #[cfg(windows)]
    fn probe_windows(&mut self) {
        for key in [
            r"HKLM\SOFTWARE\ODBC\ODBCINST.INI",
            r"HKLM\SOFTWARE\WOW6432Node\ODBC\ODBCINST.INI",
        ] {
            match run_reg_query(key) {
                Ok(text) => {
                    self.tried.push(format!("注册表 {key}"));
                    self.absorb(parse_reg_query_output(&text), |_| Origin::WindowsRegistry {
                        key: key.to_string(),
                    });
                }
                Err(why) => self.tried.push(format!("注册表 {key}：{why}")),
            }
        }
        self.finish();
    }

    /// Linux/麒麟：读 unixODBC 的 `odbcinst.ini`。
    ///
    /// 查找顺序遵循 unixODBC 自己的规则：`ODBCSYSINI` 覆盖目录、`ODBCINSTINI`
    /// 覆盖文件名，默认 `/etc/odbcinst.ini`。额外再看一眼数据源文件
    /// （`ODBCINI`，默认 `/etc/odbc.ini`）——它的存在与否能区分
    /// 「unixODBC 装了但没注册 DM 驱动」与「unixODBC 根本没装」。
    #[cfg(not(windows))]
    fn probe_unix(&mut self) {
        let mut candidates: Vec<PathBuf> = Vec::new();
        let file_name = std::env::var("ODBCINSTINI").unwrap_or_else(|_| "odbcinst.ini".to_string());
        if let Some(dir) = std::env::var_os("ODBCSYSINI") {
            candidates.push(PathBuf::from(dir).join(&file_name));
        }
        candidates.push(PathBuf::from("/etc/odbcinst.ini"));
        candidates.push(PathBuf::from("/usr/local/etc/odbcinst.ini"));
        candidates.push(PathBuf::from("/etc/unixODBC/odbcinst.ini"));

        let mut any_file = false;
        for path in candidates {
            match std::fs::read_to_string(&path) {
                Ok(text) => {
                    any_file = true;
                    self.tried.push(path.display().to_string());
                    self.absorb(parse_odbcinst_ini(&text), |_| Origin::OdbcInstIni {
                        path: path.clone(),
                    });
                }
                Err(e) => self.tried.push(format!("{}：{e}", path.display())),
            }
        }

        let dsn = std::env::var_os("ODBCINI")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/etc/odbc.ini"));
        self.tried.push(match std::fs::metadata(&dsn) {
            Ok(_) => format!("{}（数据源文件，存在）", dsn.display()),
            Err(_) => format!("{}（数据源文件，不存在）", dsn.display()),
        });

        if !any_file {
            self.tried
                .push("unixODBC 未安装（未找到任何 odbcinst.ini）".to_string());
        }
        self.finish();
    }
}

// -------------------------------------------------------------------- 纯解析器

/// 解析 `reg.exe query <key> /s` 的输出。
///
/// 形如：
///
/// ```text
/// HKEY_LOCAL_MACHINE\SOFTWARE\ODBC\ODBCINST.INI\DM8 ODBC DRIVER
///     Driver    REG_SZ    D:\dmdbms\bin\dodbc.dll
///     Setup    REG_SZ    D:\dmdbms\bin\dodbc.dll
/// ```
///
/// 键路径取**最后一段**作为驱动名；没有值行的键被丢弃（那只是空壳键）。
/// 值内部允许含空格（`C:\Program Files\...`），所以只把前两段当
/// `NAME`/`TYPE`，其余整段都是值。
#[must_use]
pub fn parse_reg_query_output(text: &str) -> Vec<(String, BTreeMap<String, String>)> {
    let mut out: Vec<(String, BTreeMap<String, String>)> = Vec::new();
    for raw in text.lines() {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        if line.trim().is_empty() {
            continue;
        }
        if !line.starts_with(char::is_whitespace) {
            // 键行：顶格、以 HKEY_ 开头。
            if let Some(rest) = line.strip_prefix("HKEY_") {
                let name = rest.rsplit('\\').next().unwrap_or(rest).trim();
                if !name.is_empty() {
                    out.push((name.to_string(), BTreeMap::new()));
                }
            }
            continue;
        }
        // 值行：缩进 + `NAME  TYPE  VALUE`。
        let Some((_, kv)) = out.last_mut() else {
            continue; // 值行出现在任何键之前 ⇒ 忽略
        };
        let trimmed = line.trim_start();
        let Some((key, rest)) = trimmed.split_once(char::is_whitespace) else {
            continue;
        };
        let rest = rest.trim_start();
        let value = match rest.split_once(char::is_whitespace) {
            Some((_ty, v)) => v.trim_start().to_string(),
            // 只有 `NAME TYPE`、没有值：也算一个空字符串值
            None => String::new(),
        };
        kv.insert(key.to_string(), value);
    }
    out.retain(|(_, kv)| !kv.is_empty());
    out
}

/// 解析 unixODBC 的 `odbcinst.ini` / `odbc.ini`。
///
/// ```ini
/// [DM8 ODBC DRIVER]
/// Driver      = /opt/dmdbms/bin/libdodbc.so
/// Setup       = /opt/dmdbms/bin/libdodbc.so
/// ```
///
/// `#` 与 `;` 开头为注释；值两侧空白被裁掉；节名大小写原样保留。
/// 出现在任何节之前的键被丢弃。
#[must_use]
pub fn parse_odbcinst_ini(text: &str) -> Vec<(String, BTreeMap<String, String>)> {
    let mut out: Vec<(String, BTreeMap<String, String>)> = Vec::new();
    for raw in text.lines() {
        let line = raw.strip_suffix('\r').unwrap_or(raw).trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if let Some(rest) = line.strip_prefix('[') {
            if let Some(name) = rest.strip_suffix(']') {
                let name = name.trim();
                if !name.is_empty() {
                    out.push((name.to_string(), BTreeMap::new()));
                }
            }
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        if let Some((_, kv)) = out.last_mut() {
            kv.insert(k.trim().to_string(), v.trim().to_string());
        }
    }
    out.retain(|(_, kv)| !kv.is_empty());
    out
}

/// 供调用方判断某个路径是否像达梦驱动库。**仅用于诊断信息**，不做校验。
#[must_use]
pub fn looks_like_dm_driver(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
        return false;
    };
    let n = name.to_ascii_lowercase();
    n.contains("dodbc") || n.contains("dmdpi")
}

/// 进程级缓存。`probe()` 在 Windows 上会 spawn `reg.exe`，不该反复来；
/// 驱动注册情况在一个进程的生命周期内也不会变。
#[must_use]
pub fn probe_cached() -> &'static DriverRegistry {
    static CACHE: std::sync::OnceLock<DriverRegistry> = std::sync::OnceLock::new();
    CACHE.get_or_init(DriverRegistry::probe)
}

/// 连接建立序列（§5.5）的**步骤 ②**：按配置里的驱动名校验本机注册情况。
///
/// 返回已注册的条目；未注册则给 [`DmErrorKind::DriverNotFound`]。
/// 注意这只是**预检**——它不保证服务端可达。设计上允许用户跳过：
/// 无 DSN 连接串直接指定 `DRIVER=...` 时，ODBC 自己会给出错误。
pub fn check_registered(cfg: &crate::config::DmConfig) -> Result<&'static DriverInfo, DmError> {
    probe_cached().require(&cfg.driver)
}

// ------------------------------------------------------------------ Windows 执行

/// 执行 `reg.exe query <key> /s`。
#[cfg(windows)]
fn run_reg_query(key: &str) -> Result<String, String> {
    use std::os::windows::process::CommandExt;
    /// `CREATE_NO_WINDOW`：GUI 进程里不闪控制台。
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let out = std::process::Command::new("reg.exe")
        .args(["query", key, "/s"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| format!("无法执行 reg.exe：{e}"))?;
    if out.status.success() {
        return String::from_utf8(out.stdout).map_err(|e| format!("reg.exe 输出不是 UTF-8：{e}"));
    }
    // 键不存在时 reg.exe 返回非 0，这是**正常**情况，当作「该视图没有驱动」。
    let msg = String::from_utf8_lossy(&out.stderr).trim().to_string();
    Err(if msg.is_empty() {
        "键不存在".to_string()
    } else {
        msg
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const REG_SAMPLE: &str = "\r\nHKEY_LOCAL_MACHINE\\SOFTWARE\\ODBC\\ODBCINST.INI\\DM8 ODBC DRIVER\r\n    Driver    REG_SZ    D:\\dmdbms\\bin\\dodbc.dll\r\n    Setup    REG_SZ    D:\\dmdbms\\bin\\dodbc.dll\r\n\r\nHKEY_LOCAL_MACHINE\\SOFTWARE\\ODBC\\ODBCINST.INI\\SQL Server\r\n    Driver    REG_SZ    C:\\Windows\\System32\\sqlsrv32.dll\r\n    UsageCount    REG_DWORD    0x2\r\n";

    fn reg_sample_registry() -> DriverRegistry {
        DriverRegistry::from_texts(
            &[(
                r"HKLM\SOFTWARE\ODBC\ODBCINST.INI".to_string(),
                REG_SAMPLE.to_string(),
            )],
            &[],
        )
    }

    #[test]
    fn reg_output_parses_driver_and_setup() {
        let parsed = parse_reg_query_output(REG_SAMPLE);
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].0, "DM8 ODBC DRIVER");
        assert_eq!(
            parsed[0].1.get("Driver").map(String::as_str),
            Some(r"D:\dmdbms\bin\dodbc.dll")
        );
        assert_eq!(
            parsed[0].1.get("Setup").map(String::as_str),
            Some(r"D:\dmdbms\bin\dodbc.dll")
        );
        assert_eq!(parsed[1].0, "SQL Server");
    }

    #[test]
    fn reg_output_keeps_spaces_inside_value() {
        let text = "HKEY_LOCAL_MACHINE\\A\\B\\Prog Files\r\n    Driver    REG_SZ    C:\\Program Files\\x\\d.dll\r\n";
        let parsed = parse_reg_query_output(text);
        assert_eq!(parsed[0].0, "Prog Files");
        assert_eq!(
            parsed[0].1.get("Driver").map(String::as_str),
            Some(r"C:\Program Files\x\d.dll")
        );
    }

    #[test]
    fn reg_output_tolerates_empty_and_garbage() {
        assert!(parse_reg_query_output("").is_empty());
        assert!(parse_reg_query_output("随便一行没有 HKEY\r\n").is_empty());
        // 只有键、没有值 ⇒ 空壳键，丢弃
        assert!(parse_reg_query_output("HKEY_LOCAL_MACHINE\\A\\B\r\n").is_empty());
        // 值行出现在键之前 ⇒ 忽略
        assert!(parse_reg_query_output("    Driver REG_SZ x\r\n").is_empty());
    }

    #[test]
    fn ini_parses_sections_and_ignores_comments() {
        let text = "\
# 注释
; 也是注释
[DM8 ODBC DRIVER]
Driver = /opt/dmdbms/bin/libdodbc.so
Setup  = /opt/dmdbms/bin/libdodbc.so

[Other Driver]
Driver=/x/libodbc.so
";
        let parsed = parse_odbcinst_ini(text);
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].0, "DM8 ODBC DRIVER");
        assert_eq!(
            parsed[0].1.get("Driver").map(String::as_str),
            Some("/opt/dmdbms/bin/libdodbc.so")
        );
        assert_eq!(parsed[1].0, "Other Driver");
    }

    #[test]
    fn ini_without_section_header_is_ignored() {
        assert!(parse_odbcinst_ini("Driver=/x.so\n").is_empty());
    }

    #[test]
    fn find_is_case_insensitive_and_trims() {
        let reg = reg_sample_registry();
        assert!(reg.find("dm8 odbc driver").is_some());
        assert!(reg.find("  DM8 ODBC DRIVER  ").is_some());
        assert!(reg.find("DM8").is_none());
        assert_eq!(reg.names(), vec!["DM8 ODBC DRIVER", "SQL Server"]);
    }

    #[test]
    fn require_ok_returns_entry_with_origin() {
        let reg = reg_sample_registry();
        let got = reg.require("DM8 ODBC DRIVER").unwrap();
        assert_eq!(
            got.origin,
            Origin::WindowsRegistry {
                key: r"HKLM\SOFTWARE\ODBC\ODBCINST.INI".to_string()
            }
        );
        assert_eq!(got.driver_path.as_deref(), Some(r"D:\dmdbms\bin\dodbc.dll"));
    }

    #[test]
    fn require_missing_gives_driver_not_found_with_tried_and_names() {
        let reg = reg_sample_registry();
        let err = reg.require("Nope").unwrap_err();
        assert_eq!(err.context.stage, Stage::DriverLoad);
        match &err.kind {
            DmErrorKind::DriverNotFound { tried, hint } => {
                assert!(tried.iter().any(|t| t.contains("本机已注册：")));
                assert!(tried.iter().any(|t| t.contains("请求的驱动名：Nope")));
                assert!(hint.contains("unixODBC"));
                // 只要探测到了别的驱动，就不该说「未注册任何 ODBC 驱动」
                assert!(!tried.iter().any(|t| t.contains("未注册任何")));
            }
            other => panic!("期望 DriverNotFound，实为 {other:?}"),
        }
        // 驱动没装属于「不该重试」的类别
        assert!(!err.is_retryable());
        assert!(!err.kind.is_connection_lost());
    }

    #[test]
    fn require_missing_on_empty_registry_says_so() {
        let reg = DriverRegistry::from_texts(&[], &[]);
        let err = reg.require("DM8 ODBC DRIVER").unwrap_err();
        match &err.kind {
            DmErrorKind::DriverNotFound { tried, .. } => {
                assert!(tried.iter().any(|t| t.contains("未注册任何 ODBC 驱动")));
            }
            other => panic!("期望 DriverNotFound，实为 {other:?}"),
        }
    }

    #[test]
    fn require_empty_name_is_config_error() {
        let reg = DriverRegistry::from_texts(&[], &[]);
        let err = reg.require("   ").unwrap_err();
        assert_eq!(err.context.stage, Stage::Config);
        assert!(matches!(err.kind, DmErrorKind::Config(_)));
    }

    #[test]
    fn registry_wins_over_ini_for_same_name() {
        let reg = DriverRegistry::from_texts(
            &[(
                r"HKLM\SOFTWARE\ODBC\ODBCINST.INI".to_string(),
                REG_SAMPLE.to_string(),
            )],
            &[(
                PathBuf::from("/etc/odbcinst.ini"),
                "[DM8 ODBC DRIVER]\nDriver=/opt/dmdbms/bin/libdodbc.so\n".to_string(),
            )],
        );
        let got = reg.find("DM8 ODBC DRIVER").unwrap();
        assert!(matches!(got.origin, Origin::WindowsRegistry { .. }));
        assert_eq!(got.driver_path.as_deref(), Some(r"D:\dmdbms\bin\dodbc.dll"));
    }

    #[test]
    fn ini_only_registry_records_path_in_tried() {
        let reg = DriverRegistry::from_texts(
            &[],
            &[(
                PathBuf::from("/etc/odbcinst.ini"),
                "[DM8 ODBC DRIVER]\nDriver=/opt/dmdbms/bin/libdodbc.so\nSetup=/opt/dmdbms/bin/libdodbc.so\n"
                    .to_string(),
            )],
        );
        assert!(reg.tried().iter().any(|t| t == "/etc/odbcinst.ini"));
        let got = reg.find("DM8 ODBC DRIVER").unwrap();
        assert_eq!(
            got.origin,
            Origin::OdbcInstIni {
                path: PathBuf::from("/etc/odbcinst.ini")
            }
        );
    }

    #[test]
    fn looks_like_dm_driver_recognises_dm_libraries() {
        assert!(looks_like_dm_driver(Path::new(r"D:\dmdbms\bin\dodbc.dll")));
        assert!(looks_like_dm_driver(Path::new(
            "/opt/dmdbms/bin/libdodbc.so"
        )));
        assert!(looks_like_dm_driver(Path::new("libdmdpi.so")));
        assert!(!looks_like_dm_driver(Path::new("sqlsrv32.dll")));
        assert!(!looks_like_dm_driver(Path::new("/")));
    }

    /// 真机探测。默认忽略：CI 机器上不一定装了达梦客户端。
    /// 本机跑：`cargo test -p dm-core -- --ignored driver_registry_live --nocapture`
    #[test]
    #[ignore = "需要本机已安装达梦客户端并注册 ODBC 驱动"]
    fn driver_registry_live() {
        let reg = DriverRegistry::probe();
        eprintln!("tried = {:#?}", reg.tried());
        eprintln!("names = {:?}", reg.names());
        let found = reg
            .require("DM8 ODBC DRIVER")
            .expect("本机应已注册 DM8 ODBC DRIVER");
        assert!(
            found
                .driver_path
                .as_deref()
                .is_some_and(|p| looks_like_dm_driver(Path::new(p))),
            "Driver 值应指向达梦驱动库，实为 {:?}",
            found.driver_path
        );
    }
}
