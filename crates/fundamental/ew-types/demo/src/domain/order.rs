//! 订单领域。

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use super::user::UserId;

/// 币种。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Currency {
    /// 人民币。
    Cny,
    /// 美元。
    Usd,
}

/// 金额。以最小货币单位记账，不做浮点运算。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Money {
    /// 最小货币单位数额。单位：分。范围：[0, i64::MAX]。默认值：无。非空。
    pub minor: i64,
    /// 币种。单位：—。范围：见 [`Currency`]。默认值：无。与明细逐项比对，必须一致。
    pub currency: Currency,
}

impl Money {
    /// 构造金额。不校验正负，由上层业务决定。
    pub fn new(minor: i64, currency: Currency) -> Self {
        Self { minor, currency }
    }
}

/// 订单标识。26 位、按时间有序，全局唯一。
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct OrderId(String);

impl OrderId {
    /// 生成新标识。时间戳打头，保证同一毫秒内递增。
    pub fn new() -> Self {
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let seq = SEQ.fetch_add(1, Ordering::Relaxed);
        Self(format!("{:020}{:06}", nanos, seq % 1_000_000))
    }

    /// 标识的文本形式。
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for OrderId {
    fn default() -> Self {
        Self::new()
    }
}

/// 订单明细行。下单时冻结单价，后续改价不影响已生成的订单。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineItem {
    /// 商品编码。单位：—。范围：[1, 64] 字符。默认值：无。非空。
    pub sku: String,
    /// 成交单价。单位：分。范围：[0, i64::MAX]。默认值：无。非空。
    pub price: Money,
    /// 购买数量。单位：件。范围：[1, 999]。默认值：1。非空。
    pub qty: u32,
}

/// 订单状态。
///
/// # 流转
/// - Pending -> Paid：支付成功
/// - Paid -> Shipped：仓库发货
/// - Pending -> Cancelled：用户取消或超时未支付
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OrderStatus {
    /// 已创建，等待支付。
    Pending,
    /// 已支付，等待发货。
    Paid,
    /// 已发货，运输中。
    Shipped,
    /// 已取消。原因记在订单备注。
    Cancelled,
}

/// 订单构造错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OrderError {
    /// 商品明细为空。
    EmptyItems,
    /// 明细币种与总额币种不一致。
    CurrencyMismatch,
}

/// 订单。下单时冻结商品价格，创建后不再变更。
///
/// 退款另开退款单，不改本结构。
///
/// # 不变量
/// - `items` 非空
/// - `total` 的币种与每个 `items` 项一致
///
/// # 示例
/// ```
/// use demo::domain::order::{Currency, LineItem, Money, Order, OrderId};
///
/// let item = LineItem {
///     sku: "SKU-1".into(),
///     price: Money::new(1990, Currency::Cny),
///     qty: 1,
/// };
/// let order = Order::try_new(OrderId::new(), vec![item], Money::new(1990, Currency::Cny))
///     .expect("构造失败");
/// assert_eq!(order.items().len(), 1);
/// ```
#[derive(Debug, Clone)]
pub struct Order {
    /// 订单标识。单位：—。范围：26 字符。默认值：无。非空。
    id: OrderId,
    /// 下单用户。单位：—。范围：—。默认值：无。非空。
    buyer: UserId,
    /// 商品明细。单位：—。范围：非空。默认值：无。保持下单顺序。
    items: Vec<LineItem>,
    /// 当前状态。单位：—。范围：见 [`OrderStatus`]。默认值：Pending。
    status: OrderStatus,
    /// 应付总额。单位：分。范围：≥ 1。默认值：无。两位小数。
    total: Money,
    /// 创建时间。单位：UTC。范围：≤ 当前时刻。默认值：创建时刻。非空。
    created_at: SystemTime,
}

impl Order {
    /// 创建订单。字段私有，只能经此入口构造，不变量在此校验。
    ///
    /// # 错误
    /// - `items` 为空时返回 [`OrderError::EmptyItems`]
    /// - 明细币种与总额不一致时返回 [`OrderError::CurrencyMismatch`]
    pub fn try_new(
        id: OrderId,
        items: Vec<LineItem>,
        total: Money,
    ) -> Result<Self, OrderError> {
        if items.is_empty() {
            return Err(OrderError::EmptyItems);
        }
        let buyer = UserId::new("anonymous");
        if items.iter().any(|i| i.price.currency != total.currency) {
            return Err(OrderError::CurrencyMismatch);
        }
        Ok(Self {
            id,
            buyer,
            status: OrderStatus::Pending,
            items,
            total,
            created_at: SystemTime::now(),
        })
    }

    /// 订单标识。
    pub fn id(&self) -> &OrderId {
        &self.id
    }

    /// 商品明细。
    pub fn items(&self) -> &[LineItem] {
        &self.items
    }

    /// 应付总额。
    pub fn total(&self) -> Money {
        self.total
    }

    /// 当前状态。
    pub fn status(&self) -> OrderStatus {
        self.status
    }
}
