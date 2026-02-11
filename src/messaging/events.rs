use serde::{Deserialize, Serialize};
use uuid::Uuid;
use rust_decimal::Decimal;

/// Base trait for all Lanai events
/// Base trait for all Lanai events
pub trait LanaiEvent {
    fn subject(&self) -> String;
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ProductCreatedEvent {
    pub product_id: Uuid,
    pub org_id: Uuid,
    pub name: String,
    pub description: Option<String>,
}

impl LanaiEvent for ProductCreatedEvent {
    fn subject(&self) -> String {
        format!("lanai.inventory.product.created.{}", self.org_id)
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct StockItem {
    pub product_id: Uuid,
    /// Quantity supports fractional values (kg, L) for Restaurant/Agro verticals
    pub quantity: Decimal,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ReserveStockRequest {
    pub order_id: Uuid,
    pub org_id: Uuid,
    pub items: Vec<StockItem>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ReserveStockResponse {
    pub order_id: Uuid,
    pub success: bool,
    pub error: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ReleaseStockRequest {
    pub order_id: Uuid,
    pub org_id: Uuid,
    pub items: Vec<StockItem>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ReturnCompletedEvent {
    pub return_id: Uuid,
    pub order_id: Uuid,
    pub org_id: Uuid,
    pub items: Vec<ReturnItemEvent>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ReturnItemEvent {
    pub product_id: Uuid,
    /// Quantity supports fractional values (kg, L) for Restaurant/Agro verticals
    pub quantity: Decimal,
    pub inventory_action: String, // RESTOCK, QUARANTINE, DISPOSE
}

impl LanaiEvent for ReturnCompletedEvent {
    fn subject(&self) -> String {
        format!("lanai.sales.return.completed.{}", self.org_id)
    }
}
