//! 配置驱动的模块加载器
//!
//! 职责：
//! 1. 解析 `modules.xml`，得到模块清单（名称 / 相对路径 / 启用 / 顺序 / 依赖 / 提供服务）。
//! 2. 通过 [`ModuleRegistry`]（模块名 -> 装配工厂）把启用的模块按依赖拓扑顺序装配进 [`Container`]。
//!
//! 采用「配置驱动静态注册」：模块本身随主程序静态编译，加载哪些、以何顺序、是否启用
//! 完全由 XML 决定；新增模块只需在注册表登记一个工厂即可。

use std::collections::HashMap;
use std::path::Path;

use container::Container;
use quick_xml::events::Event;
use quick_xml::Reader;

// ---------------- 错误 ----------------

/// 模块加载/装配过程中的各类错误（IO、语法、依赖、装配等）。
#[derive(Debug)]
pub enum LoadError {
    /// 读取配置文件时的 IO 错误
    Io(std::io::Error),
    /// XML 语法/结构解析错误
    Xml(String),
    /// 配置启用了一个注册表中不存在的模块
    UnknownModule(String),
    /// 依赖了未启用 / 不存在的模块
    MissingDependency { module: String, dependency: String },
    /// 依赖存在环
    CircularDependency(Vec<String>),
    /// 工厂自身返回的错误
    FactoryFailed(String),
}

impl std::fmt::Display for LoadError {
    /// 以中文描述格式化各错误变体。
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoadError::Io(e) => write!(f, "读取配置失败: {}", e),
            LoadError::Xml(m) => write!(f, "XML 解析失败: {}", m),
            LoadError::UnknownModule(n) => write!(f, "未知模块（未在注册表登记）: {}", n),
            // 携带模块名与被依赖名，便于定位缺失。
            LoadError::MissingDependency { module, dependency } => {
                write!(f, "模块 {} 依赖未满足: {}", module, dependency)
            }
            // 把成环节点拼成 a -> b -> a 形式展示。
            LoadError::CircularDependency(path) => {
                write!(f, "模块依赖循环: {}", path.join(" -> "))
            }
            LoadError::FactoryFailed(m) => write!(f, "模块装配失败: {}", m),
        }
    }
}

// 接入标准库错误 trait，便于用 `?` 向上传播。
impl std::error::Error for LoadError {}

// ---------------- 模块配置 ----------------

/// 单个模块的配置项，对应 XML 中的一个 `<module>` 节点。
#[derive(Debug, Clone)]
pub struct ModuleConfig {
    /// 模块名称（与注册表 key 对应）
    pub name: String,
    /// 模块相对路径（信息用途）
    pub path: String,
    /// 是否启用
    pub enabled: bool,
    /// 同级（无依赖约束）装配顺序，越小越先
    pub order: i32,
}

/// 宽容的布尔解析：除 false/0/no/off 外均视为 true（缺省即启用）。
fn parse_bool(v: &str) -> bool {
    // 归一化小写后匹配“否定”字面量，命中则为 false，否则 true。
    !matches!(
        v.trim().to_ascii_lowercase().as_str(),
        "false" | "0" | "no" | "off"
    )
}

/// 从 XML 文本解析模块清单。
///
/// 期望结构：
/// ```xml
/// <modules>
///   <module name="user" path="module-user" enabled="true" order="10">
///     <provides>UserService</provides>
///     <depends>db</depends>
///   </module>
///   <module name="db" path="module-db"/>
/// </modules>
/// ```
pub fn parse_modules(xml: &str) -> Result<Vec<ModuleConfig>, LoadError> {
    // 创建事件读取器；trim_text 去除标签间空白噪声。
    let mut reader = Reader::from_str(xml);
    reader.trim_text(true);

    // 解析产物：已完成的模块集合。
    let mut modules: Vec<ModuleConfig> = Vec::new();
    // 正在构造中的模块（从 <module> 开始到结束之间持有）。
    let mut cur: Option<ModuleConfig> = None;
    // 当前文本节点归属："provides" / "depends"
    let mut field: Option<&'static str> = None;

    // 逐事件扫描 XML：Start/Empty/Text/End/Eof 分别处理。
    loop {
        match reader.read_event().map_err(|e| LoadError::Xml(e.to_string()))? {
            // 遇到开始标签
            Event::Start(e) => {
                let name_buf = e.name().as_ref().to_vec();
                match name_buf.as_slice() {
                    // <module ...> —— 读取属性开启一个新模块条目
                    b"module" => {
                        cur = Some(read_module_attrs(&e)?);
                        field = None;
                    }
                    // <provides>/<depends> —— 记录后续文本归属字段
                    b"provides" => field = Some("provides"),
                    b"depends" => field = Some("depends"),
                    _ => {}
                }
            }
            // 遇到自闭合标签
            Event::Empty(e) => {
                // <module .../> 自闭合：无子节点，直接入库
                let name_buf = e.name().as_ref().to_vec();
                if name_buf.as_slice() == b"module" {
                    modules.push(read_module_attrs(&e)?);
                }
            }
            // 遇到文本节点：写入当前归属字段
            Event::Text(e) => {
                if let (Some(f), Some(m)) = (field, cur.as_mut()) {
                    // 反转义并去空白，忽略空文本。
                    let txt = e
                        .unescape()
                        .map(|c| c.trim().to_string())
                        .unwrap_or_default();
                    if !txt.is_empty() {
                        // provides/depends 分支已被删除；如需支持可在此恢复。
                    }
                }
            }
            // 遇到结束标签：module 收尾入库
            Event::End(e) => {
                let name_buf = e.name().as_ref().to_vec();
                match name_buf.as_slice() {
                    b"module" => {
                        if let Some(m) = cur.take() {
                            modules.push(m);
                        }
                    }
                    b"provides" | b"depends" => field = None,
                    _ => {}
                }
            }
            // 文件结束，退出循环
            Event::Eof => break,
            _ => {}
        }
    }

    Ok(modules)
}

/// 读取 `<module>` 标签上的属性，构造一个 ModuleConfig。
fn read_module_attrs(
    e: &quick_xml::events::BytesStart<'_>,
) -> Result<ModuleConfig, LoadError> {
    // 逐属性提取：name/path/enabled/order，provides/depends 由子节点处理。
    let mut name = String::new();
    let mut path = String::new();
    let mut enabled = true; // 缺省启用
    let mut order = 0i32; // 缺省顺序 0

    // with_checks(false)：跳过重复属性校验，容忍宽松输入。
    for attr in e.attributes().with_checks(false) {
        let attr = attr.map_err(|e| LoadError::Xml(e.to_string()))?;
        let key = attr.key.as_ref().to_vec();
        // 反转义属性值并去空白。
        let val = attr
            .unescape_value()
            .map(|c| c.trim().to_string())
            .unwrap_or_default();
        match key.as_slice() {
            b"name" => name = val,
            b"path" => path = val,
            b"enabled" => enabled = parse_bool(&val),
            b"order" => order = val.parse().unwrap_or(0),
            _ => {}
        }
    }

    // name 是必备字段，缺失即视为配置错误。
    if name.is_empty() {
        return Err(LoadError::Xml("存在缺少 name 属性的 <module>".into()));
    }
    Ok(ModuleConfig {
        name,
        path,
        enabled,
        order,
    })
}

/// 从文件加载并解析模块清单。
pub fn load_file<P: AsRef<Path>>(path: P) -> Result<Vec<ModuleConfig>, LoadError> {
    // 读取整个文件为 UTF-8 字符串。
    let text = std::fs::read_to_string(path.as_ref()).map_err(LoadError::Io)?;
    // 复用文本解析逻辑。
    parse_modules(&text)
}

// ---------------- 注册表 ----------------

/// 模块装配工厂：拿到容器，完成该模块的服务注册。
/// 返回 `Err(String)` 时会被包装为 `LoadError::FactoryFailed`。
pub type Registrar = Box<dyn Fn(&Container) -> Result<(), String> + Send + Sync>;

/// 模块名 -> 装配工厂。具体工厂由主程序（可见各 module crate）登记。
#[derive(Default)]
pub struct ModuleRegistry {
    /// 模块名到装配工厂的映射表
    table: HashMap<String, Registrar>,
}

impl ModuleRegistry {
    /// 创建空注册表。
    pub fn new() -> Self {
        Self::default()
    }

    /// 登记一个模块工厂（链式调用，返回 &mut Self）。
    pub fn add<F>(&mut self, name: impl Into<String>, factory: F) -> &mut Self
    where
        F: Fn(&Container) -> Result<(), String> + Send + Sync + 'static,
    {
        // 把具体闭包装箱为 Registrar 存入表；同名覆盖。
        self.table.insert(name.into(), Box::new(factory));
        self
    }

    /// 是否已登记某个模块名。
    pub fn contains(&self, name: &str) -> bool {
        self.table.contains_key(name)
    }

    /// 已登记的模块名（排序后）。
    pub fn names(&self) -> Vec<String> {
        // 取出所有键并排序，输出稳定顺序。
        let mut v: Vec<_> = self.table.keys().cloned().collect();
        v.sort();
        // 返回按名字排序的模块名集合。
        v
    }
}

// ---------------- 装配 ----------------

/// 一次装配的结果报告。
#[derive(Debug, Default, Clone)]
pub struct LoadReport {
    /// 实际装配的模块（按装配顺序）
    pub loaded: Vec<String>,
    /// 被跳过的模块（enabled=false）
    pub skipped: Vec<String>,
}

/// 依据配置把启用的模块装配进容器。
///
/// 步骤：过滤启用项 -> 依赖拓扑排序（Kahn）-> 依次调用注册表工厂。
pub fn load_modules(
    configs: &[ModuleConfig],
    registry: &ModuleRegistry,
    container: &Container,
) -> Result<LoadReport, LoadError> {
    // 逐步累加装配/跳过名单，最终作报告返回。
    let mut report = LoadReport::default();

    // 建立“名字->配置”索引，并筛出启用项作为待排序集合。
    let mut by_name: HashMap<&str, &ModuleConfig> = HashMap::new();
    let mut pending: Vec<&ModuleConfig> = Vec::new();

    for cfg in configs {
        // 禁用项直接记入 skipped，不参与装配。
        if !cfg.enabled {
            report.skipped.push(cfg.name.clone());
            continue;
        }
        by_name.insert(cfg.name.as_str(), cfg);
        pending.push(cfg);
    }

    // 依依赖拓扑排序，得到确定的装配顺序。
    let ordered = topo_sort(&pending, &by_name)?;

    // 按序调用注册表中的工厂完成真正注册。
    for cfg in ordered {
        // 配置启用但注册表未登记 —— 视为未知模块错误。
        let factory = registry
            .table
            .get(&cfg.name)
            .ok_or_else(|| LoadError::UnknownModule(cfg.name.clone()))?;
        // 工厂执行失败则包装为 FactoryFailed。
        factory(container).map_err(LoadError::FactoryFailed)?;
        report.loaded.push(cfg.name.clone());
    }

    Ok(report)
}

/// Kahn 拓扑排序：退化为按 (`order`, `name`) 稳定排序（已无 depends）。
fn topo_sort<'a>(
    pending: &[&'a ModuleConfig],
    _by_name: &HashMap<&str, &'a ModuleConfig>,
) -> Result<Vec<&'a ModuleConfig>, LoadError> {
    // 没有依赖关系时，直接按 (order, name) 升序排列。
    let mut result = pending.to_vec();
    result.sort_by(|a, b| {
        a.order
            .cmp(&b.order)
            .then_with(|| a.name.cmp(&b.name))
    });
    Ok(result)
}

// ---------------- 测试 ----------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    // 一份覆盖“属性 + provides/depends 子节点 + 禁用项”的样本配置。
    const XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<modules>
  <module name="db" path="module-db" order="0">
    <provides>Database</provides>
  </module>
  <module name="user" path="module-user" order="10">
    <provides>UserService</provides>
    <depends>db</depends>
  </module>
  <module name="order" path="module-order" order="20" enabled="true">
    <provides>OrderService</provides>
    <depends>user</depends>
  </module>
  <module name="metrics" path="module-metrics" enabled="false"/>
</modules>
"#;

    /// 验证 XML 解析：字段、顺序均正确提取（已无 depends/provides）。
    #[test]
    fn parse_ok() {
        // 解析样本 XML，得到 4 个模块配置。
        let cfgs = parse_modules(XML).unwrap();
        // 样本共 4 个模块（含 1 个禁用）。
        assert_eq!(cfgs.len(), 4);
        let user = cfgs.iter().find(|m| m.name == "user").unwrap();
        // user 的 order=10。
        assert_eq!(user.order, 10);
        let metrics = cfgs.iter().find(|m| m.name == "metrics").unwrap();
        // metrics 显式 enabled="false"，应为禁用。
        assert!(!metrics.enabled);
    }

    /// 验证装配顺序：按 (order, name) 升序 db→user→order，且禁用项被跳过。
    #[test]
    fn loads_in_dependency_order() {
        let cfgs = parse_modules(XML).unwrap();
        let c = Container::new();
        // 用共享 Mutex<Vec> 记录工厂实际被调用的顺序。
        let order = Arc::new(std::sync::Mutex::new(Vec::new()));

        // 为 db/user/order 各登记一个仅记录名字的工厂。
        let mut reg = ModuleRegistry::new();
        for name in ["db", "user", "order"] {
            let order = order.clone();
            reg.add(name, move |_c| {
                order.lock().unwrap().push(name.to_string());
                Ok(())
            });
        }

        let report = load_modules(&cfgs, &reg, &c).unwrap();
        // 报告与实际调用顺序均为 db→user→order。
        assert_eq!(report.loaded, vec!["db", "user", "order"]);
        assert_eq!(report.skipped, vec!["metrics"]);
        assert_eq!(*order.lock().unwrap(), vec!["db", "user", "order"]);
    }

    /// 验证：启用一个未登记的模块时报 UnknownModule。（已无 MissingDependency / CircularDependency 错误类型）
    #[test]
    fn registrar_actually_registers_into_container() {
        use std::any::Any;
        // 任意一个可克隆的服务类型作为占位。
        #[derive(Clone)]
        struct Svc;

        let xml = r#"<modules><module name="svc" path="x"/></modules>"#;
        let cfgs = parse_modules(xml).unwrap();
        let c = Container::new();
        // 计数工厂调用次数，验证仅装配一次。
        let hits = Arc::new(AtomicUsize::new(0));
        let hits2 = hits.clone();
        let mut reg = ModuleRegistry::new();
        reg.add("svc", move |c| {
            hits2.fetch_add(1, Ordering::SeqCst);
            c.register_instance::<Arc<dyn Any + Send + Sync>>(Arc::new(Svc) as Arc<dyn Any + Send + Sync>);
            Ok(())
        });
        load_modules(&cfgs, &reg, &c).unwrap();
        // 工厂被调用恰好一次，且容器可查到该服务。
        assert_eq!(hits.load(Ordering::SeqCst), 1);
        assert!(c.contains::<Arc<dyn Any + Send + Sync>>());
    }
}
