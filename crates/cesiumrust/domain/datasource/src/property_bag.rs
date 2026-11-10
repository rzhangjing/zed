//! PropertyBag - 一个动态键值属性容器。
//!
//! 它以有序的属性名列表配合一张按名称索引的属性表，保存一组动态属性；
//! 求值时逐属性取回当前时刻的计算值并组装为键值映射，常用于承载实体的
//! 自定义扩展属性（如描述、示例等）。

use crate::property_system::property::{ConstantProperty, DynProperty};
use crate::property_system::value::PropertyValue;
use cesium_time::JulianDate;
use std::collections::HashMap;
use std::sync::Arc;

/// 一个属性，其值是属性名到其他属性计算值之间的键值映射。
///
/// 属性名以 `property_names` 保序，实际值存放在 `properties` 表中；
/// 两者共同决定容器的内容与遍历顺序，克隆时深拷贝整份名称与映射，
/// 求值则按名称逐个取回其当前时刻的计算值。
#[derive(Clone)]
pub struct PropertyBag {
    /// 有序的属性名。
    property_names: Vec<String>,
    /// 按名称索引的属性值。
    properties: HashMap<String, Arc<dyn DynProperty>>,
}

impl PropertyBag {
    /// 创建新的空 PropertyBag。
    pub fn new() -> Self {
        Self {
            property_names: Vec::new(),
            properties: HashMap::new(),
        }
    }

    /// 由一组键值对创建 PropertyBag，其中值是
    /// 被包裹在 ConstantProperty 中的原始值。
    ///
    /// 映射到 `new PropertyBag({a: 1, b: 2})`
    pub fn from_values(values: &[(&str, PropertyValue)]) -> Self {
        let mut bag = Self::new();
        for (name, value) in values {
            let prop = ConstantProperty::new(value.clone());
            bag.add_property_with(name, Arc::new(prop));
        }
        bag
    }

    /// 由现有属性创建 PropertyBag。
    pub fn from_properties(props: &[(&str, Arc<dyn DynProperty>)]) -> Self {
        let mut bag = Self::new();
        for (name, prop) in props {
            bag.add_property_with(name, Arc::clone(prop));
        }
        bag
    }

    /// 获取此实例上注册的所有属性名。
    /// 映射到 `PropertyBag.prototype.propertyNames`
    pub fn property_names(&self) -> &[String] {
        &self.property_names
    }

    /// 若此属性为常量（所有成员均为常量）则返回 true。
    /// 映射到 `PropertyBag.prototype.isConstant`
    pub fn is_constant(&self) -> bool {
        self.property_names.iter().all(|name| {
            match self.properties.get(name) {
                Some(prop) => prop.is_constant(),
                None => true,
            }
        })
    }

    /// 判断此对象是否定义了具有给定名称的属性。
    /// 映射到 `PropertyBag.prototype.hasProperty`
    pub fn has_property(&self, property_name: &str) -> bool {
        self.property_names.contains(&property_name.to_string())
    }

    /// 添加一个无值的属性。
    /// 映射到 `PropertyBag.prototype.addProperty(name)`
    pub fn add_property(&mut self, property_name: &str) {
        assert!(
            !property_name.is_empty(),
            "propertyName is required."
        );
        assert!(
            !self.property_names.contains(&property_name.to_string()),
            "{property_name} is already a registered property."
        );
        self.property_names.push(property_name.to_string());
    }

    /// 添加一个带属性值的属性。
    /// 映射到 `PropertyBag.prototype.addProperty(name, value)`
    pub fn add_property_with(&mut self, property_name: &str, value: Arc<dyn DynProperty>) {
        assert!(
            !property_name.is_empty(),
            "propertyName is required."
        );
        assert!(
            !self.property_names.contains(&property_name.to_string()),
            "{property_name} is already a registered property."
        );
        self.property_names.push(property_name.to_string());
        self.properties.insert(property_name.to_string(), value);
    }

    /// 添加一个带原始值的属性（包裹在 ConstantProperty 中）。
    /// 映射到 `PropertyBag.prototype.addProperty(name, rawValue)`
    pub fn add_property_value(&mut self, property_name: &str, value: PropertyValue) {
        let prop = ConstantProperty::new(value);
        self.add_property_with(property_name, Arc::new(prop));
    }

    /// 移除之前通过 addProperty 添加的属性。
    /// 映射到 `PropertyBag.prototype.removeProperty`
    pub fn remove_property(&mut self, property_name: &str) {
        assert!(
            !property_name.is_empty(),
            "propertyName is required."
        );
        let index = self
            .property_names
            .iter()
            .position(|n| n == property_name);
        assert!(
            index.is_some(),
            "{property_name} is not a registered property."
        );
        let index = index.unwrap();
        self.property_names.remove(index);
        self.properties.remove(property_name);
    }

    /// 获取具有给定名称的属性。
    pub fn get_property(&self, property_name: &str) -> Option<&Arc<dyn DynProperty>> {
        self.properties.get(property_name)
    }

    /// 为已存在的属性名设置属性值。
    pub fn set_property(&mut self, property_name: &str, value: Arc<dyn DynProperty>) {
        assert!(
            self.property_names.contains(&property_name.to_string()),
            "{property_name} is not a registered property."
        );
        self.properties.insert(property_name.to_string(), value);
    }

    /// 获取此属性在给定时间处的值。
    /// 其中包含的每个属性都在给定时间处求值，整体
    /// 结果是属性名到这些值的映射。
    ///
    /// 映射到 `PropertyBag.prototype.getValue`
    pub fn get_value(&self, time: &JulianDate) -> HashMap<String, PropertyValue> {
        let mut result = HashMap::new();
        for name in &self.property_names {
            let value = match self.properties.get(name) {
                Some(prop) => prop.get_value(time),
                None => PropertyValue::Undefined,
            };
            result.insert(name.clone(), value);
        }
        result
    }

    /// 获取值，并合并到已有的结果映射中。
    /// result 中不属于此 PropertyBag 的属性保持原样。
    pub fn get_value_with_result(
        &self,
        time: &JulianDate,
        result: &mut HashMap<String, PropertyValue>,
    ) {
        for name in &self.property_names {
            let value = match self.properties.get(name) {
                Some(prop) => prop.get_value(time),
                None => PropertyValue::Undefined,
            };
            result.insert(name.clone(), value);
        }
    }

    /// 从源为此对象上的每个未赋值属性赋值。
    /// 映射到 `PropertyBag.prototype.merge`
    pub fn merge(&mut self, source: &PropertyBag) {
        for name in &source.property_names {
            if !self.property_names.contains(name) {
                self.property_names.push(name.clone());
            }
            if let Some(prop) = source.properties.get(name) {
                self.properties.insert(name.clone(), Arc::clone(prop));
            }
        }
    }

    /// 将此属性与提供的属性进行比较。
    /// 映射到 `PropertyBag.prototype.equals`
    pub fn equals(&self, other: &PropertyBag) -> bool {
        if self.property_names.len() != other.property_names.len() {
            return false;
        }
        for name in &self.property_names {
            if !other.property_names.contains(name) {
                return false;
            }
            let self_prop = self.properties.get(name);
            let other_prop = other.properties.get(name);
            match (self_prop, other_prop) {
                (Some(a), Some(b)) => {
                    if !a.equals(b.as_ref()) {
                        return false;
                    }
                }
                (None, None) => {}
                _ => return false,
            }
        }
        true
    }
}

impl Default for PropertyBag {
    /// 构造一个不含任何属性的空容器。
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for PropertyBag {
    /// 仅输出属性名列表，避免递归打印潜在自引用的属性值。
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PropertyBag")
            .field("property_names", &self.property_names)
            .finish()
    }
}
