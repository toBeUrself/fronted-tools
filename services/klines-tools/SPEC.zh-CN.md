# K线指标分析与网格状态识别 Spec（Production Ready）

## 0. 文档信息

| 项目 | 内容 |
|---|---|
| 模块名 | `services/klines-tools` |
| 文档定位 | 需求与系统设计文档，不绑定当前已有代码实现 |
| 核心能力 | K线数据分析、指标计算、行情状态识别、网格策略辅助、TradingView 展示数据输出 |
| 版本 | v1.0 production-ready spec |
| 适用阶段 | 后续重新实施、分析服务设计、前端展示、回测验证、准实盘风控接入 |
| 不适用范围 | 自动下单、账户管理、保证盈利、替代回测 |

> production-ready 指工程需求、接口契约、风控约束、测试、回测、观测和灰度机制达到可生产落地标准；不表示策略一定盈利。所有评分阈值必须通过历史数据、手续费、滑点和流动性回测后才能用于真实交易。

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
```

核心输出：

```text
range_score：震荡/网格适配评分
up_score：上涨/向上突破评分
down_score：下跌/向下破位评分
state：最终行情状态
grid_plan：网格建议
risk：风控动作与风险等级
signals：前端图表可展示信号
reasons：评分和状态判断原因
```

---

## 2. 总体架构

```text
K线 API / CSV / 数据库
        ↓
数据校验、去重、时间排序、连续性检查
        ↓
多周期聚合与指标计算
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
网格计划 / 风控动作 / TradingView 信号
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

请求参数：

| 参数 | 是否必填 | 类型 | 说明 |
|---|---:|---|---|
| `source` | 否 | string | 数据来源，不传时后端默认一般为 `binance` |
| `symbol` | 是 | string | 交易对，例如 `BTCUSDT` |
| `interval` | 是 | string | 周期，例如 `1m`、`5m`、`15m`、`30m`、`1h`、`4h`、`1d` |
| `startTime` | 否 | i64 | 开始时间，毫秒时间戳；后端可兼容 `start_time` |
| `endTime` | 否 | i64 | 结束时间，毫秒时间戳；后端可兼容 `end_time` |
| `limit` | 否 | i64 | 返回数量，默认 500，最大 1000 |

### 3.2 CSV 输入

支持标准字段：

```csv
open_time,open,high,low,close,volume
```

支持数据库字段别名：

```csv
open_time,open_price,high_price,low_price,close_price,base_volume
```

### 3.3 API 返回结构兼容

支持直接数组、`data/items/rows/list/klines` 包裹结构、对象式 K线和数组式 K线。

数组式 K线顺序：

```text
[open_time, open, high, low, close, volume]
```

---

## 4. 数据模型与数据质量

### 4.1 Kline 模型

```rust
pub struct Kline {
    pub open_time: i64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
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
所有价格和成交量字段均为有限数字
```

必须检查：

```text
是否存在重复 open_time
是否存在缺失 K线
是否存在乱序 K线
是否包含未闭合 K线
最新 K线延迟是否超过阈值
```

策略状态确认只允许使用已闭合 K线。未闭合 K线只能用于实时观察。

### 4.3 DataQuality

```rust
pub struct DataQuality {
    pub input_kline_count: usize,
    pub missing_kline_count: usize,
    pub duplicate_kline_count: usize,
    pub has_gap: bool,
    pub has_unclosed_kline: bool,
    pub latest_kline_delay_ms: i64,
    pub quality_score: f64,
}
```

`quality_score` 范围为 `0.0 ~ 1.0`。若存在严重缺口、延迟或未闭合 K线参与确认，必须降低置信度或进入 `wait` / `risk_control`。

---

## 5. 多周期设计

### 5.1 周期分工

| 周期 | 用途 |
|---|---|
| `4h` / `1h` | 大方向与系统性风险过滤 |
| `30m` / `15m` | 判断当前是否适合网格 |
| `5m` / `1m` | 网格触发、信号展示、成交模拟 |
| `1d` / `1w` | 长期趋势与极端风险背景 |

### 5.2 多周期输入结构

```rust
pub struct MultiTimeframeInput {
    pub higher: TimeframeAnalysis,
    pub middle: TimeframeAnalysis,
    pub lower: TimeframeAnalysis,
}
```

### 5.3 多周期合并决策矩阵

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
P0：大周期下跌风险 > 中周期下跌风险 > 小周期破位预警
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

### 7.1 MA / EMA

```text
MA20 用于中期方向
MA20 / MA60 粘合度用于判断震荡
MA20 斜率用于判断方向变化
EMA20 偏离率用于判断价格偏离均值的程度
```

```text
ma_spread = (max(MA20, MA60) - min(MA20, MA60)) / close
ma20_slope = (MA20_now - MA20_n_bars_ago) / MA20_n_bars_ago
ema20_deviation = (close - EMA20) / EMA20
```

### 7.2 BOLL 与 %B

默认：`period = 20`，`multiplier = 2.0`。

```text
mid = SMA(close, period)
std = standard_deviation(close, period)
upper = mid + multiplier * std
lower = mid - multiplier * std
bandwidth = (upper - lower) / abs(mid)
percent_b = (close - lower) / (upper - lower)
```

解释：

```text
percent_b > 0.8：接近 BOLL 上沿，偏强或上沿测试
percent_b < 0.2：接近 BOLL 下沿，偏弱或下沿测试
0.3 <= percent_b <= 0.7：区间中部，偏震荡
```

BOLL 带宽极低不等于适合网格，可能是变盘前夜；BOLL 上下轨可作为动态区间参考，但不能作为唯一止损依据。

### 7.3 MACD

默认：`fast = 12`、`slow = 26`、`signal = 9`。

```text
dif = EMA(close, fast) - EMA(close, slow)
dea = EMA(dif, signal)
hist = dif - dea
```

### 7.4 ATR

默认：`period = 14`。

```text
TR = max(
  high - low,
  abs(high - previous_close),
  abs(low - previous_close)
)
```

ATR 使用 Wilder 平滑，用于动态网格间距、波动风险识别、止损缓冲和趋势跟随网格范围。

### 7.5 ADX / DMI

默认：`period = 14`。

经验阈值：

```text
ADX < 20：趋势弱
20 <= ADX < 25：趋势开始增强
ADX >= 25：趋势较明显
```

实际阈值必须按 `source + symbol + interval` 回测校准。

### 7.6 RSI

默认：`period = 14`。

```text
RSI 40~60：偏中性，适合震荡判断
RSI > 70：短期超买
RSI < 30：短期超卖
```

RSI 不单独决定买卖，只作为位置和风险辅助。

### 7.7 Volume Ratio

默认：`volume_ma_period = 20`。

```text
volume_ratio = current_volume / SMA(volume, 20)
```

```text
volume_ratio >= 1.5：放量
0.7 <= volume_ratio <= 1.3：成交量平稳
volume_ratio < 0.7：缩量
```

### 7.8 Donchian Channel

默认：`N = 20`。

```text
donchian_high = highest(high, N)
donchian_low = lowest(low, N)
```

用途：

```text
close > donchian_high：向上突破候选
close < donchian_low：向下破位候选
突破后若 2~3 根 K线内回到通道内，视为假突破候选
```

Donchian 适合补充 BOLL。BOLL 更偏均值和波动，Donchian 更偏真实价格结构边界。

### 7.9 Keltner Channel（v1.1）

```text
keltner_mid = EMA(close, 20)
keltner_upper = keltner_mid + 2 * ATR(20)
keltner_lower = keltner_mid - 2 * ATR(20)
```

用途：

```text
BOLL 突破 + Keltner 同向突破：突破可信度更高
BOLL 突破但 Keltner 未突破：假突破风险更高
BOLL 带宽扩张 + Keltner 扩张：波动扩张确认
```

### 7.10 Price Structure

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
```

分位数解释：

```text
< 15%：极低
15% ~ 30%：偏低
30% ~ 70%：正常
70% ~ 85%：偏高
> 85%：极高
```

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

### 9.3 `range_score` 默认权重与评分函数

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

### 9.4 `up_score` 默认权重与评分函数

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

### 9.5 `down_score` 默认权重与评分函数

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

---

## 10. 评分平滑、评分动能与互斥修正

### 10.1 评分平滑

三评分必须同时保留原始分和平滑分。

```text
smoothed_score = EMA(raw_score, smooth_period)
```

默认：

```text
smooth_period = 3 或 5
```

输出建议：

```text
raw_scores
smoothed_scores
score_momentum
```

### 10.2 评分动能

```text
range_momentum = range_score_now - range_score_prev
up_momentum = up_score_now - up_score_prev
down_momentum = down_score_now - down_score_prev
```

示例：

```text
range_score 仍高，但 range_momentum 连续为负，且 up_momentum 快速上升：进入 up_break_warning candidate
range_score 仍高，但 range_momentum 连续为负，且 down_momentum 快速上升：进入 down_break_warning candidate
```

### 10.3 评分互斥修正

```text
if up_score >= 60 or down_score >= 60:
    range_score = range_score * 0.7

if up_score >= 70 and down_score >= 70:
    final_state = wait
    risk_level = advisory 或 soft_block
    reason += "多空评分冲突，进入等待"
```

---

## 11. 假突破过滤器

### 11.1 目标

减少以下错误：

```text
价格短暂突破区间上沿后迅速回落，导致过早关闭网格
价格短暂跌破区间下沿后迅速拉回，导致过早止损
缩量突破导致误判趋势启动
插针突破导致状态异常切换
```

### 11.2 向上假突破规则

进入 `up_break_warning` 后，若满足以下条件，则回到 `range_grid` 或 `wait`：

```text
价格在 fake_breakout_window 根 K线内重新回到 BOLL 上轨或 Donchian 上轨下方
volume_ratio < breakout_volume_confirm_threshold
ADX 未继续上升
up_score 回落到 warning_exit 以下
```

默认参数：

```text
fake_breakout_window = 2~3
breakout_volume_confirm_threshold = 1.3~1.5
warning_exit = 45
```

### 11.3 向下假突破规则

进入 `down_break_warning` 后，若满足以下条件，则不立即进入 `downtrend_risk`：

```text
价格在 fake_breakout_window 根 K线内重新回到 BOLL 下轨或 Donchian 下轨上方
volume_ratio 不支持放量破位
ADX 未继续上升
下跌结构未形成 lower_high + lower_low
```

### 11.4 真突破确认规则

突破确认至少需要满足两类以上证据：

```text
价格证据：收盘价有效突破 BOLL / Donchian / range 边界
波动证据：BOLL bandwidth 或 ATR percentile 上升
趋势证据：ADX 上升且 DMI 方向一致
成交量证据：volume_ratio >= 阈值
结构证据：higher_high/higher_low 或 lower_high/lower_low 成立
```

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

确认期和冷却期必须依赖状态上下文。

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
```

状态迁移输出：

```rust
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

---

## 13. 风控等级与动作

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

### 13.2 全局硬止损

无论状态机结果如何，必须配置全局硬止损：

```text
max_loss_per_symbol
max_daily_loss
max_drawdown
max_position_ratio
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
    pub risk_level: RiskLevel,
    pub risk_action: String,
    pub confidence: f64,
}
```

### 14.2 普通网格允许条件

即使状态为 `range_grid`，仍必须满足：

```text
数据质量合格
波动率未失控
流动性满足要求
单格利润覆盖手续费、滑点和最小利润缓冲
大周期无 downtrend_risk
中周期 range_grid 成立
状态不在冷却期
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
generated_at
is_closed_kline
data_quality
raw_scores
smoothed_scores
score_momentum
state_transition
```

### 16.2 confidence 定义

```text
base_confidence = 主状态评分 / 100
multi_tf_factor = 多周期一致性系数，范围 0.5 ~ 1.2
data_quality_factor = data_quality.quality_score
confidence = clamp(base_confidence * multi_tf_factor * data_quality_factor, 0.0, 1.0)
```

### 16.3 JSON 示例

```json
{
  "schema_version": "1.0",
  "model_version": "rule-v1",
  "config_version": "grid-analysis-v1",
  "source": "binance",
  "symbol": "BTCUSDT",
  "interval": "5m",
  "time": 1710000000000,
  "generated_at": 1710000060000,
  "is_closed_kline": true,
  "state": "range_grid",
  "raw_scores": {
    "range_score": 74,
    "up_score": 20,
    "down_score": 12
  },
  "smoothed_scores": {
    "range_score": 72,
    "up_score": 18,
    "down_score": 10
  },
  "score_momentum": {
    "range_momentum": 2,
    "up_momentum": -1,
    "down_momentum": 0
  },
  "confidence": 0.72,
  "data_quality": {
    "input_kline_count": 1000,
    "missing_kline_count": 0,
    "duplicate_kline_count": 0,
    "has_gap": false,
    "has_unclosed_kline": false,
    "latest_kline_delay_ms": 0,
    "quality_score": 1.0
  },
  "latest": {
    "close": 68050.0,
    "percent_b": 0.62,
    "ema20_deviation": 0.003,
    "donchian": {
      "high": 68650.0,
      "low": 67250.0
    }
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
    "risk_level": "advisory",
    "risk_action": "normal",
    "confidence": 0.72
  },
  "risk": {
    "risk_level": "advisory",
    "allow_new_grid": true,
    "allow_new_buy_orders": true,
    "allow_new_sell_orders": true,
    "position_action": "hold_or_normal_grid",
    "reasons": [
      "ADX 较低，趋势强度偏弱",
      "BOLL 带宽处于正常分位",
      "成交量平稳"
    ]
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

---

## 18. 数据库设计建议

### 18.1 analysis_market_states

```text
source
symbol
interval
open_time
schema_version
model_version
config_version
state
raw_scores jsonb
smoothed_scores jsonb
score_momentum jsonb
confidence
data_quality jsonb
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
risk_level
risk_action
confidence
model_version
config_version
created_at
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
    "percentile_window": 1000
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
    "max_capital_usage_at_lower_bound": 0.5
  },
  "risk": {
    "max_position_ratio": 0.3,
    "max_grid_capital": 1000,
    "max_loss_per_symbol": 0.03,
    "max_daily_loss": 0.05,
    "max_drawdown": 0.1
  }
}
```

---

## 20. 前端 TradingView 集成

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

---

## 21. 回测要求与上线验收标准

### 21.1 回测数据要求

```text
多个交易对：BTC、ETH、主流山寨
多个周期：5m、15m、30m、1h
多种行情：震荡、慢涨、急涨、慢跌、暴跌、插针、低波动蓄势
至少 6~12 个月历史数据
```

### 21.2 成本模型

```text
手续费
滑点
买卖价差
最小下单金额
成交延迟
部分成交
```

### 21.3 对照组

```text
A. 固定网格，不加指标过滤
B. BOLL 区间网格
C. BOLL + MACD + ATR + ADX 过滤网格
D. 生产版维度评分 + 多周期 + 状态机网格
E. 生产版 + Donchian/%B/评分平滑/假突破过滤器
```

### 21.4 核心评价指标

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
```

### 21.5 上线验收标准

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

## 22. 测试要求

### 22.1 单元测试

```text
CSV 标准字段解析
CSV 别名字段解析
API data 包裹解析
API 数组式 K线解析
时间字符串解析
非法 OHLC 报错
K线连续性检查
BOLL 计算
MACD 计算
ATR 计算
ADX 计算
RSI 计算
Volume Ratio 计算
Donchian Channel 计算
%B 计算
EMA20 偏离率计算
分位数计算
评分函数
评分平滑
评分动能
假突破过滤器
```

### 22.2 集成测试

```text
调用 K线 API 成功
API 返回空数据
API 返回非法数据
状态机确认期
状态机冷却期
多周期合并判断
输出 JSON contract 稳定性
```

### 22.3 回归测试

每次调整权重或阈值必须跑固定回测集，输出指标对比。

---

## 23. 可观测性与告警

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
```

---

## 24. 备选方案与扩展路线

### 24.1 机器学习分类器

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

### 24.2 波动率择时模型

更轻量的备选方案：

```text
不强行判断方向
只判断波动率是否适合网格
波动率正常：允许网格
波动率急剧扩张：暂停网格
波动率极低：等待变盘确认
```

可作为 MVP 或回测对照组。

### 24.3 趋势 / 周期分解

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

### 24.4 盘口微观结构

若后续可获取 Level2 数据，可增加：

```text
order_book_imbalance
spread
depth
large_order_flow
```

用于短周期提前预警，但不作为第一版依赖。

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

---

## 26. 版本演进计划

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
Keltner Channel
OBV / VWAP 偏离
TradingView marks
历史信号入库
WebSocket 推送
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

## 27. 关键结论

1. 本模块是行情状态识别与网格风控辅助系统，不是盈利保证系统。
2. 指标之间有重叠，必须按维度评分，避免重复计分。
3. 评分没有通用标准，默认权重只是初始经验值，必须用历史数据回测校准。
4. 高性价比增强项是 Donchian、`%B`、EMA20 偏离率、评分平滑、评分动能、假突破过滤器和全局硬止损。
5. production-ready 必须包含多周期、成交量、价格结构、成本约束、状态上下文、确认期、冷却期、风险等级、数据质量、假突破过滤和回测验收。
6. 网格策略的重点不是预测最低点，而是识别什么时候不能继续普通网格。
