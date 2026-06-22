# K线指标分析与网格状态识别 Spec（Production Ready）

## 0. 文档信息

| 项目 | 内容 |
|---|---|
| 模块名 | `services/klines-tools` |
| 文档定位 | 需求、系统设计、接口契约、风控动作契约、回测验收文档 |
| 核心能力 | K线数据分析、指标计算、行情状态识别、网格策略辅助、TradingView 展示数据输出、回测与复盘数据契约 |
| 版本 | v1.1 production-ready execution spec |
| 适用阶段 | 重新实施、分析服务设计、前端展示、回测验证、影子运行、准实盘风控接入、小资金灰度 |
| 不适用范围 | 自动下单、账户管理、资金划转、保证盈利、替代回测、替代人工风控 |

> 本模块是行情状态识别与网格风控辅助系统，不是投资建议系统，也不保证盈利。所有评分阈值、权重、状态迁移和风控动作必须通过历史数据、手续费、滑点、流动性、交易所规则和样本外验证后，才能进入真实交易链路。

---

## 1. 背景与目标

网格交易适合震荡行情，但在单边上涨中容易过早卖出，在单边下跌中容易持续补仓导致套牢。`services/klines-tools` 的目标不是预测下一根 K线涨跌，而是把 K线数据转化为可解释、可回测、可展示、可被风控系统消费的行情状态。

核心问题：

```text
当前是否适合开启普通震荡网格？
上涨突破风险是否正在上升？
下跌破位风险是否正在上升？
普通网格是否应该暂停、移动、减仓或进入等待？
当前信号能否解释给前端和后续复盘？
当前风控动作是否能被执行层无歧义消费？
```

核心输出：

```text
range_score：震荡/网格适配评分
up_score：上涨/向上突破评分
down_score：下跌/向下破位评分
state：最终行情状态
grid_plan：网格建议
risk：风险等级与解释
risk_decision：可执行风控决策
signals：前端图表可展示信号
reasons：评分和状态判断原因
```

设计原则：

1. 风控阻断优先于收益机会。
2. 只使用已闭合 K线进行状态确认，未闭合 K线只能用于观察。
3. 分析层输出建议和动作契约，但不直接下单。
4. 回测、实时分析和复盘必须使用同一套指标、状态机和配置版本。
5. 默认参数只是初始经验值，生产参数必须经过回测、样本外验证和灰度。
6. 宁愿错过部分行情，也不能在极端下跌中灾难性连续补仓。

---

## 2. 总体架构

```text
K线 API / CSV / 数据库
        ↓
数据校验、去重、时间排序、连续性检查、闭合状态识别
        ↓
多周期聚合与时间对齐
        ↓
指标计算、warmup 与可用性检查
        ↓
特征归一化与历史分位数计算
        ↓
维度评分：震荡、趋势、波动、动能、成交量、价格结构、成本适配
        ↓
评分平滑、评分动能、互斥修正
        ↓
多周期合并决策
        ↓
状态机：候选状态 + 确认期 + 冷却期 + 滞后阈值 + 假突破回归
        ↓
网格计划 / 可执行风控动作 / TradingView 信号
        ↓
API 输出 / 入库 / 监控 / 回测 / 灰度验证
```

本模块负责 K线分析、指标计算、行情评分、状态机、网格计划、风险等级、前端展示数据、回测和复盘所需稳定契约。本模块不负责自动下单、账户管理、资金划转、保证盈利或替代回测。

---

## 3. 数据输入

### 3.1 K线 API

```text
GET /api/v1/public/market/klines
```

| 参数 | 是否必填 | 类型 | 说明 |
|---|---:|---|---|
| `source` | 否 | string | 数据来源，不传时后端默认一般为 `binance` |
| `symbol` | 是 | string | 交易对，例如 `BTCUSDT` |
| `interval` | 是 | string | 周期，例如 `1m`、`5m`、`15m`、`30m`、`1h`、`4h`、`1d` |
| `startTime` | 否 | i64 | 开始时间，毫秒时间戳；后端可兼容 `start_time` |
| `endTime` | 否 | i64 | 结束时间，毫秒时间戳；后端可兼容 `end_time` |
| `limit` | 否 | i64 | 返回数量，默认 500，最大 1000 |

### 3.2 CSV 输入

标准字段：

```csv
open_time,open,high,low,close,volume
```

数据库字段别名：

```csv
open_time,open_price,high_price,low_price,close_price,base_volume
```

可选字段：

```csv
close_time,quote_volume,trade_count,taker_buy_base_volume,taker_buy_quote_volume,is_closed
```

### 3.3 API 返回结构兼容

支持直接数组、`data/items/rows/list/klines` 包裹结构、对象式 K线和数组式 K线。

数组式 K线顺序：

```text
[open_time, open, high, low, close, volume]
```

如果数组包含更多字段，前 6 个字段必须保持上述含义。额外字段可解析为 `close_time`、`quote_volume` 等辅助信息，但不得改变核心 OHLCV 语义。

---

## 4. 数据模型与数据质量

### 4.1 Kline 模型

```rust
pub struct Kline {
    pub open_time: i64,
    pub close_time: Option<i64>,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
    pub quote_volume: Option<f64>,
    pub trade_count: Option<u64>,
    pub is_closed: bool,
}
```

### 4.2 数据校验

每根 K线必须满足：

```text
high >= low
high >= open
high >= close
low <= open
low <= close
volume >= 0
quote_volume 若存在则 >= 0
所有价格和成交量字段均为有限数字，不能为 NaN / Inf
open_time 必须能被 interval 对齐，或记录为质量问题
```

必须检查：

```text
是否存在重复 open_time
是否存在缺失 K线
是否存在乱序 K线
是否包含未闭合 K线
最新 K线延迟是否超过阈值
K线数量是否满足核心指标 warmup 要求
```

策略状态确认只允许使用已闭合 K线。未闭合 K线只能用于实时观察和前端提示，不得用于 `confirmed` 状态、冷却期推进、候选状态计数或风控强制动作。

### 4.3 DataQuality

```rust
pub struct DataQuality {
    pub input_kline_count: usize,
    pub usable_closed_kline_count: usize,
    pub missing_kline_count: usize,
    pub duplicate_kline_count: usize,
    pub out_of_order_count: usize,
    pub invalid_ohlcv_count: usize,
    pub has_gap: bool,
    pub has_unclosed_kline: bool,
    pub latest_kline_delay_ms: i64,
    pub warmup_satisfied: bool,
    pub quality_score: f64,
    pub issues: Vec<String>,
}
```

`quality_score` 范围为 `0.0 ~ 1.0`。若存在严重缺口、延迟、核心指标不可用或未闭合 K线试图参与确认，必须降低置信度或进入 `wait` / `risk_control`。

建议处理：

| 问题 | 处理 |
|---|---|
| 非法 OHLCV | 丢弃非法 K线；超过阈值返回错误或进入 `wait` |
| 重复 open_time | 保留最后一条或按数据源规则合并，并记录 issue |
| 乱序 K线 | 排序后继续处理，记录 issue |
| 缺失 K线 | 轻微缺失降低 `quality_score`；严重缺失进入 `wait` |
| latest 延迟超过 1 个周期 | 降低置信度 |
| latest 延迟超过 2 个周期 | 禁止新开普通网格 |
| warmup 不满足 | 不输出 confirmed 状态，只允许 `wait` |
| 未闭合 K线参与确认 | 禁止，视为实现错误 |

---

## 5. 多周期设计

### 5.1 周期分工

| 周期 | 用途 |
|---|---|
| `4h` / `1h` | 大方向与系统性风险过滤 |
| `30m` / `15m` | 判断当前是否适合网格 |
| `5m` / `1m` | 网格触发、信号展示、成交模拟 |
| `1d` / `1w` | 长期趋势与极端风险背景 |

默认组合：

| 模式 | higher | middle | lower |
|---|---|---|---|
| 短线观察 | `1h` | `15m` | `5m` |
| 常规网格 | `4h` | `30m` | `5m` |
| 高频展示 | `1h` | `15m` | `1m` |
| 日内风控 | `1d` | `1h` | `15m` |

### 5.2 多周期输入结构

```rust
pub struct MultiTimeframeInput {
    pub higher: TimeframeAnalysis,
    pub middle: TimeframeAnalysis,
    pub lower: TimeframeAnalysis,
}

pub struct TimeframeSnapshotRef {
    pub source: String,
    pub symbol: String,
    pub interval: String,
    pub open_time: i64,
    pub close_time: i64,
    pub is_closed: bool,
    pub state: MarketState,
    pub raw_scores: Scores,
    pub smoothed_scores: Scores,
    pub confidence: f64,
}
```

### 5.3 多周期时间对齐规则

生产和回测必须使用相同的时间对齐规则：

```text
所有状态确认只使用各周期最新已闭合 K线。
lower timeframe 的时间点 T，只能引用 close_time <= T 的 higher / middle timeframe 分析结果。
不得用尚未闭合的 higher timeframe K线参与 lower timeframe 状态确认。
如果 higher timeframe 最新已闭合结果延迟超过配置阈值，则降低 confidence 或进入 wait。
每个 multi-timeframe 输出必须记录实际引用的各周期 snapshot open_time / close_time。
```

### 5.4 多周期合并决策矩阵

| 大周期状态 | 中周期状态 | 小周期状态 | 最终状态/动作 |
|---|---|---|---|
| `downtrend_risk` confirmed | 任意 | 任意 | 禁止普通多头网格，`hard_block` |
| `down_break_warning` confirmed | 任意 | 任意 | 暂停新增买单，`soft_block` |
| 任意 | `downtrend_risk` confirmed | 任意 | 关闭普通网格，`hard_block` |
| 任意 | `down_break_warning` confirmed | 任意 | 暂停新增买单，取消下方补仓单 |
| `uptrend_follow` | `range_grid` | 无下跌风险 | 只允许上涨跟随网格，不允许固定震荡网格 |
| `range_grid` / `wait` | `range_grid` | 接近区间下沿且无破位 | 允许普通震荡网格 |
| `wait` | `range_grid` | 无破位 | 小资金观察模式或等待确认 |
| 任意 | 任意 | `down_break_warning` | 不新增买单，等待下一根确认 |
| 任意 | 任意 | `up_break_warning` | 减少卖出，准备上移网格 |

优先级：

```text
P0：全局硬止损 > 大周期下跌风险 > 中周期下跌风险 > 小周期破位预警
P1：风控阻断优先于网格开启
P2：上涨趋势中只允许趋势跟随网格，不允许固定震荡网格
P3：只有多周期均无下跌风险且中周期震荡成立，才允许普通网格
```

---

## 6. 指标体系

采用“按维度评分”，而不是简单按指标相加。BOLL、ATR、MACD、ADX、MA 等指标存在信息重叠，不能让同一类证据重复计分。

| 指标 | 作用 | 优先级 |
|---|---|---:|
| BOLL | 区间、波动带、价格位置 | MVP |
| MACD | 动能方向与动能变化 | MVP |
| ATR | 真实波动率、网格间距 | MVP |
| ADX / DMI | 趋势强度、多空方向压力 | MVP |
| MA20 / MA60 | 趋势方向、均线粘合、支撑压力 | MVP |
| RSI | 超买超卖、震荡区间位置 | MVP |
| Volume Ratio | 突破确认、假突破过滤 | MVP |
| Donchian Channel | 箱体高低点突破 | v1-production |
| `%B` | BOLL 区间相对位置 | v1-production |
| EMA20 偏离率 | 价格相对中期均值偏离 | v1-production |
| Price Structure | 高低点结构、箱体边界 | v1-production |
| Fee / Slippage | 网格收益空间判断 | v1-production |
| Liquidity / Spread | 实盘成交质量 | v1-production |
| Keltner Channel | 突破和波动扩张二次确认 | v1.1 |
| OBV / VWAP 偏离 | 成交量和成交成本辅助 | v1.1 |
| Order Book Imbalance | 高频盘口预警 | v2 |

---

## 7. 指标计算规范

### 7.1 通用计算边界

```text
所有指标默认只基于已闭合 K线计算。
指标不足 warmup 时返回 null / unavailable，不得用 0 代替。
所有 NaN / Inf 必须转换为 unavailable，并写入 data_quality.issues。
所有百分比类指标统一用小数表示，例如 1% = 0.01。
所有评分输入必须先经过 finite 检查。
```

推荐 warmup：

| 指标 | 最小可用 K线 | 建议 warmup |
|---|---:|---:|
| MA20 / EMA20 | 20 | 60 |
| MA60 | 60 | 120 |
| BOLL20 | 20 | 60 |
| MACD 12/26/9 | 35 | 120 |
| ATR14 | 15 | 100 |
| ADX/DMI14 | 28 | 150 |
| RSI14 | 15 | 100 |
| Volume Ratio20 | 20 | 60 |
| Donchian20 | 20 | 60 |
| Percentile1000 | 100 | 1000 |

```rust
pub struct IndicatorAvailability {
    pub min_required_bars: usize,
    pub warmup_bars: usize,
    pub ready: bool,
    pub unavailable_fields: Vec<String>,
}
```

状态确认必须在 MVP 核心指标 ready 后进行；若非核心增强指标不可用，可降低 confidence，但不得强行退出整个分析。

### 7.2 MA / EMA

```text
ma_spread = (max(MA20, MA60) - min(MA20, MA60)) / close
ma20_slope = (MA20_now - MA20_n_bars_ago) / MA20_n_bars_ago
ema20_deviation = (close - EMA20) / EMA20
```

EMA 默认使用首个 EMA period 的 SMA 作为初始值。历史数据不足时返回 unavailable。

### 7.3 BOLL 与 %B

默认：`period = 20`，`multiplier = 2.0`。

```text
mid = SMA(close, period)
std = population_standard_deviation(close, period)
upper = mid + multiplier * std
lower = mid - multiplier * std
bandwidth = (upper - lower) / abs(mid)
percent_b = (close - lower) / (upper - lower)
```

特殊情况：

```text
如果 upper == lower，则 percent_b unavailable，bandwidth = 0。
如果 mid == 0，则 bandwidth unavailable。
```

### 7.4 MACD

默认：`fast = 12`、`slow = 26`、`signal = 9`。

```text
dif = EMA(close, fast) - EMA(close, slow)
dea = EMA(dif, signal)
hist = dif - dea
金叉：hist_prev <= 0 && hist_now > 0
死叉：hist_prev >= 0 && hist_now < 0
```

### 7.5 ATR

默认：`period = 14`。

```text
TR = max(high - low, abs(high - previous_close), abs(low - previous_close))
首个 ATR = 前 period 个 TR 的 SMA
后续 ATR = (previous_ATR * (period - 1) + TR_now) / period
```

### 7.6 ADX / DMI

默认：`period = 14`。

```text
ADX < 20：趋势弱
20 <= ADX < 25：趋势开始增强
ADX >= 25：趋势较明显
+DI > -DI：多头方向压力更强
-DI > +DI：空头方向压力更强
```

实际阈值必须按 `source + symbol + interval` 回测校准。

### 7.7 RSI

默认：`period = 14`。

```text
RSI 40~60：偏中性，适合震荡判断
RSI > 70：短期超买
RSI < 30：短期超卖
```

RSI 使用 Wilder 平滑。RSI 不单独决定买卖，只作为位置和风险辅助。

### 7.8 Volume Ratio

默认：`volume_ma_period = 20`。

```text
volume_ratio = current_volume / SMA(volume, 20)
volume_ratio >= 1.5：放量
0.7 <= volume_ratio <= 1.3：成交量平稳
volume_ratio < 0.7：缩量
```

如果 `SMA(volume, 20) == 0`，则 `volume_ratio` unavailable。

### 7.9 Donchian Channel

默认：`N = 20`。

```text
donchian_high = highest(high, N)
donchian_low = lowest(low, N)
close > donchian_high_prev：向上突破候选
close < donchian_low_prev：向下破位候选
```

突破判断建议使用上一根之前形成的通道边界，避免当前 K线同时参与边界计算导致永远不突破。

### 7.10 Keltner Channel（v1.1）

```text
keltner_mid = EMA(close, 20)
keltner_upper = keltner_mid + 2 * ATR(20)
keltner_lower = keltner_mid - 2 * ATR(20)
```

### 7.11 Price Structure

需要识别：

```text
higher_high
higher_low
lower_high
lower_low
range_high
range_low
swing_high
swing_low
```

推荐 swing 识别规则：

```text
swing_high：当前 high 高于左右各 pivot_left / pivot_right 根 K线 high
swing_low：当前 low 低于左右各 pivot_left / pivot_right 根 K线 low
```

为了避免未来函数，实时模式下只有当右侧 `pivot_right` 根 K线闭合后，才能确认 swing 点。

---

## 8. 特征归一化与分位数

生产版本必须支持按 `source + symbol + interval` 的历史分位数。

建议滚动窗口：

```text
500 ~ 2000 根 K线
```

需要计算分位数的特征：

```text
BOLL bandwidth
ATR / close
volume_ratio
ma_spread
ema20_deviation
recent_range_width
grid_step / close
spread / close
liquidity_depth
```

分位数解释：

```text
< 15%：极低
15% ~ 30%：偏低
30% ~ 70%：正常
70% ~ 85%：偏高
> 85%：极高
```

样本数不足时，使用固定阈值兜底并降低 confidence；样本数满足 `percentile_min_samples` 后，优先使用历史分位数。

---

## 9. 评分系统设计

### 9.1 总体原则

```text
range_score = 趋势弱分 + 波动适配分 + 均线粘合分 + 价格往返分 + RSI 中性分 + 成交平稳分 + 成本适配分
up_score = 价格方向分 + 动能增强分 + 趋势增强分 + 突破确认分 + 成交量确认分 + 价格结构分
down_score = 价格方向分 + 动能转弱分 + 趋势增强分 + 破位确认分 + 成交量确认分 + 价格结构分
```

要求：

1. 每个评分范围 `0 ~ 100`。
2. 每个子维度先计算 `0 ~ 100`，再乘权重。
3. 同一类证据不能重复加分。
4. 默认权重是初始经验值，必须回测校准。
5. 所有阈值必须配置化。
6. 每个评分输出必须包含 reasons，便于前端解释和复盘。
7. 如果关键子维度 unavailable，应降低 confidence，并在 reasons 中说明。

### 9.2 通用线性函数

```text
linear_down(x, x_low, x_high):
  x <= x_low  -> 100
  x >= x_high -> 0
  otherwise   -> 100 * (x_high - x) / (x_high - x_low)

linear_up(x, x_low, x_high):
  x <= x_low  -> 0
  x >= x_high -> 100
  otherwise   -> 100 * (x - x_low) / (x_high - x_low)
```

所有输出必须 clamp 到 `0 ~ 100`。

### 9.3 `range_score` 默认权重

| 维度 | 权重 | 评分函数 |
|---|---:|---|
| 趋势弱 | 25 | `ADX <= 18 -> 100`; `18~25` 从 100 降到 40；`25~35` 从 40 降到 0；`>35 -> 0` |
| 波动适配 | 20 | `bbw_percentile < 15% -> 40`; `15~30% -> 80`; `30~70% -> 100`; `70~85% -> 50`; `>85% -> 0` |
| 均线粘合 | 15 | `ma_spread <= 1% -> 100`; `1~3%` 从 100 降到 40；`>5% -> 0` |
| 价格往返 | 15 | 最近 `lookback` 内穿越 MA20/BOLL mid 次数越多分越高，`>=3` 次为 100 |
| RSI 中性 | 10 | `RSI 45~55 -> 100`; `40~60 -> 80`; `30~70 -> 40`; 其他为 0 |
| 成交平稳 | 10 | `volume_ratio 0.8~1.2 -> 100`; `0.6~1.5 -> 60`; 其他为 20 或 0 |
| 成本适配 | 5 | `grid_step / center > 2*fee + slippage + buffer -> 100`，否则 0 |

默认进入条件：

```text
range_score >= 65
up_score < 55
down_score < 55
```

默认退出条件：

```text
range_score < 55
或 up_score >= 60
或 down_score >= 60
```

### 9.4 `up_score` 默认权重

| 维度 | 权重 | 评分函数 |
|---|---:|---|
| 价格方向 | 20 | close > MA20 且 MA20 斜率向上，或 percent_b > 0.8 为高分 |
| 动能增强 | 20 | MACD 金叉、hist > 0 且连续放大为高分 |
| 趋势增强 | 20 | +DI > -DI 且 ADX 上升为高分 |
| 突破确认 | 15 | close 突破 BOLL upper、Donchian high 或 range_high 为高分 |
| 成交量确认 | 15 | 突破时 volume_ratio >= 1.5 为高分；缩量突破降权 |
| 价格结构 | 10 | higher_high + higher_low 为高分 |

默认判定：

```text
55 <= up_score < 70：up_break_warning
up_score >= 70：uptrend_follow candidate
up_score >= 80 且连续 confirm_bars 根闭合 K线：uptrend_follow confirmed
```

### 9.5 `down_score` 默认权重

| 维度 | 权重 | 评分函数 |
|---|---:|---|
| 价格方向 | 20 | close < MA20 且 MA20 斜率向下，或 percent_b < 0.2 为高分 |
| 动能转弱 | 20 | MACD 死叉、hist < 0 且负值扩大为高分 |
| 趋势增强 | 20 | -DI > +DI 且 ADX 上升为高分 |
| 破位确认 | 15 | close 跌破 BOLL lower、Donchian low 或 range_low 为高分 |
| 成交量确认 | 15 | 下跌时 volume_ratio >= 1.5 为高分；缩量阴跌给中等风险分 |
| 价格结构 | 10 | lower_high + lower_low 为高分 |

默认判定：

```text
55 <= down_score < 70：down_break_warning
down_score >= 70：downtrend_risk candidate
down_score >= 80 且连续 confirm_bars 根闭合 K线：downtrend_risk confirmed
```

### 9.6 评分输出结构

```rust
pub struct Scores {
    pub range_score: f64,
    pub up_score: f64,
    pub down_score: f64,
}

pub struct ScoreDetail {
    pub name: String,
    pub raw_value: Option<f64>,
    pub sub_score: Option<f64>,
    pub weight: f64,
    pub weighted_score: Option<f64>,
    pub available: bool,
    pub reason: String,
}

pub struct ScoreBreakdown {
    pub range: Vec<ScoreDetail>,
    pub up: Vec<ScoreDetail>,
    pub down: Vec<ScoreDetail>,
}
```

---

## 10. 评分平滑、评分动能与互斥修正

三评分必须同时保留原始分和平滑分。

```text
smoothed_score = EMA(raw_score, smooth_period)
```

默认：

```text
smooth_period = 3 或 5
```

评分动能：

```text
range_momentum = range_score_now - range_score_prev
up_momentum = up_score_now - up_score_prev
down_momentum = down_score_now - down_score_prev
```

互斥修正：

```text
if up_score >= 60 or down_score >= 60:
    range_score = range_score * 0.7

if up_score >= 70 and down_score >= 70:
    final_state = wait
    risk_level = advisory 或 soft_block
    reason += "多空评分冲突，进入等待"
```

状态机默认使用 `smoothed_scores`；前端和复盘同时展示 raw 与 smoothed。互斥修正必须记录在 `reasons` 和 `score_breakdown` 中，不能静默修改。

---

## 11. 假突破过滤器

目标是减少短暂突破、缩量突破、插针突破导致的错误状态切换。

向上假突破规则：进入 `up_break_warning` 后，若满足以下条件，则回到 `range_grid` 或 `wait`：

```text
价格在 fake_breakout_window 根 K线内重新回到 BOLL 上轨或 Donchian 上轨下方
volume_ratio < breakout_volume_confirm_threshold
ADX 未继续上升
up_score 回落到 warning_exit 以下
```

向下假突破规则：进入 `down_break_warning` 后，若满足以下条件，则不立即进入 `downtrend_risk`：

```text
价格在 fake_breakout_window 根 K线内重新回到 BOLL 下轨或 Donchian 下轨上方
volume_ratio 不支持放量破位
ADX 未继续上升
下跌结构未形成 lower_high + lower_low
```

真突破确认至少需要满足两类以上证据：

```text
价格证据：收盘价有效突破 BOLL / Donchian / range 边界
波动证据：BOLL bandwidth 或 ATR percentile 上升
趋势证据：ADX 上升且 DMI 方向一致
成交量证据：volume_ratio >= 阈值
结构证据：higher_high/higher_low 或 lower_high/lower_low 成立
```

插针过滤：若仅 high / low 突破，但 close 回到区间内，默认不视为有效突破。

```text
向上插针：high > upper_boundary && close <= upper_boundary
向下插针：low < lower_boundary && close >= lower_boundary
```

插针可产生 `warning` signal，但不得直接触发 confirmed 趋势状态。

---

## 12. 状态机设计

### 12.1 状态枚举

```rust
pub enum MarketState {
    Wait,
    RangeGrid,
    UpBreakWarning,
    UptrendFollow,
    DownBreakWarning,
    DowntrendRisk,
}
```

### 12.2 状态上下文

```rust
pub struct StateContext {
    pub previous_state: MarketState,
    pub previous_state_since: i64,
    pub candidate_state: Option<MarketState>,
    pub candidate_bars: usize,
    pub cooldown_remaining_bars: usize,
    pub last_transition_time: Option<i64>,
    pub last_grid_exit_time: Option<i64>,
    pub last_stop_loss_time: Option<i64>,
}

pub struct StateTransition {
    pub previous_state: MarketState,
    pub candidate_state: Option<MarketState>,
    pub final_state: MarketState,
    pub transition_type: String,
    pub candidate_bars: usize,
    pub cooldown_remaining_bars: usize,
    pub reasons: Vec<String>,
}
```

### 12.3 状态动作

| 状态 | 含义 | 默认动作 |
|---|---|---|
| `wait` | 方向不清晰或数据质量不足 | 不开普通网格 |
| `range_grid` | 震荡条件成立 | 允许普通震荡网格 |
| `up_break_warning` | 震荡可能向上失效 | 减少卖出，准备上移网格 |
| `uptrend_follow` | 上涨趋势确认 | 关闭普通震荡网格，允许趋势跟随网格 |
| `down_break_warning` | 震荡可能向下失效 | 暂停新增买单，取消下方补仓单 |
| `downtrend_risk` | 下跌趋势风险确认 | 关闭普通网格，禁止新开多头网格，减仓或止损 |

### 12.4 确认期、冷却期与滞后阈值

默认：

```text
confirm_bars = 3
cooldown_bars_after_exit = 5
cooldown_bars_after_stop_loss = 20
```

规则：

1. 进入趋势确认状态必须满足连续 `confirm_bars` 根闭合 K线。
2. 退出网格后，冷却期内不能重新进入普通网格。
3. 止损后，必须使用更长冷却期。
4. 进入状态和退出状态使用不同阈值，避免临界抖动。
5. 假突破回归路径优先于趋势确认。
6. 候选状态计数只能由已闭合 K线推进。

### 12.5 状态迁移表

| From | Candidate / To | 条件 | 确认 | 冷却 | 默认动作 |
|---|---|---|---|---|---|
| `wait` | `range_grid` | `range_score >= range_enter` 且 `up/down < warning_enter` 且数据质量合格 | `confirm_bars` | 无 | 允许普通网格 |
| `range_grid` | `up_break_warning` | `up_score >= warning_enter` 或上沿突破候选 | 1 bar | 无 | 不新增普通固定网格，减少卖出密度 |
| `range_grid` | `down_break_warning` | `down_score >= warning_enter` 或下沿破位候选 | 1 bar | 无 | 暂停新增买单，取消下方补仓单 |
| `up_break_warning` | `uptrend_follow` | `up_score >= trend_confirm` 且真突破证据 >= 2 类 | `confirm_bars` | `cooldown_bars_after_exit` | 切换上涨跟随模式 |
| `down_break_warning` | `downtrend_risk` | `down_score >= trend_confirm` 且真破位证据 >= 2 类 | `confirm_bars` | `cooldown_bars_after_exit` 或止损冷却 | hard block |
| `up_break_warning` | `range_grid` | fake breakout confirmed 且 range 条件仍成立 | 1 bar | 可选 1~2 bars | 恢复普通网格或等待 |
| `down_break_warning` | `range_grid` | fake breakdown confirmed 且 range 条件仍成立 | 1 bar | 可选 1~2 bars | 谨慎恢复，默认小资金 |
| `uptrend_follow` | `wait` | up_score 低于 exit 且 range 不成立 | `confirm_bars` | `cooldown_bars_after_exit` | 停止趋势跟随新增 |
| `downtrend_risk` | `wait` | down_score 低于 exit 且无新低结构 | `confirm_bars` | `cooldown_bars_after_stop_loss` | 解除 hard block 但不立即开网格 |
| 任意 | `wait` | 数据质量不足或指标 unavailable | 1 bar | 无 | 不开新网格 |
| 任意 | `risk_control` | 全局硬止损触发 | 立即 | `cooldown_bars_after_stop_loss` | emergency stop |

优先级：

```text
1. 全局硬止损 / emergency_stop
2. 数据质量严重不足
3. confirmed downtrend_risk
4. confirmed down_break_warning
5. confirmed uptrend_follow
6. warning 状态
7. range_grid
8. wait
```

---

## 13. 风控等级与可执行动作

### 13.1 RiskLevel

```rust
pub enum RiskLevel {
    Advisory,
    SoftBlock,
    HardBlock,
    EmergencyStop,
}
```

| 风控等级 | 含义 | 动作 |
|---|---|---|
| `advisory` | 仅提示 | 前端展示，不阻断 |
| `soft_block` | 软阻断 | 禁止新增订单，不处理已有仓位 |
| `hard_block` | 硬阻断 | 取消未成交单，降低仓位 |
| `emergency_stop` | 紧急停止 | 停止策略，人工或高优先级流程介入 |

### 13.2 可执行 RiskDecision

分析层必须输出足够明确的动作字段，避免执行层自行解释自然语言。

```rust
pub enum PositionAction {
    Hold,
    ReduceOnly,
    ReduceByRatio,
    StopLoss,
    ManualReview,
}

pub struct RiskDecision {
    pub risk_level: RiskLevel,
    pub allow_new_grid: bool,
    pub allow_new_buy_orders: bool,
    pub allow_new_sell_orders: bool,
    pub allow_replace_grid: bool,
    pub cancel_open_buy_orders: bool,
    pub cancel_open_sell_orders: bool,
    pub cancel_buy_orders_below_price: Option<f64>,
    pub cancel_sell_orders_above_price: Option<f64>,
    pub position_action: PositionAction,
    pub reduce_position_ratio: Option<f64>,
    pub require_manual_confirm: bool,
    pub action_ttl_bars: usize,
    pub reasons: Vec<String>,
}
```

默认映射：

| 状态 | risk_level | allow_new_grid | allow_new_buy_orders | allow_new_sell_orders | position_action |
|---|---|---:|---:|---:|---|
| `wait` | `advisory` | false | false | false | `Hold` |
| `range_grid` | `advisory` | true | true | true | `Hold` |
| `up_break_warning` | `advisory` / `soft_block` | false | false | true | `Hold` |
| `uptrend_follow` | `advisory` | true，仅趋势跟随 | true | true | `Hold` |
| `down_break_warning` | `soft_block` | false | false | true | `ReduceOnly` |
| `downtrend_risk` | `hard_block` | false | false | false | `ReduceByRatio` 或 `StopLoss` |
| 全局硬止损 | `emergency_stop` | false | false | false | `StopLoss` / `ManualReview` |

### 13.3 全局硬止损

无论状态机结果如何，必须配置全局硬止损：

```text
max_loss_per_symbol
max_daily_loss
max_drawdown
max_position_ratio
max_grid_capital
```

若触发：

```text
risk_level = emergency_stop
停止策略
取消未成交单
记录告警
人工或高优先级风控流程介入
```

全局硬止损优先级高于任何评分、状态机和网格计划。

---

## 14. 网格计划设计

### 14.1 GridPlan 结构

```rust
pub enum GridMode {
    Wait,
    RangeGrid,
    UptrendFollow,
    RiskControl,
    StopOrReduce,
}

pub struct GridPlan {
    pub enabled: bool,
    pub mode: GridMode,
    pub boundary_mode: String,
    pub lower: Option<f64>,
    pub upper: Option<f64>,
    pub center: Option<f64>,
    pub grid_count: usize,
    pub grid_step: Option<f64>,
    pub price_levels: Vec<f64>,
    pub risk_level: RiskLevel,
    pub risk_action: String,
    pub confidence: f64,
}
```

### 14.2 普通网格允许条件

即使状态为 `range_grid`，仍必须满足：

```text
数据质量合格
核心指标 ready
波动率未失控
流动性满足要求
单格利润覆盖手续费、滑点和最小利润缓冲
大周期无 downtrend_risk
中周期 range_grid 成立
状态不在冷却期
未触发全局硬止损
交易所规则允许下单
```

### 14.3 边界模式

```text
boundary_mode = boll | range | blended
```

| 模式 | 规则 |
|---|---|
| `boll` | `lower = boll.lower`, `upper = boll.upper` |
| `range` | `lower = range_low`, `upper = range_high` |
| `blended` | BOLL 边界与箱体边界加权融合 |

限制条件：

```text
max_grid_width_by_atr
max_grid_width_by_percent
max_capital_usage_at_lower_bound
grid_step / center > 2 * fee_rate + expected_slippage_rate + min_profit_buffer
```

### 14.4 预警状态动作

`up_break_warning`：

```text
不新增普通固定网格
减少卖出密度
准备上移网格
已有网格可以只做止盈，不主动加反向仓
```

`down_break_warning`：

```text
暂停新增买单
取消下方补仓单
已有仓位进入风险观察
等待 confirmed 后再执行 hard_block 或止损
```

### 14.5 交易所约束

实盘或准实盘输出必须考虑交易所规则，否则 GridPlan 只能用于展示，不能用于执行。

```rust
pub struct ExchangeConstraints {
    pub tick_size: f64,
    pub step_size: f64,
    pub min_qty: f64,
    pub min_notional: f64,
    pub price_precision: u32,
    pub quantity_precision: u32,
    pub max_open_orders: Option<usize>,
    pub maker_fee_rate: f64,
    pub taker_fee_rate: f64,
}
```

规则：

```text
所有 price levels 必须按 tick_size round。
所有 quantity 必须按 step_size round。
round 后若低于 min_notional，则该网格层不可执行。
如果 grid_count 超过 max_open_orders，必须降低 grid_count 或返回 disabled。
成本判断优先使用 maker_fee_rate；若策略可能市价退出，止损成本必须使用 taker_fee_rate。
```

---

## 15. 信号输出设计

```rust
pub enum SignalType {
    GridBuyWatch,
    GridSellWatch,
    UpBreakWarning,
    DownBreakWarning,
    PauseGrid,
    ResumeGrid,
    MoveGridUp,
    MoveGridDown,
    RiskReduce,
}

pub struct Signal {
    pub time: i64,
    pub price: f64,
    pub signal_type: SignalType,
    pub strength: f64,
    pub text: String,
}
```

信号规则：

```text
signal 是展示和复盘标记，不等于订单。
每个 signal 必须绑定产生它的 state、score 和 reason。
同一根 K线上重复信号应合并或去重。
```

---

## 16. 输出 Schema 与 JSON 契约

### 16.1 版本化要求

输出必须包含：

```text
schema_version
model_version
config_version
source
symbol
interval
time
generated_at
is_closed_kline
data_quality
indicator_availability
raw_scores
smoothed_scores
score_momentum
score_breakdown
state_transition
grid_plan
risk_decision
signals
```

### 16.2 confidence 定义

```text
base_confidence = 主状态评分 / 100
multi_tf_factor = 多周期一致性系数，范围 0.5 ~ 1.2
data_quality_factor = data_quality.quality_score
indicator_factor = 核心指标可用性系数，范围 0.5 ~ 1.0
confidence = clamp(base_confidence * multi_tf_factor * data_quality_factor * indicator_factor, 0.0, 1.0)
```

### 16.3 JSON 示例

```json
{
  "schema_version": "1.1",
  "model_version": "rule-v1",
  "config_version": "grid-analysis-v1",
  "source": "binance",
  "symbol": "BTCUSDT",
  "interval": "5m",
  "time": 1710000000000,
  "generated_at": 1710000060000,
  "is_closed_kline": true,
  "state": "range_grid",
  "raw_scores": { "range_score": 74, "up_score": 20, "down_score": 12 },
  "smoothed_scores": { "range_score": 72, "up_score": 18, "down_score": 10 },
  "score_momentum": { "range_momentum": 2, "up_momentum": -1, "down_momentum": 0 },
  "confidence": 0.72,
  "data_quality": {
    "input_kline_count": 1000,
    "usable_closed_kline_count": 1000,
    "missing_kline_count": 0,
    "duplicate_kline_count": 0,
    "out_of_order_count": 0,
    "invalid_ohlcv_count": 0,
    "has_gap": false,
    "has_unclosed_kline": false,
    "latest_kline_delay_ms": 0,
    "warmup_satisfied": true,
    "quality_score": 1.0,
    "issues": []
  },
  "grid_plan": {
    "enabled": true,
    "mode": "range_grid",
    "boundary_mode": "boll",
    "lower": 67360.0,
    "upper": 68600.0,
    "center": 67980.0,
    "grid_count": 20,
    "grid_step": 62.0,
    "price_levels": [],
    "risk_level": "advisory",
    "risk_action": "normal",
    "confidence": 0.72
  },
  "risk_decision": {
    "risk_level": "advisory",
    "allow_new_grid": true,
    "allow_new_buy_orders": true,
    "allow_new_sell_orders": true,
    "allow_replace_grid": true,
    "cancel_open_buy_orders": false,
    "cancel_open_sell_orders": false,
    "cancel_buy_orders_below_price": null,
    "cancel_sell_orders_above_price": null,
    "position_action": "hold",
    "reduce_position_ratio": null,
    "require_manual_confirm": false,
    "action_ttl_bars": 1,
    "reasons": ["ADX 较低，趋势强度偏弱", "BOLL 带宽处于正常分位", "成交量平稳"]
  },
  "signals": []
}
```

---

## 17. 分析服务 API 设计

```text
GET /api/v1/analysis/market-state
GET /api/v1/analysis/grid-plan
GET /api/v1/analysis/signals
GET /api/v1/analysis/multi-timeframe-state
GET /api/v1/analysis/marks
```

`/api/v1/analysis/marks` 用于 TradingView `getMarks` / `getTimescaleMarks`。

建议参数：

| 参数 | 是否必填 | 说明 |
|---|---:|---|
| `source` | 否 | 数据源 |
| `symbol` | 是 | 交易对 |
| `interval` | 是 | 分析周期 |
| `time` | 否 | 指定 K线时间，不传则使用最新已闭合 K线 |
| `config_version` | 否 | 指定配置版本 |
| `mode` | 否 | `single` / `multi_timeframe` |

错误码建议：

| code | 含义 |
|---|---|
| `INSUFFICIENT_KLINES` | K线不足，指标 warmup 不满足 |
| `DATA_QUALITY_LOW` | 数据质量过低 |
| `INDICATOR_UNAVAILABLE` | 核心指标不可用 |
| `CONFIG_NOT_FOUND` | 配置版本不存在 |
| `EXCHANGE_CONSTRAINT_FAILED` | 交易所约束不满足 |

---

## 18. 数据库设计建议

### 18.1 analysis_market_states

```text
source
symbol
interval
open_time
close_time
schema_version
model_version
config_version
state
raw_scores jsonb
smoothed_scores jsonb
score_momentum jsonb
score_breakdown jsonb
confidence
data_quality jsonb
indicator_availability jsonb
reasons jsonb
created_at
```

唯一键：

```text
source + symbol + interval + open_time + model_version + config_version
```

### 18.2 analysis_state_transitions

```text
source
symbol
interval
open_time
previous_state
candidate_state
final_state
transition_type
candidate_bars
cooldown_remaining_bars
transition_reason jsonb
model_version
config_version
created_at
```

### 18.3 analysis_signals

```text
source
symbol
interval
open_time
signal_type
price
strength
text
model_version
config_version
created_at
```

### 18.4 analysis_grid_plans

```text
source
symbol
interval
open_time
mode
boundary_mode
enabled
lower
upper
center
grid_count
grid_step
price_levels jsonb
risk_level
risk_action
risk_decision jsonb
confidence
model_version
config_version
created_at
```

### 18.5 analysis_backtest_runs

```text
run_id
source
symbols jsonb
intervals jsonb
start_time
end_time
model_version
config_version
cost_model jsonb
exchange_constraints jsonb
metrics jsonb
created_at
```

### 18.6 analysis_config_versions

```text
config_version
config jsonb
created_by
created_at
notes
parent_config_version
backtest_run_id
approved_for_shadow
approved_for_gray
approved_for_production
```

---

## 19. 配置化要求

所有阈值必须配置化，不能硬编码为不可调整逻辑。

```json
{
  "indicator": {
    "boll_period": 20,
    "boll_mult": 2.0,
    "macd_fast": 12,
    "macd_slow": 26,
    "macd_signal": 9,
    "atr_period": 14,
    "adx_period": 14,
    "rsi_period": 14,
    "volume_ma_period": 20,
    "donchian_period": 20,
    "score_smooth_period": 3,
    "percentile_window": 1000,
    "percentile_min_samples": 100,
    "pivot_left": 2,
    "pivot_right": 2,
    "structure_lookback": 20
  },
  "state": {
    "range_enter": 65,
    "range_exit": 55,
    "warning_enter": 55,
    "warning_exit": 45,
    "trend_candidate": 70,
    "trend_confirm": 80,
    "confirm_bars": 3,
    "fake_breakout_window": 3,
    "breakout_volume_confirm_threshold": 1.5,
    "cooldown_bars_after_exit": 5,
    "cooldown_bars_after_stop_loss": 20
  },
  "grid": {
    "grid_count": 20,
    "boundary_mode": "boll",
    "min_profit_buffer": 0.001,
    "fee_rate": 0.001,
    "expected_slippage_rate": 0.0005,
    "max_grid_width_by_percent": 0.08,
    "max_grid_width_by_atr": 6.0,
    "max_capital_usage_at_lower_bound": 0.5
  },
  "risk": {
    "max_position_ratio": 0.3,
    "max_grid_capital": 1000,
    "max_loss_per_symbol": 0.03,
    "max_daily_loss": 0.05,
    "max_drawdown": 0.1,
    "default_reduce_position_ratio": 0.3
  },
  "data_quality": {
    "max_latest_delay_intervals": 2,
    "max_missing_kline_ratio": 0.01,
    "min_quality_score_for_grid": 0.9
  }
}
```

---

## 20. 参数校准与防过拟合

参数校准必须至少包含：

```text
训练集：用于搜索参数
验证集：用于选择参数
样本外测试集：最终确认，不允许反复调参
walk-forward：按时间滚动训练/验证
```

禁止只基于单个交易对、单个周期、单段行情优化参数。

优化目标优先级：

```text
1. 降低最大回撤
2. 降低灾难性连续补仓
3. 控制状态切换频率
4. 降低误杀和漏判
5. 在上述约束下提升收益回撤比
```

每次调整权重或阈值必须：

```text
记录 config_version
跑固定回归回测集
生成与上一版本的指标对比
人工确认是否进入 shadow / gray / production
保留可回滚版本
```

防过拟合要求：

```text
参数数量越多，越需要更大的样本外数据集。
不得为了单一历史暴跌或单一山寨币插针过度调参。
如果新参数提升收益但显著增加最大回撤，不得进入 production。
如果新参数降低收益但显著降低回撤，可进入只读分析或小资金灰度。
```

---

## 21. 前端 TradingView 集成

第一阶段：

```text
TradingView 内置 BOLL / MACD 负责视觉指标
services/klines-tools 输出 grid_plan 画网格线
services/klines-tools 输出 signals 画 marker
services/klines-tools 输出 state/scores/risk 显示右侧状态面板
```

第二阶段：

```text
增加历史指标序列接口
增加 TradingView marks 接口
增加 WebSocket 推送闭合 K线后的分析结果
```

前端展示要求：

```text
必须展示当前 state、risk_level、confidence。
必须展示主要 reasons，不能只展示分数。
warning / hard_block / emergency_stop 必须有明显视觉区分。
未闭合 K线产生的观察信号必须标记为 realtime / unconfirmed。
```

---

## 22. 回测要求与上线验收标准

### 22.1 回测数据要求

```text
多个交易对：BTC、ETH、主流山寨
多个周期：5m、15m、30m、1h
多种行情：震荡、慢涨、急涨、慢跌、暴跌、插针、低波动蓄势
至少 6~12 个月历史数据
```

### 22.2 成本模型

```text
手续费
滑点
买卖价差
最小下单金额
成交延迟
部分成交
资金费率，若用于合约
```

### 22.3 K线级成交模拟规则

网格回测若只使用 OHLCV，必须显式规定成交假设：

```text
限价买单：当 low <= buy_price 时视为可成交。
限价卖单：当 high >= sell_price 时视为可成交。
同一根 K线同时触发多个网格价位时，默认使用对策略不利的成交顺序。
若同一根 K线同时触发止损和止盈，默认先触发对策略不利的路径。
成交价必须考虑滑点、手续费和 tick_size rounding。
未考虑盘口队列位置时，回测结果必须标记为 optimistic 或 conservative。
```

建议提供两种模式：

| 模式 | 说明 |
|---|---|
| conservative | 同 K线内按最不利路径成交，用于上线验收 |
| optimistic | 同 K线内按较有利路径成交，仅用于上限参考 |

### 22.4 对照组

```text
A. 固定网格，不加指标过滤
B. BOLL 区间网格
C. BOLL + MACD + ATR + ADX 过滤网格
D. 生产版维度评分 + 多周期 + 状态机网格
E. 生产版 + Donchian/%B/评分平滑/假突破过滤器
```

### 22.5 核心评价指标

```text
总收益
年化收益
最大回撤
收益回撤比
胜率
盈亏比
交易次数
手续费占比
最大连续亏损
最大浮亏
资金利用率
状态切换次数
误杀次数：退出后立刻反弹
漏判次数：未退出后继续大跌
假突破过滤命中率
灾难性连续补仓次数
```

### 22.6 上线验收标准

生产接入前必须至少满足：

```text
相对固定网格，最大回撤降低 >= 30%
相对固定网格，收益回撤比提升 >= 20%
手续费占毛收益比例低于配置阈值
状态切换频率不能高到导致频繁启停
假突破过滤器减少误杀次数
极端下跌行情中不得出现灾难性连续补仓
BTC、ETH、至少 3 个主流币样本表现稳定
参数变更后必须通过固定回归回测集
```

如果收益下降但回撤显著降低，可以进入只读分析或小资金灰度，不能直接生产放量。

---

## 23. 测试要求

### 23.1 单元测试

```text
CSV 标准字段解析
CSV 别名字段解析
API data 包裹解析
API 数组式 K线解析
时间字符串解析
非法 OHLC 报错
NaN / Inf 拒绝或 unavailable
K线连续性检查
未闭合 K线不能参与状态确认
BOLL 计算
MACD 计算
ATR 计算
ADX 计算
RSI 计算
Volume Ratio 计算
Donchian Channel 计算
%B 计算
EMA20 偏离率计算
Price Structure 识别
分位数计算
评分函数
评分平滑
评分动能
评分互斥修正
假突破过滤器
交易所 tick_size / step_size rounding
```

### 23.2 集成测试

```text
调用 K线 API 成功
API 返回空数据
API 返回非法数据
状态机确认期
状态机冷却期
多周期时间对齐
多周期合并判断
输出 JSON contract 稳定性
risk_decision 动作字段稳定性
```

### 23.3 回归测试

每次调整权重或阈值必须跑固定回测集，输出指标对比。

### 23.4 Golden Tests

建议维护一组固定输入与固定输出的 golden cases：

```text
标准震荡行情 -> range_grid
放量上破 -> up_break_warning / uptrend_follow
放量下破 -> down_break_warning / downtrend_risk
缩量假突破 -> 回归 range_grid 或 wait
数据缺失 -> wait + data_quality issue
未闭合 K线 -> realtime only，不推进状态机
```

---

## 24. 可观测性与告警

关键日志：

```text
source
symbol
interval
open_time
schema_version
model_version
config_version
state
raw_scores
smoothed_scores
score_momentum
grid_plan
risk_level
risk_action
risk_decision
reasons
input_kline_count
missing_kline_count
calculation_latency_ms
```

监控指标：

```text
分析延迟
API 请求失败率
K线缺失数量
状态切换频率
downtrend_risk 触发次数
网格暂停次数
异常波动触发次数
假突破过滤次数
risk_decision hard_block 次数
emergency_stop 次数
```

告警条件：

```text
K线延迟超过 2 个周期
K线连续性缺失
分析任务失败
状态切换异常频繁
多个交易对同时 downtrend_risk
API 错误率超过阈值
触发 emergency_stop
risk_decision 与执行层回执不一致
```

---

## 25. 发布与灰度

### 25.1 阶段一：只读分析

```text
只计算指标和状态
只前端展示
不影响真实交易
```

### 25.2 阶段二：影子回测

```text
实时生成信号
不下单
记录如果按信号执行会怎样
与真实行情对比
```

### 25.3 阶段三：小资金灰度

```text
只允许少量交易对
限制资金比例
启用严格止损
人工确认重要状态切换
```

### 25.4 阶段四：生产策略接入

```text
接入自动风控
接入监控告警
定期回测校准
每次参数变更需要回归测试
```

### 25.5 回滚要求

```text
任意 config_version 必须可回滚。
model_version 变更必须保留旧版本读取和回测能力。
生产中若 hard_block / emergency_stop 异常增多，应自动降级到只读分析或上一稳定配置。
```

---

## 26. 备选方案与扩展路线

### 26.1 机器学习分类器

可选方向：

```text
LightGBM / XGBoost：基于特征窗口分类震荡、上涨、下跌
LSTM / Transformer：直接学习序列状态，复杂度更高
```

上线前提：

```text
必须有清晰标签定义
必须样本外验证
必须防止过拟合
必须保留规则模型作为风控兜底
```

### 26.2 波动率择时模型

更轻量的备选方案：

```text
不强行判断方向
只判断波动率是否适合网格
波动率正常：允许网格
波动率急剧扩张：暂停网格
波动率极低：等待变盘确认
```

可作为 MVP 或回测对照组。

### 26.3 趋势 / 周期分解

可选方法：

```text
Kalman Filter
Hodrick-Prescott Filter
EMA trend + residual oscillation
```

目标：

```text
趋势项判断方向
周期项判断震荡稳定性
残差波动判断是否适合网格
```

### 26.4 盘口微观结构

若后续可获取 Level2 数据，可增加：

```text
order_book_imbalance
spread
depth
large_order_flow
```

用于短周期提前预警，但不作为第一版依赖。

---

## 27. 版本演进计划

### MVP

```text
BOLL / MACD / ATR / ADX / MA / RSI / Volume
基础 range_score / up_score / down_score
单周期状态机
JSON 输出
TradingView 只读展示
全局硬止损
```

### v1-production

```text
多周期合并
Donchian Channel
%B
EMA20 偏离率
评分平滑
评分动能
假突破过滤器
状态上下文
确认期和冷却期
风险等级
数据质量字段
回测验收标准
```

### v1.1

```text
指标 warmup 与可用性契约
多周期时间对齐规则
可执行 risk_decision
交易所约束与 rounding
K线级成交模拟规则
参数校准与防过拟合流程
Golden tests
回滚要求
```

### v2

```text
参数自动校准
机器学习分类器
趋势/周期分解
盘口微观结构
组合风控
```

---

## 28. 关键结论

1. 本模块是行情状态识别与网格风控辅助系统，不是盈利保证系统。
2. 指标之间有重叠，必须按维度评分，避免重复计分。
3. 评分没有通用标准，默认权重只是初始经验值，必须用历史数据回测校准。
4. 高性价比增强项是 Donchian、`%B`、EMA20 偏离率、评分平滑、评分动能、假突破过滤器和全局硬止损。
5. production-ready 必须包含多周期、成交量、价格结构、成本约束、状态上下文、确认期、冷却期、风险等级、数据质量、假突破过滤和回测验收。
6. 真正可执行的 production-ready 还必须包含指标 warmup、时间对齐、状态迁移表、`risk_decision`、交易所约束、成交模拟、参数校准、Golden tests 和回滚机制。
7. 网格策略的重点不是预测最低点，而是识别什么时候不能继续普通网格。
