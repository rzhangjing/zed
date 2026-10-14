//! CustomDataSource - 一个带有实体集合的基础命名 DataSource。
//!
//! 它把名称、实体集合、时钟与可见性打包成一个可直接交付的可自定义数据源。

use crate::datasource_clock::DataSourceClock;
use crate::entity_collection::EntityCollection;

/// 一个基础的 DataSource，具有名称、实体集合、时钟和可见性。
///
/// 供上层注册自定义内容，实体全部经由内部集合托管，时钟为可选项。
#[derive(Debug)]
pub struct CustomDataSource {
    /// 此数据源的显示名称。
    name: String,
    /// 实体集合。
    entities: EntityCollection,
    /// 与此数据源关联的时钟。
    clock: Option<DataSourceClock>,
    /// 数据源当前是否显示。
    show: bool,
    /// 数据源当前是否正在加载。
    is_loading: bool,
}

impl CustomDataSource {
    /// 创建一个具有给定名称的新 CustomDataSource。
    ///
    /// 映射到 `new CustomDataSource(name)`
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            entities: EntityCollection::new(),
            clock: None,
            show: true,
            is_loading: false,
        }
    }

    /// 获取名称。
    pub fn name(&self) -> &str {
        &self.name
    }

    /// 设置名称。
    pub fn set_name(&mut self, name: &str) {
        self.name = name.to_string();
    }

    /// 获取实体集合。
    pub fn entities(&self) -> &EntityCollection {
        &self.entities
    }

    /// 以可变方式获取实体集合。
    pub fn entities_mut(&mut self) -> &mut EntityCollection {
        &mut self.entities
    }

    /// 获取时钟。
    pub fn clock(&self) -> Option<&DataSourceClock> {
        self.clock.as_ref()
    }

    /// 设置时钟。
    pub fn set_clock(&mut self, clock: Option<DataSourceClock>) {
        self.clock = clock;
    }

    /// 获取数据源是否显示。
    pub fn show(&self) -> bool {
        self.show
    }

    /// 设置数据源是否显示。
    pub fn set_show(&mut self, show: bool) {
        self.show = show;
        self.entities.set_show(show);
    }

    /// 获取数据源是否正在加载。
    pub fn is_loading(&self) -> bool {
        self.is_loading
    }

    /// 设置数据源是否正在加载。
    pub fn set_is_loading(&mut self, is_loading: bool) {
        self.is_loading = is_loading;
    }
}
