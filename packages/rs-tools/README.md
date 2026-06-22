# rs-tools

Rust tools for kline analysis. The first binary, `kline-analyze`, reads OHLCV data from CSV or from the market kline API and outputs a JSON report containing:

- BOLL, MACD, ATR, ADX latest values
- market state scores: range, uptrend, downtrend
- grid strategy mode and suggested range
- warning/watch signals for grid trading

This is an analysis helper, not an auto-trading engine.

## API input

The CLI can fetch klines from:

```text
/api/v1/public/market/klines
```

Supported query params match the backend API:

```rust
source?: string       // default is backend default, usually binance
symbol: string        // e.g. BTCUSDT
interval: string      // e.g. 1m, 5m, 30m
startTime?: i64       // milliseconds timestamp
endTime?: i64         // milliseconds timestamp
limit?: i64           // backend default usually 500, max usually 1000
```

Run with API:

```bash
cd packages/rs-tools

cargo run --bin kline-analyze -- \
  --api-base-url http://127.0.0.1:3000 \
  --source binance \
  --symbol BTCUSDT \
  --interval 5m \
  --api-limit 1000
```

With time range:

```bash
cargo run --bin kline-analyze -- \
  --api-base-url http://127.0.0.1:3000 \
  --symbol BTCUSDT \
  --interval 5m \
  --start-time 1710000000000 \
  --end-time 1710086400000 \
  --api-limit 1000
```

The parser supports these API response shapes:

```json
[
  {"open_time":1710000000000,"open_price":"68000","high_price":"68100","low_price":"67900","close_price":"68050","base_volume":"123.4"}
]
```

```json
{"data":[{"open_time":1710000000000,"open":"68000","high":"68100","low":"67900","close":"68050","volume":"123.4"}]}
```

It also supports array-style rows such as:

```json
{"data":[[1710000000000,"68000","68100","67900","68050","123.4"]]}
```

`open_time` accepts millisecond timestamps, second timestamps, numeric strings, or RFC3339 datetime strings. The analyzer preserves numeric timestamp units in the output. RFC3339 strings are converted to milliseconds.

## CSV input

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

Run with CSV:

```bash
cd packages/rs-tools
cargo run --bin kline-analyze -- --input ./sample.csv --symbol BTCUSDT --interval 5m
```

Read CSV from stdin:

```bash
cat ./sample.csv | cargo run --bin kline-analyze -- --symbol BTCUSDT --interval 5m
```

Limit to recent rows after loading data:

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
2. Run this analyzer from a backend analysis job, or move `analyze_klines` into your Rust service directly.
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
