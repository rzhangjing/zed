//! # dm-core —— 达梦中间件的配置、连接串与（后续的）值模型
//!
//! 层次：`dm-error ← dm-core ← dm-odbc ← dm-pool ← dm-query ← {dameng, toasty-driver-dameng}`。
//!
//! **硬约束（CI 用 `cargo tree` 强校验）**：本 crate 不依赖 `tokio`，也不依赖 `odbc-api`。
//! 这里只放纯数据结构与纯逻辑，一切 I/O 都在上层。
//!
//! ## 已实现（M0 / M1 / M2）
//!
//! - [`config`] —— [`DmConfig`]、[`DmConfigBuilder`]、[`SecretString`]、[`PoolConfig`]、校验
//! - [`connstr`] —— ODBC 连接串与 `dm://` URL 的渲染/解析，`parse(render(c)) == c`
//! - [`driver_registry`] —— ODBC 驱动注册探测（注册表 / `odbcinst.ini`）+ `DriverNotFound`
//! - [`datatype`] —— ODBC 码值 ↔ [`DataType`]（含 [`DataType::Unknown`] 兜底，**不 panic**）
//! - [`column`](mod@column) —— [`ColumnMeta`] 与「声明长度单位」判定（§4.4 / D16）
//! - [`value`] —— [`DmValue`]/[`ValueRef`] 与 §7 的转换矩阵（[`ConvertError`]）
//! - [`capability`] —— [`Capability`] 能力表与 [`DAMENG`]（§5.10，未声明即 `Unsupported`）
//! - [`backend`] —— [`Backend`] trait 与 [`ExecOutcome`]/[`FetchState`]/[`RowBuf`]，
//!   以及批量写的 [`ParamArrayOutcome`]（§5.11 / §4.7）

#![forbid(unsafe_code)]

pub mod backend;
pub mod capability;
pub mod column;
pub mod config;
pub mod connstr;
pub mod datatype;
pub mod driver_registry;
pub mod value;

pub use backend::{
    Backend, ExecOutcome, FetchState, LOB_WRITE_CHUNK_BYTES, LobBytes, LobChunk, LobKind, LobParam,
    LobReader, LobSource, ParamArrayOutcome, RowBuf,
};

pub use capability::Capability;
pub use capability::{
    AutoIncrementSyntax, ColumnSizeUnit, DAMENG, IsolationLevel, LimitStyle, PlaceholderStyle,
    SupportTier, UpsertSyntax,
};

pub use column::{ColumnMeta, GB18030_MAX_BYTES_PER_CHAR, LengthProbe, interpret_length_in_char};
pub use config::{
    CharsetCheck, CompatMode, DEFAULT_CONNECTION_TIMEOUT, DEFAULT_DRIVER, DEFAULT_LOGIN_TIMEOUT,
    DEFAULT_PORT, DmConfig, DmConfigBuilder, ENV_KEYS, PoolConfig, SecretString,
};
pub use connstr::{
    ALIAS_KEYS, RESERVED_KEYS, STRUCTURED_KEYS, URL_PARAM_KEYS, parse, parse_dm_url,
    parse_odbc_string, render_dm_url, render_odbc_string,
};
pub use datatype::{DataType, IntervalKind, TypeCategory};
pub use driver_registry::{
    DRIVER_NOT_FOUND_HINT, DriverInfo, DriverRegistry, Origin, check_registered,
    looks_like_dm_driver, parse_odbcinst_ini, parse_reg_query_output, probe_cached,
};
pub use value::{ConvertError, DmInterval, DmValue, ValueRef};

#[cfg(test)]
mod integration {
    use super::*;

    /// 端到端：builder → 渲染 → 解析 → 等价。
    #[test]
    fn builder_to_connection_string_and_back() {
        let cfg = DmConfig::builder()
            .server("10.1.2.3")
            .port(5237)
            .user("APP")
            .password("s3cret;with;semicolons")
            .schema("APP")
            .character_code("PG_UTF8")
            .build()
            .expect("构造失败");

        let cs = render_odbc_string(&cfg);
        let back = parse_odbc_string(&cs).expect("解析失败");
        assert_eq!(back, cfg, "连接串 = {cs}");
        assert!(
            cs.contains("PWD={s3cret;with;semicolons};"),
            "口令里的分号必须被保护：{cs}"
        );
    }

    /// 端到端：URL → 解析 → 与 builder 结果一致。
    #[test]
    fn url_and_builder_agree() {
        let from_url = parse("dm://APP:pw@10.1.2.3:5237/APP?character_code=PG_UTF8").unwrap();
        let from_builder = DmConfig::builder()
            .server("10.1.2.3")
            .port(5237)
            .user("APP")
            .password("pw")
            .schema("APP")
            .character_code("PG_UTF8")
            .build()
            .unwrap();
        assert_eq!(from_url, from_builder);
    }

    /// 保底断言：默认配置可渲染、可解析、可校验。
    #[test]
    fn default_config_is_usable_end_to_end() {
        let cfg = DmConfig::default();
        cfg.validate().expect("默认配置必须合法");
        assert_eq!(parse(&render_odbc_string(&cfg)).unwrap(), cfg);
        assert_eq!(parse(&render_dm_url(&cfg)).unwrap(), cfg);
    }
}
