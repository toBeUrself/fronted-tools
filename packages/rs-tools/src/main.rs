use anyhow::{Context, Result};
use chrono::DateTime;
use clap::Parser;
use reqwest::blocking::Client;
use rs_tools::{analyze_klines, AnalysisConfig, Kline};
use serde_json::Value;
use std::fs::File;
use std::io::{self, Read};
use std::time::Duration;

const KLINES_PATH: &str = "/api/v1/public/market/klines";

#[derive(Debug, Parser)]
#[command(name = "kline-analyze")]
#[command(about = "Analyze OHLCV kline data from CSV or market API and output market state / grid plan JSON")]
struct Args {
    /// CSV file path. If omitted and api-base-url is not set, CSV is read from stdin.
    #[arg(short, long)]
    input: Option<String>,

    /// API base URL, for example: http://127.0.0.1:3000 . When set, klines are fetched from /api/v1/public/market/klines.
    #[arg(long)]
    api_base_url: Option<String>,

    /// Data source for API query. If omitted, backend default is used, usually binance.
    #[arg(long)]
    source: Option<String>,

    /// Symbol name, e.g. BTCUSDT. Required when api-base-url is used.
    #[arg(long)]
    symbol: Option<String>,

    /// Interval name, e.g. 1m, 5m, 30m. Required when api-base-url is used.
    #[arg(long)]
    interval: Option<String>,

    /// API startTime query param, milliseconds timestamp.
    #[arg(long, alias = "start-time")]
    start_time: Option<i64>,

    /// API endTime query param, milliseconds timestamp.
    #[arg(long, alias = "end-time")]
    end_time: Option<i64>,

    /// API limit query param. Backend default is usually 500 and max is usually 1000.
    #[arg(long)]
    api_limit: Option<i64>,

    /// BOLL period.
    #[arg(long, default_value_t = 20)]
    boll_period: usize,

    /// BOLL standard deviation multiplier.
    #[arg(long, default_value_t = 2.0)]
    boll_mult: f64,

    /// Suggested grid count.
    #[arg(long, default_value_t = 20)]
    grid_count: usize,

    /// Keep only the latest N rows after loading data.
    #[arg(long)]
    limit: Option<usize>,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let mut klines = if let Some(api_base_url) = args.api_base_url.as_deref() {
        fetch_api_klines(api_base_url, &args)?
    } else {
        let csv_text = read_input(args.input.as_deref())?;
        load_csv(&csv_text)?
    };

    klines.sort_by_key(|k| k.open_time);
    if let Some(limit) = args.limit {
        if klines.len() > limit {
            klines = klines[klines.len() - limit..].to_vec();
        }
    }

    let config = AnalysisConfig {
        boll_period: args.boll_period,
        boll_mult: args.boll_mult,
        grid_count: args.grid_count,
        ..AnalysisConfig::default()
    };

    let report = analyze_klines(&klines, &config, args.symbol, args.interval);
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

fn fetch_api_klines(api_base_url: &str, args: &Args) -> Result<Vec<Kline>> {
    let symbol = args
        .symbol
        .as_deref()
        .context("--symbol is required when --api-base-url is used")?;
    let interval = args
        .interval
        .as_deref()
        .context("--interval is required when --api-base-url is used")?;

    let url = format!("{}{}", api_base_url.trim_end_matches('/'), KLINES_PATH);
    let mut query = vec![
        ("symbol".to_string(), symbol.to_string()),
        ("interval".to_string(), interval.to_string()),
    ];

    if let Some(source) = args.source.as_deref() {
        query.push(("source".to_string(), source.to_string()));
    }
    if let Some(start_time) = args.start_time {
        query.push(("startTime".to_string(), start_time.to_string()));
    }
    if let Some(end_time) = args.end_time {
        query.push(("endTime".to_string(), end_time.to_string()));
    }
    if let Some(limit) = args.api_limit {
        query.push(("limit".to_string(), limit.to_string()));
    }

    let client = Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .context("failed to build HTTP client")?;

    let response = client
        .get(&url)
        .query(&query)
        .send()
        .with_context(|| format!("failed to request klines API: {url}"))?
        .error_for_status()
        .context("klines API returned an error status")?;

    let value = response
        .json::<Value>()
        .context("failed to decode klines API JSON")?;

    parse_klines_response(value)
}

fn parse_klines_response(value: Value) -> Result<Vec<Kline>> {
    let rows = extract_rows(value).context("failed to find kline array in API response")?;
    let mut klines = Vec::with_capacity(rows.len());

    for row in rows {
        let kline = match row {
            Value::Array(items) => parse_kline_array(&items)?,
            Value::Object(_) => serde_json::from_value::<Kline>(row)
                .context("failed to parse kline object from API response")?,
            other => anyhow::bail!("unsupported kline row format: {other}"),
        };
        validate_kline(&kline)?;
        klines.push(kline);
    }

    Ok(klines)
}

fn extract_rows(value: Value) -> Option<Vec<Value>> {
    match value {
        Value::Array(rows) => Some(rows),
        Value::Object(mut object) => {
            for key in ["data", "items", "rows", "list", "klines"] {
                if let Some(inner) = object.remove(key) {
                    if let Some(rows) = extract_rows(inner) {
                        return Some(rows);
                    }
                }
            }
            None
        }
        _ => None,
    }
}

fn parse_kline_array(items: &[Value]) -> Result<Kline> {
    if items.len() < 5 {
        anyhow::bail!("array kline row must contain at least 5 columns");
    }

    Ok(Kline {
        open_time: parse_i64_value(&items[0]).context("failed to parse array kline open_time")?,
        open: parse_f64_value(&items[1]).context("failed to parse array kline open")?,
        high: parse_f64_value(&items[2]).context("failed to parse array kline high")?,
        low: parse_f64_value(&items[3]).context("failed to parse array kline low")?,
        close: parse_f64_value(&items[4]).context("failed to parse array kline close")?,
        volume: items.get(5).map(parse_f64_value).transpose()?.unwrap_or(0.0),
    })
}

fn parse_i64_value(value: &Value) -> Result<i64> {
    match value {
        Value::Number(number) => number
            .as_i64()
            .or_else(|| number.as_u64().and_then(|v| i64::try_from(v).ok()))
            .or_else(|| number.as_f64().map(|v| v as i64))
            .context("invalid numeric timestamp"),
        Value::String(text) => {
            let trimmed = text.trim();
            if let Ok(ts) = trimmed.parse::<i64>() {
                return Ok(ts);
            }
            DateTime::parse_from_rfc3339(trimmed)
                .map(|dt| dt.timestamp_millis())
                .context("timestamp string is not numeric or RFC3339")
        }
        _ => anyhow::bail!("timestamp must be number or string"),
    }
}

fn parse_f64_value(value: &Value) -> Result<f64> {
    match value {
        Value::Number(number) => number.as_f64().context("invalid JSON number"),
        Value::String(text) => text
            .trim()
            .parse::<f64>()
            .context("numeric string parse failed"),
        _ => anyhow::bail!("numeric value must be number or string"),
    }
}

fn read_input(path: Option<&str>) -> Result<String> {
    let mut buf = String::new();
    match path {
        Some(path) => {
            File::open(path)
                .with_context(|| format!("failed to open input file: {path}"))?
                .read_to_string(&mut buf)
                .with_context(|| format!("failed to read input file: {path}"))?;
        }
        None => {
            io::stdin()
                .read_to_string(&mut buf)
                .context("failed to read csv from stdin")?;
        }
    }
    Ok(buf)
}

fn load_csv(csv_text: &str) -> Result<Vec<Kline>> {
    let mut reader = csv::ReaderBuilder::new()
        .trim(csv::Trim::All)
        .from_reader(csv_text.as_bytes());

    let mut rows = Vec::new();
    for result in reader.deserialize::<Kline>() {
        let row = result.context("failed to deserialize kline row")?;
        validate_kline(&row)?;
        rows.push(row);
    }
    Ok(rows)
}

fn validate_kline(k: &Kline) -> Result<()> {
    if !k.open.is_finite()
        || !k.high.is_finite()
        || !k.low.is_finite()
        || !k.close.is_finite()
        || !k.volume.is_finite()
    {
        anyhow::bail!("kline contains non-finite number at open_time={}", k.open_time);
    }
    if k.high < k.low || k.high < k.open || k.high < k.close || k.low > k.open || k.low > k.close {
        anyhow::bail!("invalid OHLC relation at open_time={}", k.open_time);
    }
    if k.volume < 0.0 {
        anyhow::bail!("volume cannot be negative at open_time={}", k.open_time);
    }
    Ok(())
}
