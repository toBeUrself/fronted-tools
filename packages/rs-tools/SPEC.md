# Kline Indicator Analysis Spec

## 1. Background

`rs-tools` provides an explainable Kline indicator analysis module for grid-trading assistance. It reads historical OHLCV Kline data, calculates common technical indicators, classifies the current market state, and outputs a JSON report that can be consumed by backend jobs or frontend TradingView overlays.

This module is not an auto-trading engine. It only produces analysis results, grid suggestions, and warning signals. Any real trading logic must add backtesting, fee/slippage modeling, risk control, and manual or automated execution rules outside this module.

## 2. Goals

The module should:

1. Fetch or load Kline data.
2. Validate and sort Kline data by `open_time`.
3. Calculate technical indicators.
4. Classify market state into explainable categories.
5. Generate grid-trading helper output.
6. Generate chart-friendly signals.
7. Return stable JSON that can be displayed on TradingView.

## 3. Non-goals

The module does not:

1. Place orders.
2. Manage exchange accounts or API keys.
3. Guarantee price prediction accuracy.
4. Replace backtesting.
5. Persist analysis results to a database in the first version.
6. Draw directly on TradingView. The frontend should consume the JSON output and draw markers/lines.

## 4. Package location

```text
packages/rs-tools
```

Main binary:

```text
kline-analyze
```

Main library function:

```rust
analyze_klines(klines, config, symbol, interval) -> AnalysisReport
```

## 5. Input sources

### 5.1 CSV input

The CLI supports local CSV files and stdin.

Accepted canonical headers:

```csv
open_time,open,high,low,close,volume
```

Accepted database-style aliases:

```csv
open_time,open_price,high_price,low_price,close_price,base_volume
```

### 5.2 Market Kline API input

The CLI can fetch Kline data from:

```text
/api/v1/public/market/klines
```

API query parameters:

| Parameter | Required | Description |
|---|---:|---|
| `source` | No | Data source, default is backend default, usually `binance`. |
| `symbol` | Yes | Trading pair, e.g. `BTCUSDT`. |
| `interval` | Yes | Kline interval, e.g. `1m`, `5m`, `30m`. |
| `startTime` | No | Start time in milliseconds. Alias support is handled by backend as `start_time`. |
| `endTime` | No | End time in milliseconds. Alias support is handled by backend as `end_time`. |
| `limit` | No | Return count. Backend default is usually 500 and max is usually 1000. |

CLI example:

```bash
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

## 6. Supported response shapes

The parser should support direct array responses:

```json
[
  {
    "open_time": 1710000000000,
    "open_price": "68000",
    "high_price": "68100",
    "low_price": "67900",
    "close_price": "68050",
    "base_volume": "123.4"
  }
]
```

Wrapped responses:

```json
{
  "data": [
    {
      "open_time": 1710000000000,
      "open": "68000",
      "high": "68100",
      "low": "67900",
      "close": "68050",
      "volume": "123.4"
    }
  ]
}
```

Alternative wrapper keys:

```text
data, items, rows, list, klines
```

Array-style Kline rows:

```json
{
  "data": [
    [1710000000000, "68000", "68100", "67900", "68050", "123.4"]
  ]
}
```

Array row order:

```text
[open_time, open, high, low, close, volume]
```

## 7. Kline data model

Internal model:

```rust
pub struct Kline {
    pub open_time: i64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
}
```

### 7.1 Timestamp handling

`open_time` accepts:

1. Millisecond timestamp.
2. Second timestamp.
3. Numeric string.
4. RFC3339 datetime string.

The analyzer preserves numeric timestamp units. RFC3339 strings are converted to milliseconds.

### 7.2 Number handling

Price and volume fields accept both JSON numbers and numeric strings.

### 7.3 Validation rules

Each Kline must satisfy:

```text
high >= low
high >= open
high >= close
low <= open
low <= close
volume >= 0
all numeric fields are finite
```

Invalid rows should return an error instead of being silently ignored.

## 8. Indicator calculations

### 8.1 BOLL

Default parameters:

```text
period = 20
multiplier = 2.0
```

Formula:

```text
mid = SMA(close, period)
std = standard_deviation(close, period)
upper = mid + multiplier * std
lower = mid - multiplier * std
bandwidth = (upper - lower) / abs(mid)
```

Purpose:

1. Estimate dynamic price range.
2. Detect volatility contraction/expansion.
3. Provide grid upper/lower/center reference.

### 8.2 MACD

Default parameters:

```text
fast = 12
slow = 26
signal = 9
```

Formula:

```text
dif = EMA(close, fast) - EMA(close, slow)
dea = EMA(dif, signal)
hist = dif - dea
```

Purpose:

1. Detect momentum direction.
2. Detect momentum weakening or strengthening.
3. Assist breakout warning and grid buy/sell watch signals.

### 8.3 ATR

Default parameter:

```text
period = 14
```

True Range:

```text
TR = max(
  high - low,
  abs(high - previous_close),
  abs(low - previous_close)
)
```

ATR is Wilder-smoothed.

Purpose:

1. Estimate recent volatility.
2. Generate trend-follow grid range in breakout/uptrend mode.
3. Provide spacing reference for dynamic grid.

### 8.4 ADX / DMI

Default parameter:

```text
period = 14
```

Outputs:

```text
adx
plus_di
minus_di
```

Purpose:

1. Determine whether trend strength is weak or rising.
2. Distinguish range-friendly environments from trend/risk environments.
3. Help identify upward or downward directional pressure.

## 9. Market state classification

The classifier is score-based and explainable. It produces three scores:

```text
range_score
up_score
down_score
```

Each score ranges from 0 to 100.

### 9.1 State enum

```rust
pub enum MarketState {
    RangeGrid,
    UpBreakWarning,
    DownBreakWarning,
    UptrendFollow,
    DowntrendRisk,
    Wait,
}
```

### 9.2 RangeGrid

Meaning:

The market is likely range-bound and ordinary grid strategy may be enabled.

Typical evidence:

1. BOLL mid is close to flat.
2. BOLL bandwidth is not extremely narrow or extremely wide.
3. Price crosses BOLL mid multiple times recently.
4. MACD histogram flips around zero.
5. ADX is low.

Default decision rule:

```text
range_score >= 65
up_score < 55
down_score < 55
```

Action:

```text
enable ordinary range grid
use BOLL lower/upper/mid as grid lower/upper/center
```

### 9.3 UpBreakWarning

Meaning:

Range may be failing upward, but trend is not fully confirmed.

Typical evidence:

1. Price is above BOLL mid.
2. BOLL mid starts rising.
3. BOLL bandwidth expands.
4. MACD histogram strengthens.
5. +DI is greater than -DI and ADX is not weakening.

Default decision rule:

```text
up_score >= 55 and up_score < 70
```

Action:

```text
reduce aggressive grid selling
prepare to move grid upward
avoid selling inventory too early during breakout
```

### 9.4 UptrendFollow

Meaning:

The market is more likely in an upward trend-follow environment.

Default decision rule:

```text
up_score >= 70
```

Action:

```text
use trend-follow grid mode
center around BOLL mid or current structure
use ATR to generate asymmetric upper/lower range
```

### 9.5 DownBreakWarning

Meaning:

Range may be failing downward, but downtrend risk is not fully confirmed.

Typical evidence:

1. Price is below BOLL mid or BOLL lower.
2. BOLL mid starts falling.
3. BOLL bandwidth expands.
4. MACD histogram weakens.
5. -DI is greater than +DI and ADX is not weakening.

Default decision rule:

```text
down_score >= 55 and down_score < 70
```

Action:

```text
pause or reduce new buy orders
avoid ordinary grid continuing to average down blindly
```

### 9.6 DowntrendRisk

Meaning:

The market is more likely in a downward trend/risk-control environment.

Default decision rule:

```text
down_score >= 70
```

Action:

```text
disable ordinary grid
pause new buy orders
wait for recovery or a new stable range
```

### 9.7 Wait

Meaning:

There is not enough clarity or insufficient data.

Action:

```text
do not enable ordinary grid automatically
wait for clearer market state
```

## 10. Grid plan output

`GridPlan` should be returned in every report.

```rust
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
```

### 10.1 Range grid mode

When `state = range_grid`:

```text
lower = boll.lower
upper = boll.upper
center = boll.mid
grid_step = (upper - lower) / grid_count
enabled = grid_step > 0
risk_action = normal
```

### 10.2 Uptrend follow mode

When `state = up_break_warning` or `state = uptrend_follow`:

```text
center = boll.mid
lower = center - 1.5 * atr
upper = center + 2.5 * atr
enabled = true
risk_action = reduce_sell_strength_and_move_grid_up_on_pullback
```

### 10.3 Downtrend risk mode

When `state = down_break_warning` or `state = downtrend_risk`:

```text
enabled = false
mode = risk_control
risk_action = pause_new_buy_orders
```

### 10.4 Wait mode

When `state = wait`:

```text
enabled = false
mode = wait
risk_action = wait_for_clearer_state
```

## 11. Signal output

Signals are chart-friendly events.

```rust
pub struct Signal {
    pub time: i64,
    pub price: f64,
    pub signal_type: String,
    pub strength: f64,
    pub text: String,
}
```

Supported signal types in the first version:

| Signal type | Meaning | Frontend display suggestion |
|---|---|---|
| `grid_buy_watch` | Price is near BOLL lower and MACD bearish momentum is weakening. | Up marker near Kline low/close. |
| `grid_sell_watch` | Price is near BOLL upper and MACD bullish momentum is weakening. | Down marker near Kline high/close. |
| `up_break_warning` | Range may be failing upward. | Warning marker above candle. |
| `down_break_warning` | Range may be failing downward. | Warning marker below candle. |

Signal strength is normalized to `0.0..1.0` where possible.

## 12. JSON output contract

Example response:

```json
{
  "symbol": "BTCUSDT",
  "interval": "5m",
  "latest": {
    "time": 1710000000000,
    "close": 68050.0,
    "boll": {
      "mid": 67980.0,
      "upper": 68600.0,
      "lower": 67360.0,
      "bandwidth": 0.0182
    },
    "macd": {
      "dif": 120.5,
      "dea": 100.1,
      "hist": 20.4
    },
    "atr": 300.5,
    "adx": {
      "adx": 18.2,
      "plus_di": 24.5,
      "minus_di": 19.3
    }
  },
  "market": {
    "state": "range_grid",
    "scores": {
      "range_score": 80,
      "up_score": 20,
      "down_score": 15
    },
    "reasons": [
      "BOLL 中轨接近走平",
      "ADX 较低，趋势强度偏弱"
    ]
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
  "signals": [
    {
      "time": 1710000000000,
      "price": 68050.0,
      "signal_type": "grid_buy_watch",
      "strength": 0.7,
      "text": "价格接近 BOLL 下轨，且 MACD 空头动能减弱"
    }
  ]
}
```

## 13. Frontend TradingView integration

The frontend should use the report as follows:

| Report field | TradingView usage |
|---|---|
| `latest.boll.upper/mid/lower` | Optional latest value display. Continuous BOLL line should use either TradingView built-in study or future indicator series endpoint. |
| `grid_plan.lower` | Draw grid lower horizontal line. |
| `grid_plan.upper` | Draw grid upper horizontal line. |
| `grid_plan.center` | Draw grid center horizontal line. |
| `signals[]` | Draw chart markers. |
| `market.state` | Display current market status panel. |
| `market.scores` | Display score bars or badges. |
| `market.reasons` | Display explanation text. |

Recommended first-stage display:

1. Use TradingView built-in BOLL/MACD for visual indicators.
2. Use backend analyzer output for grid lines and markers.
3. Display `market.state`, scores, and reasons in a side panel.

Future improvement:

1. Add an endpoint returning historical indicator series.
2. Draw BOLL/grid/status curves as custom TradingView studies.
3. Add historical signal marks through TradingView `getMarks` / `getTimescaleMarks`.

## 14. CLI parameters

| CLI parameter | Description |
|---|---|
| `--input` | CSV file path. If omitted and API URL is not set, read CSV from stdin. |
| `--api-base-url` | Base URL used to call `/api/v1/public/market/klines`. |
| `--source` | API `source` query parameter. |
| `--symbol` | Symbol. Required for API mode. |
| `--interval` | Interval. Required for API mode. |
| `--start-time` | API `startTime` in milliseconds. |
| `--end-time` | API `endTime` in milliseconds. |
| `--api-limit` | API `limit`. |
| `--boll-period` | BOLL period. Default `20`. |
| `--boll-mult` | BOLL multiplier. Default `2.0`. |
| `--grid-count` | Suggested grid count. Default `20`. |
| `--limit` | Keep only the latest N rows after loading. |

## 15. Error handling

The analyzer should fail fast for:

1. Missing `symbol` or `interval` in API mode.
2. HTTP request failure.
3. Non-success HTTP status.
4. Unsupported API response shape.
5. Invalid Kline row.
6. Non-finite price or volume values.
7. Invalid OHLC relationship.

Errors should be returned with enough context for debugging.

## 16. Testing requirements

Minimum test coverage should include:

1. CSV parsing with canonical headers.
2. CSV parsing with database-style aliases.
3. API response parsing for direct array.
4. API response parsing for `{ data: [...] }` wrapper.
5. API response parsing for array-style rows.
6. BOLL calculation on known close values.
7. MACD calculation shape and non-empty output.
8. ATR validation on known OHLC values.
9. ADX output availability after enough rows.
10. State classification for synthetic range data.
11. State classification for synthetic uptrend data.
12. State classification for synthetic downtrend data.
13. Invalid OHLC row should fail.

## 17. Backtesting requirements before real usage

Before using the output as trading input, run backtests with:

1. Historical Kline data across multiple symbols.
2. Multiple market regimes: range, strong uptrend, strong downtrend, high volatility crash, low liquidity periods.
3. Trading fees.
4. Slippage.
5. Minimum order size.
6. Grid order fill model.
7. Maximum drawdown.
8. Consecutive loss count.
9. Capital usage.
10. Comparison against simple baseline grid.

Recommended comparison groups:

```text
A. baseline fixed grid
B. BOLL-only filtered grid
C. BOLL + MACD + ATR + ADX state-filtered grid
```

The indicators are considered useful only if they improve risk-adjusted results after fees and slippage.

## 18. Version 1 limitations

1. The classifier uses heuristic scoring, not machine learning.
2. Thresholds are not optimized.
3. Only latest indicator values are returned in `AnalysisReport`.
4. No database persistence for reports.
5. No real-time websocket mode.
6. No direct TradingView rendering.
7. No order execution.

## 19. Future work

1. Add historical indicator series API.
2. Add backtesting module.
3. Add config file support for thresholds.
4. Add database persistence for market states and signals.
5. Add multiple timeframe confirmation.
6. Add TradingView mark endpoint.
7. Add WebSocket push for closed Kline analysis updates.
8. Add unit tests and integration tests.
9. Add cargo workspace integration if the repository moves to a Rust workspace.
