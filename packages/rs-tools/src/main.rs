use anyhow::{Context, Result};
use clap::Parser;
use rs_tools::{analyze_klines, AnalysisConfig, Kline};
use std::fs::File;
use std::io::{self, Read};

#[derive(Debug, Parser)]
#[command(name = "kline-analyze")]
#[command(about = "Analyze OHLCV kline CSV data and output market state / grid plan JSON")]
struct Args {
    /// CSV file path. If omitted, CSV is read from stdin.
    #[arg(short, long)]
    input: Option<String>,

    /// Optional symbol name for report output.
    #[arg(long)]
    symbol: Option<String>,

    /// Optional interval name for report output, e.g. 5m, 30m, 4h.
    #[arg(long)]
    interval: Option<String>,

    /// BOLL period.
    #[arg(long, default_value_t = 20)]
    boll_period: usize,

    /// BOLL standard deviation multiplier.
    #[arg(long, default_value_t = 2.0)]
    boll_mult: f64,

    /// Suggested grid count.
    #[arg(long, default_value_t = 20)]
    grid_count: usize,

    /// Keep only the latest N rows before analysis.
    #[arg(long)]
    limit: Option<usize>,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let csv_text = read_input(args.input.as_deref())?;
    let mut klines = load_csv(&csv_text)?;

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
