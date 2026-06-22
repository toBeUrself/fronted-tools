# rs-tools

Rust tools for kline analysis. The first binary, `kline-analyze`, reads OHLCV CSV data and outputs a JSON report containing:

- BOLL, MACD, ATR, ADX latest values
- market state scores: range, uptrend, downtrend
- grid strategy mode and suggested range
- warning/watch signals for grid trading

This is an analysis helper, not an auto-trading engine.

## CSV format

The CLI accepts headers like:

```csv
open_time,open,high,low,close,volume
1710000000,68000,68100,67900,68050,123.4
```

It also accepts aliases from the kline table naming style:

```csv
open_time,open_price,high_price,low_price,close_price,base_volume
1710000000,68000,68100,67900,68050,123.4
```

`open_time` can be seconds or milliseconds. The analyzer only preserves and returns the same timestamp unit; it does not convert units internally.

## Run

```bash
cd packages/rs-tools
cargo run --bin kline-analyze -- --input ./sample.csv --symbol BTCUSDT --interval 5m
```

Read CSV from stdin:

```bash
cat ./sample.csv | cargo run --bin kline-analyze -- --symbol BTCUSDT --interval 5m
```

Limit to recent rows:

```bash
cargo run --bin kline-analyze -- --input ./sample.csv --limit 500 --grid-count 20
```

## Output example

```json
{
  "symbol": "BTCUSDT",
  "interval": "5m",
  "latest": {
    "time": 1710000000,
    "close": 68050.0,
    "boll": {
      "mid": 67980.0,
      "upper": 68600.0,
      "lower": 67360.0,
      "bandwidth": 0.0182
    }
  },
  "market": {
    "state": "range_grid",
    "scores": {
      "range_score": 80,
      "up_score": 20,
      "down_score": 15
    },
    "reasons": ["BOLL 中轨接近走平"]
  },
  "grid_plan": {
    "enabled": true,
    "mode": "range_grid",
    "lower": 67360.0,
    "upper": 68600.0,
    "center": 67980.0,
    "grid_count": 20,
    "grid_step": 62.0,
    "risk_action": "normal"
  },
  "signals": []
}
```

## Suggested usage in the project

1. Keep the kline API as the data source.
2. Convert the returned kline JSON to the `Kline` struct or to CSV for this CLI.
3. Use the JSON report to draw markers and grid lines on TradingView:
   - `signals`: chart markers
   - `grid_plan.lower` / `grid_plan.upper` / `grid_plan.center`: horizontal lines
   - `market.state` and `market.scores`: side panel status

## Current state model

The classifier is deliberately simple and explainable:

- `range_grid`: BOLL mid is flat, bandwidth is not extreme, MACD flips around zero, ADX is low.
- `up_break_warning`: early upward breakout warning.
- `down_break_warning`: early downward breakout warning.
- `uptrend_follow`: trend-follow mode; avoid selling too aggressively in ordinary grid mode.
- `downtrend_risk`: risk-control mode; pause new buy orders for ordinary grid mode.
- `wait`: not enough clarity.

Before using it for real trading, backtest it against your existing kline database with fees and slippage included.
