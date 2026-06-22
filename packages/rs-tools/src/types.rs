use chrono::DateTime;
use serde::de::{self, Unexpected, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use std::fmt;

#[derive(Debug, Clone, Deserialize)]
pub struct Kline {
    #[serde(alias = "time", alias = "timestamp", deserialize_with = "deserialize_i64_any")]
    pub open_time: i64,
    #[serde(alias = "open_price", deserialize_with = "deserialize_f64_any")]
    pub open: f64,
    #[serde(alias = "high_price", deserialize_with = "deserialize_f64_any")]
    pub high: f64,
    #[serde(alias = "low_price", deserialize_with = "deserialize_f64_any")]
    pub low: f64,
    #[serde(alias = "close_price", deserialize_with = "deserialize_f64_any")]
    pub close: f64,
    #[serde(default, alias = "base_volume", deserialize_with = "deserialize_f64_any")]
    pub volume: f64,
}

fn deserialize_i64_any<'de, D>(deserializer: D) -> Result<i64, D::Error>
where
    D: Deserializer<'de>,
{
    struct AnyI64Visitor;

    impl<'de> Visitor<'de> for AnyI64Visitor {
        type Value = i64;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("an integer timestamp, numeric string, or RFC3339 datetime string")
        }

        fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E> {
            Ok(value)
        }

        fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            i64::try_from(value).map_err(|_| E::invalid_value(Unexpected::Unsigned(value), &self))
        }

        fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            if value.is_finite() {
                Ok(value as i64)
            } else {
                Err(E::invalid_value(Unexpected::Float(value), &self))
            }
        }

        fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            let trimmed = value.trim();
            if let Ok(ts) = trimmed.parse::<i64>() {
                return Ok(ts);
            }
            DateTime::parse_from_rfc3339(trimmed)
                .map(|dt| dt.timestamp_millis())
                .map_err(|_| E::invalid_value(Unexpected::Str(value), &self))
        }
    }

    deserializer.deserialize_any(AnyI64Visitor)
}

fn deserialize_f64_any<'de, D>(deserializer: D) -> Result<f64, D::Error>
where
    D: Deserializer<'de>,
{
    struct AnyF64Visitor;

    impl<'de> Visitor<'de> for AnyF64Visitor {
        type Value = f64;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("a number or numeric string")
        }

        fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E> {
            Ok(value)
        }

        fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E> {
            Ok(value as f64)
        }

        fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E> {
            Ok(value as f64)
        }

        fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            value
                .trim()
                .parse::<f64>()
                .map_err(|_| E::invalid_value(Unexpected::Str(value), &self))
        }
    }

    deserializer.deserialize_any(AnyF64Visitor)
}

#[derive(Debug, Clone, Serialize)]
pub struct BollPoint {
    pub mid: f64,
    pub upper: f64,
    pub lower: f64,
    pub bandwidth: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct MacdPoint {
    pub dif: f64,
    pub dea: f64,
    pub hist: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct AdxPoint {
    pub adx: f64,
    pub plus_di: f64,
    pub minus_di: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct IndicatorPoint {
    pub time: i64,
    pub close: f64,
    pub boll: Option<BollPoint>,
    pub macd: Option<MacdPoint>,
    pub atr: Option<f64>,
    pub adx: Option<AdxPoint>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MarketState {
    RangeGrid,
    UpBreakWarning,
    DownBreakWarning,
    UptrendFollow,
    DowntrendRisk,
    Wait,
}

#[derive(Debug, Clone, Serialize)]
pub struct MarketScores {
    pub range_score: u8,
    pub up_score: u8,
    pub down_score: u8,
}

#[derive(Debug, Clone, Serialize)]
pub struct MarketStateReport {
    pub state: MarketState,
    pub scores: MarketScores,
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GridPlan {
    pub enabled: bool,
    pub mode: String,
    pub lower: Option<f64>,
    pub upper: Option<f64>,
    pub center: Option<f64>,
    pub grid_count: usize,
    pub grid_step: Option<f64>,
    pub risk_action: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Signal {
    pub time: i64,
    pub price: f64,
    pub signal_type: String,
    pub strength: f64,
    pub text: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct AnalysisReport {
    pub symbol: Option<String>,
    pub interval: Option<String>,
    pub latest: Option<IndicatorPoint>,
    pub market: MarketStateReport,
    pub grid_plan: GridPlan,
    pub signals: Vec<Signal>,
}

#[derive(Debug, Clone)]
pub struct AnalysisConfig {
    pub boll_period: usize,
    pub boll_mult: f64,
    pub macd_fast: usize,
    pub macd_slow: usize,
    pub macd_signal: usize,
    pub atr_period: usize,
    pub adx_period: usize,
    pub grid_count: usize,
}

impl Default for AnalysisConfig {
    fn default() -> Self {
        Self {
            boll_period: 20,
            boll_mult: 2.0,
            macd_fast: 12,
            macd_slow: 26,
            macd_signal: 9,
            atr_period: 14,
            adx_period: 14,
            grid_count: 20,
        }
    }
}
