use rust_decimal::Decimal;
use serde::{Deserialize, Deserializer, de};
use std::str::FromStr;

/// Robust deserializer for Decimal that handles numbers, strings, and floats
pub fn deserialize<'de, D>(deserializer: D) -> Result<Decimal, D::Error>
where
    D: Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    match value {
        serde_json::Value::Number(num) => {
            if let Some(i) = num.as_i64() {
                Ok(Decimal::from(i))
            } else if let Some(f) = num.as_f64() {
                Decimal::from_str(&f.to_string()).map_err(de::Error::custom)
            } else {
                Err(de::Error::custom("Invalid number format"))
            }
        }
        serde_json::Value::String(s) => Decimal::from_str(&s).map_err(de::Error::custom),
        _ => Err(de::Error::custom("Expected a number or string for Decimal")),
    }
}

/// Optional version of the robust Decimal deserializer
pub fn deserialize_option<'de, D>(deserializer: D) -> Result<Option<Decimal>, D::Error>
where
    D: Deserializer<'de>,
{
    let opt = Option::<serde_json::Value>::deserialize(deserializer)?;
    match opt {
        Some(value) => match value {
            serde_json::Value::Number(num) => {
                if let Some(i) = num.as_i64() {
                    Ok(Some(Decimal::from(i)))
                } else if let Some(f) = num.as_f64() {
                    let d = Decimal::from_str(&f.to_string()).map_err(de::Error::custom)?;
                    Ok(Some(d))
                } else {
                    Err(de::Error::custom("Invalid number format"))
                }
            }
            serde_json::Value::String(s) => {
                if s.is_empty() {
                    Ok(None)
                } else {
                    let d = Decimal::from_str(&s).map_err(de::Error::custom)?;
                    Ok(Some(d))
                }
            }
            serde_json::Value::Null => Ok(None),
            _ => Err(de::Error::custom("Expected a number, string or null for Decimal")),
        },
        None => Ok(None),
    }
}
