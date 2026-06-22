# K线指标分析与网格状态识别 Spec（Production Ready）

## 0. 文档信息

| 项目 | 内容 |
|---|---|
| 模块 | `packages/rs-tools` |
| 能力 | K线指标分析、行情状态识别、网格策略辅助、TradingView 展示数据输出 |
| 版本 | v1.0 production-ready spec |
| 适用阶段 | 分析服务、前端展示、回测验证、准实盘风控接入 |
| 不适用范围 | 自动下单、账户管理、保证盈利、替代回测 |

> 说明：本 Spec 的“production ready”指工程架构、接口契约、风控约束、测试与观测设计达到可生产落地标准；不表示策略一定盈利。所有评分阈值必须通过历史数据、手续费、滑点、盘口流动性回测后才能用于真实交易。

---

## 1. 背景与目标

网格交易的主要风险是：在震荡行情中表现较好，但在单边上涨中容易卖飞，在单边下跌中容易持续补仓导致套牢甚至爆仓。

因此，本模块的目标不是预测下一根 K线涨跌，而是把 K线数据转化为可解释的行情状态：

```text
震荡是否适合开网格
上涨突破风险是否上升
下跌破位风险是否上升
网格是否应该暂停、移动、减仓或进入等待
```

核心输出为：

```text
range_score：震荡/网格适配评分
up_score：上涨/向上突破评分
down_score：下跌/向下破位评分
state：最终行情状态
grid_plan：网格建议
signals：前端图表可展示的信号点
risk：风控动作和风险说明
```

---

## 2. 总体架构

### 2.1 逻辑链路

```text
K线 API / CSV / 数据库
        ↓
数据校验与时间排序
        ↓
多周期聚合与指标计算
        ↓
特征归一化与分位数计算
        ↓
维度评分：震荡、趋势、波动、动能、成交量、价格结构、交易成本
        ↓
状态机：确认期 + 冷却期 + 滞后阈值
        ↓
网格计划 / 风控动作 / TradingView 信号
        ↓
API 输出 / 入库 / 监控 / 回测
```

### 2.2 模块边界

本模块负责：

1. 获取或接收 K线数据。
2. 校验 K线完整性与合法性。
3. 计算指标和特征。
4. 生成行情评分。
5. 生成状态机结果。
6. 输出网格策略建议。
7. 输出前端展示数据。
8. 为回测和实盘风控提供稳定 JSON 契约。

本模块不负责：

1. 自动下单。
2. 管理交易所账户。
3. 管理 API Key。
4. 资金划转。
5. 预测确定性涨跌。
6. 绘制 TradingView 图表。
7. 未经回测直接实盘。

---

## 3. 数据输入

### 3.1 K线 API

接口：

```text
GET /api/v1/public/market/klines
```

请求参数：

| 参数 | 是否必填 | 类型 | 说明 |
|---|---:|---|---|
| `source` | 否 | string | 数据来源，不传时后端默认一般为 `binance` |
| `symbol` | 是 | string | 交易对，例如 `BTCUSDT` |
| `interval` | 是 | string | 周期，例如 `1m`、`5m`、`30m`、`1h`、`4h`、`1d` |
| `startTime` | 否 | i64 | 开始时间，毫秒时间戳；后端可兼容 `start_time` |
| `endTime` | 否 | i64 | 结束时间，毫秒时间戳；后端可兼容 `end_time` |
| `limit` | 否 | i64 | 返回数量，默认 500，最大 1000 |

示例：

```bash
cargo run --bin kline-analyze -- \
  --api-base-url http://127.0.0.1:3000 \
  --source binance \
  --symbol BTCUSDT \
  --interval 5m \
  --api-limit 1000
```

带时间范围：

```bash
cargo run --bin kline-analyze -- \
  --api-base-url http://127.0.0.1:3000 \
  --symbol BTCUSDT \
  --interval 5m \
  --start-time 1710000000000 \
  --end-time 1710086400000 \
  --api-limit 1000
```

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

直接数组：

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

包裹结构：

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

支持的包裹字段：

```text
data, items, rows, list, klines
```

数组式 K线：

```json
{
  "data": [
    [1710000000000, "68000", "68100", "67900", "68050", "123.4"]
  ]
}
```

数组顺序：

```text
[open_time, open, high, low, close, volume]
```

---

## 4. 数据模型与校验

### 4.1 Kline 模型

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

### 4.2 时间处理

`open_time` 支持：

1. 毫秒时间戳。
2. 秒时间戳。
3. 数字字符串。
4. RFC3339 时间字符串。

约束：

1. 数字时间戳保持原单位输出。
2. RFC3339 字符串转换为毫秒。
3. 内部排序统一按 `open_time ASC`。
4. 多周期聚合必须按 UTC 或交易所约定时间对齐。

### 4.3 数字处理

价格与成交量字段支持：

1. JSON number。
2. 数字字符串。

所有数值必须为有限数字，不能为 `NaN`、`Infinity` 或 `-Infinity`。

### 4.4 K线合法性校验

每根 K线必须满足：

```text
high >= low
high >= open
high >= close
low <= open
low <= close
volume >= 0
```

### 4.5 连续性校验

对策略分析，建议检查：

```text
相邻 open_time 是否按 interval 连续
是否有缺失 K线
是否存在重复 open_time
是否存在未闭合 K线
```

策略默认只使用已闭合 K线。未闭合 K线只能用于实时观察，不允许直接进入最终状态机确认。

---

## 5. 多周期设计

### 5.1 推荐周期分工

| 周期 | 用途 |
|---|---|
| `4h` / `1h` | 大方向与系统性风险过滤 |
| `30m` / `15m` | 判断当前是否适合网格 |
| `5m` / `1m` | 网格触发、前端细节展示、成交模拟 |
| `1d` / `1w` | 长期趋势与极端风险背景 |

### 5.2 多周期共振规则

普通多头网格建议满足：

```text
大周期不能是 downtrend_risk
中周期为 range_grid 或 wait 偏震荡
小周期没有 down_break_warning
```

强允许条件：

```text
4h/1h: range_grid 或 wait
30m/15m: range_grid
5m: 接近下轨或区间下沿，且无破位风险
```

禁止普通网格条件：

```text
1h 或 4h 为 downtrend_risk
30m 为 down_break_warning 且连续确认
ATR 分位数极高，波动失控
成交量异常放大且价格跌破关键区间
```

---

## 6. 指标体系

本模块采用“按维度评分”，而不是简单按指标累加。指标之间存在重叠，因此每个指标只能作为某个维度的证据之一，不能重复计算同一件事。

### 6.1 指标清单

| 指标 | 作用 | 是否第一版必需 |
|---|---|---:|
| BOLL | 区间、波动带、价格位置 | 是 |
| MACD | 动能方向与动能变化 | 是 |
| ATR | 真实波动率、网格间距 | 是 |
| ADX / DMI | 趋势强度、多空方向压力 | 是 |
| MA5 / MA20 / MA60 | 趋势方向、均线粘合、支撑压力 | 是 |
| RSI | 超买超卖、震荡区间位置 | 是 |
| Volume / Volume Ratio | 突破确认、假突破过滤 | 是 |
| Price Structure | 高低点结构、箱体边界 | 是 |
| Fee / Slippage | 网格收益空间判断 | 生产必需 |
| Liquidity / Spread | 实盘成交质量 | 准实盘必需 |

---

## 7. 指标计算规范

### 7.1 MA

默认参数：

```text
MA5, MA20, MA60
```

用途：

```text
MA20 作为中期方向
MA5/MA20/MA60 粘合度判断震荡
MA20 斜率判断方向变化
```

均线粘合度：

```text
ma_spread = (max(MA5, MA20, MA60) - min(MA5, MA20, MA60)) / close
```

### 7.2 BOLL

默认参数：

```text
period = 20
multiplier = 2.0
```

公式：

```text
mid = SMA(close, period)
std = standard_deviation(close, period)
upper = mid + multiplier * std
lower = mid - multiplier * std
bandwidth = (upper - lower) / abs(mid)
```

注意：

1. BOLL 带宽极低不一定适合网格，可能是变盘前夜。
2. BOLL 上下轨适合作为动态网格边界参考，但不能作为硬止损的唯一依据。

### 7.3 MACD

默认参数：

```text
fast = 12
slow = 26
signal = 9
```

公式：

```text
dif = EMA(close, fast) - EMA(close, slow)
dea = EMA(dif, signal)
hist = dif - dea
```

用途：

```text
hist > 0 且放大：多头动能增强
hist < 0 且负值扩大：空头动能增强
hist 从负值收敛：下跌动能减弱
hist 从正值收敛：上涨动能减弱
```

### 7.4 ATR

默认参数：

```text
period = 14
```

公式：

```text
TR = max(
  high - low,
  abs(high - previous_close),
  abs(low - previous_close)
)
```

ATR 使用 Wilder 平滑。

用途：

```text
动态网格间距
波动风险识别
止损缓冲
趋势跟随网格范围
```

### 7.5 ADX / DMI

默认参数：

```text
period = 14
```

输出：

```text
adx
plus_di
minus_di
```

解释：

```text
ADX 低：趋势弱，可能震荡
ADX 上升：趋势强度增强
+DI > -DI：上行方向压力更强
-DI > +DI：下行方向压力更强
```

经验阈值：

```text
ADX < 20：趋势弱
20 <= ADX < 25：趋势开始增强
ADX >= 25：趋势较明显
```

实际阈值必须按 symbol + interval 回测校准。

### 7.6 RSI

默认参数：

```text
period = 14
```

解释：

```text
RSI 40~60：偏中性，适合震荡判断
RSI > 70：短期超买
RSI < 30：短期超卖
```

RSI 不单独决定买卖，只用于位置和风险辅助。

### 7.7 Volume Ratio

默认参数：

```text
volume_ma_period = 20
```

公式：

```text
volume_ratio = current_volume / SMA(volume, 20)
```

解释：

```text
volume_ratio >= 1.5：放量
0.7 <= volume_ratio <= 1.3：成交量平稳
volume_ratio < 0.7：缩量
```

突破方向必须结合成交量确认：

```text
放量突破：可信度更高
缩量突破：假突破概率更高
放量下跌：风险更高
缩量阴跌：风险仍然存在，但确认度低于放量破位
```

### 7.8 Price Structure

应计算最近 swing high / swing low：

```text
higher_high
higher_low
lower_high
lower_low
range_high
range_low
```

结构解释：

```text
高点抬高 + 低点抬高：上涨结构
高点降低 + 低点降低：下跌结构
高低点无方向：震荡结构
```

---

## 8. 特征归一化与分位数

固定阈值只能作为第一版经验值。生产版本必须支持按 `symbol + interval` 的历史分位数。

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
recent_range_width
```

示例：

```text
bbw_percentile = 当前 BOLL 带宽在过去 1000 根中的分位数
atr_percentile = 当前 ATR/close 在过去 1000 根中的分位数
```

分位数解释：

```text
< 15%：极低
15% ~ 30%：偏低
30% ~ 70%：正常
70% ~ 85%：偏高
> 85%：极高
```

关键原则：

```text
极低波动不等于适合网格，可能是突破前夜
极高波动不等于机会大，可能是风险失控
中低但稳定的波动最适合普通网格
```

---

## 9. 评分系统设计

### 9.1 总体原则

评分使用维度模型：

```text
range_score = 震荡结构分 + 趋势弱分 + 波动适配分 + 成交平稳分 + 成本适配分
up_score = 方向上行分 + 动能增强分 + 趋势增强分 + 放量确认分 + 结构上行分
 down_score = 方向下行分 + 动能转弱分 + 趋势增强分 + 放量破位分 + 结构下行分
```

注意：

1. 同一类证据不能重复加分。
2. 评分阈值是默认配置，不是通用标准。
3. 生产环境必须支持配置化和回测优化。

### 9.2 `range_score` 默认权重

| 维度 | 权重 | 参考指标 | 满分条件示例 |
|---|---:|---|---|
| 趋势弱 | 25 | ADX | ADX < 20 且没有持续上升 |
| 波动适配 | 20 | BOLL 带宽分位数、ATR 分位数 | 波动处于 20%~70% 分位，且非急剧扩张 |
| 均线粘合 | 15 | MA5/MA20/MA60 | ma_spread <= 2% |
| 价格往返 | 15 | BOLL mid / MA20 穿越次数 | 最近 N 根多次穿越中轨 |
| RSI 中性 | 10 | RSI | RSI 在 40~60 |
| 成交平稳 | 10 | volume_ratio | 0.7~1.3 |
| 成本适配 | 5 | grid_step、fee、slippage | 单格利润覆盖手续费和滑点 |

默认判定：

```text
range_score >= 65
up_score < 55
down_score < 55
```

### 9.3 `up_score` 默认权重

| 维度 | 权重 | 参考指标 | 满分条件示例 |
|---|---:|---|---|
| 价格方向 | 20 | close、MA20、BOLL mid | close > MA20 且 MA20 斜率向上 |
| 动能增强 | 20 | MACD | 金叉或 hist > 0 且连续放大 |
| 趋势增强 | 20 | ADX / +DI / -DI | +DI > -DI 且 ADX 上升 |
| 突破确认 | 15 | BOLL upper | close 突破上轨或箱体上沿 |
| 成交量确认 | 15 | volume_ratio | 突破时 volume_ratio >= 1.5 |
| 价格结构 | 10 | swing high / low | higher_high + higher_low |

默认判定：

```text
55 <= up_score < 70：up_break_warning
up_score >= 70：uptrend_follow candidate
up_score >= 80 且连续确认：uptrend_follow confirmed
```

### 9.4 `down_score` 默认权重

| 维度 | 权重 | 参考指标 | 满分条件示例 |
|---|---:|---|---|
| 价格方向 | 20 | close、MA20、BOLL mid | close < MA20 且 MA20 斜率向下 |
| 动能转弱 | 20 | MACD | 死叉或 hist < 0 且负值扩大 |
| 趋势增强 | 20 | ADX / +DI / -DI | -DI > +DI 且 ADX 上升 |
| 破位确认 | 15 | BOLL lower / range_low | close 跌破下轨或箱体下沿 |
| 成交量确认 | 15 | volume_ratio | 下跌时 volume_ratio >= 1.5 |
| 价格结构 | 10 | swing high / low | lower_high + lower_low |

默认判定：

```text
55 <= down_score < 70：down_break_warning
 down_score >= 70：downtrend_risk candidate
 down_score >= 80 且连续确认：downtrend_risk confirmed
```

---

## 10. 状态机设计

### 10.1 状态枚举

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

### 10.2 状态定义

#### Wait

条件：

```text
range_score < 50
up_score < 50
down_score < 50
```

动作：

```text
不开启普通网格
保留观察
等待更明确状态
```

#### RangeGrid

条件：

```text
range_score >= 65
up_score < 55
down_score < 55
```

动作：

```text
允许普通震荡网格
使用 BOLL 或近期箱体作为网格范围
单格利润必须覆盖手续费和滑点
```

#### UpBreakWarning

条件：

```text
up_score >= 55
range_score 从高位下降
未满足 uptrend_follow 确认
```

动作：

```text
暂停新增强卖单
减少普通网格卖出强度
准备上移网格
不立即追高
```

#### UptrendFollow

候选条件：

```text
up_score >= 70
```

确认条件：

```text
up_score >= 80 且连续 confirm_bars 根 K线
或 up_score >= 70 且大周期同步为上行
```

动作：

```text
关闭普通震荡网格
允许上涨跟随网格或趋势跟随
只在回踩支撑时补仓
避免固定网格过早卖飞
```

#### DownBreakWarning

条件：

```text
down_score >= 55
range_score 从高位下降
未满足 downtrend_risk 确认
```

动作：

```text
暂停新增买单
取消更低位置补仓单
降低资金使用率
收紧止损
等待确认
```

#### DowntrendRisk

候选条件：

```text
down_score >= 70
```

确认条件：

```text
down_score >= 80 且连续 confirm_bars 根 K线
或 down_score >= 70 且大周期同步为下行风险
```

动作：

```text
关闭普通网格
禁止新开多头网格
执行减仓或止损策略
等待新震荡区形成
```

### 10.3 确认期与冷却期

默认配置：

```text
confirm_bars = 3
cooldown_bars_after_exit = 5
cooldown_bars_after_stop_loss = 20
```

目的：

```text
避免假突破导致状态来回切换
减少频繁启停造成的手续费和滑点损耗
避免刚止损后马上重新开仓
```

### 10.4 滞后阈值

进入状态和退出状态使用不同阈值。

示例：

```text
进入 range_grid：range_score >= 65
退出 range_grid：range_score < 55 或 down_score/up_score >= 60
```

这样避免评分在临界值附近反复抖动。

---

## 11. 网格计划设计

### 11.1 GridPlan 结构

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
    pub confidence: f64,
}
```

### 11.2 普通震荡网格

当 `state = range_grid`：

```text
center = boll.mid 或 range_center
lower = min(boll.lower, range_low)
upper = max(boll.upper, range_high)
grid_step = (upper - lower) / grid_count
enabled = true
```

必须满足成本约束：

```text
grid_step / center > 2 * fee_rate + expected_slippage_rate + min_profit_buffer
```

默认：

```text
min_profit_buffer = 0.001，也就是 0.1%
```

### 11.3 上涨跟随网格

当 `state = up_break_warning` 或 `uptrend_follow`：

```text
center = MA20 或 BOLL mid
lower = center - 1.5 * ATR
upper = center + 2.5 * ATR
mode = uptrend_follow
enabled = true 或按策略配置决定
```

限制：

```text
减少卖出密度
只在回踩时补仓
不在突破加速段频繁反向卖出
```

### 11.4 下跌风控模式

当 `state = down_break_warning`：

```text
enabled = false
mode = risk_control
risk_action = pause_new_buy_orders
```

当 `state = downtrend_risk`：

```text
enabled = false
mode = stop_or_reduce
risk_action = close_grid_and_reduce_exposure
```

---

## 12. 信号输出设计

### 12.1 Signal 结构

```rust
pub struct Signal {
    pub time: i64,
    pub price: f64,
    pub signal_type: String,
    pub strength: f64,
    pub text: String,
}
```

### 12.2 信号类型

| signal_type | 含义 | 前端展示 |
|---|---|---|
| `grid_buy_watch` | 接近下轨/区间下沿，且下跌动能减弱 | 下方观察标记 |
| `grid_sell_watch` | 接近上轨/区间上沿，且上涨动能减弱 | 上方观察标记 |
| `up_break_warning` | 向上突破风险 | 上方预警标记 |
| `down_break_warning` | 向下破位风险 | 下方预警标记 |
| `pause_grid` | 暂停普通网格 | 状态标记 |
| `resume_grid` | 恢复普通网格 | 状态标记 |
| `move_grid_up` | 建议网格上移 | 水平线移动提示 |
| `move_grid_down` | 建议网格下移 | 水平线移动提示 |
| `risk_reduce` | 降低仓位风险动作 | 风险标记 |

---

## 13. 输出 JSON 契约

```json
{
  "symbol": "BTCUSDT",
  "interval": "5m",
  "time": 1710000000000,
  "state": "range_grid",
  "scores": {
    "range_score": 72,
    "up_score": 18,
    "down_score": 10
  },
  "confidence": 0.72,
  "latest": {
    "close": 68050.0,
    "ma": {
      "ma5": 68030.0,
      "ma20": 67980.0,
      "ma60": 67820.0
    },
    "boll": {
      "mid": 67980.0,
      "upper": 68600.0,
      "lower": 67360.0,
      "bandwidth": 0.0182,
      "bandwidth_percentile": 0.42
    },
    "macd": {
      "dif": 120.5,
      "dea": 100.1,
      "hist": 20.4
    },
    "atr": {
      "value": 300.5,
      "atr_percentile": 0.48
    },
    "adx": {
      "adx": 18.2,
      "plus_di": 24.5,
      "minus_di": 19.3
    },
    "rsi": 52.0,
    "volume": {
      "volume": 123.4,
      "volume_ma20": 110.2,
      "volume_ratio": 1.12
    }
  },
  "grid_plan": {
    "enabled": true,
    "mode": "range_grid",
    "lower": 67360.0,
    "upper": 68600.0,
    "center": 67980.0,
    "grid_count": 20,
    "grid_step": 62.0,
    "risk_action": "normal",
    "confidence": 0.72
  },
  "risk": {
    "allow_new_grid": true,
    "allow_new_buy_orders": true,
    "allow_new_sell_orders": true,
    "position_action": "hold_or_normal_grid",
    "reason": [
      "ADX 较低，趋势强度偏弱",
      "BOLL 带宽处于正常分位",
      "成交量平稳"
    ]
  },
  "signals": []
}
```

---

## 14. 前端 TradingView 集成

### 14.1 第一阶段

TradingView 图上展示：

```text
K线
TradingView 内置 BOLL / MACD
rs-tools 输出的网格上限/下限/中轴
rs-tools 输出的信号点
右侧状态面板显示 state、scores、reasons、risk_action
```

字段映射：

| 输出字段 | 前端用途 |
|---|---|
| `grid_plan.lower` | 画水平线：网格下限 |
| `grid_plan.upper` | 画水平线：网格上限 |
| `grid_plan.center` | 画水平线：网格中轴 |
| `signals[]` | 画 marker |
| `state` | 状态标签 |
| `scores` | 评分条 |
| `risk.reason` | 解释说明 |

### 14.2 第二阶段

增加历史指标序列接口：

```text
GET /api/v1/analysis/indicator-series
```

增加 TradingView marks 接口：

```text
GET /api/v1/analysis/marks
```

支持通过 `getMarks` / `getTimescaleMarks` 加载历史信号。

---

## 15. 分析服务 API 设计

### 15.1 获取当前分析结果

```text
GET /api/v1/analysis/market-state
```

参数：

```text
source
symbol
interval
limit
```

返回：`AnalysisReport`。

### 15.2 获取网格计划

```text
GET /api/v1/analysis/grid-plan
```

返回：`GridPlan` + risk 信息。

### 15.3 获取历史信号

```text
GET /api/v1/analysis/signals
```

参数：

```text
source
symbol
interval
startTime
endTime
```

### 15.4 获取多周期分析

```text
GET /api/v1/analysis/multi-timeframe-state
```

返回：

```text
1h state
30m state
5m state
combined state
combined risk action
```

---

## 16. 数据库设计建议

### 16.1 analysis_market_states

用于保存每根闭合 K线后的状态结果。

关键字段：

```text
source
symbol
interval
open_time
state
range_score
up_score
down_score
confidence
reasons jsonb
created_at
```

唯一键：

```text
source + symbol + interval + open_time
```

### 16.2 analysis_signals

保存历史信号。

关键字段：

```text
source
symbol
interval
open_time
signal_type
price
strength
text
created_at
```

### 16.3 analysis_grid_plans

保存每次网格计划。

关键字段：

```text
source
symbol
interval
open_time
mode
enabled
lower
upper
center
grid_count
grid_step
risk_action
confidence
created_at
```

---

## 17. 配置化要求

所有阈值必须配置化，不能硬编码为不可调整逻辑。

配置示例：

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
    "volume_ma_period": 20
  },
  "state": {
    "range_enter": 65,
    "range_exit": 55,
    "warning_enter": 55,
    "trend_candidate": 70,
    "trend_confirm": 80,
    "confirm_bars": 3,
    "cooldown_bars_after_exit": 5,
    "cooldown_bars_after_stop_loss": 20
  },
  "grid": {
    "grid_count": 20,
    "min_profit_buffer": 0.001,
    "fee_rate": 0.001,
    "expected_slippage_rate": 0.0005
  }
}
```

---

## 18. 风控要求

### 18.1 必须有的风控动作

| 状态 | 风控动作 |
|---|---|
| `range_grid` | 允许普通网格，但必须限制资金使用率 |
| `up_break_warning` | 减少卖出强度，准备上移网格 |
| `uptrend_follow` | 关闭普通震荡网格，切换跟随模式 |
| `down_break_warning` | 暂停新增买单，取消下方补仓单 |
| `downtrend_risk` | 关闭普通网格，禁止新开多头网格，执行减仓/止损 |
| `wait` | 不自动开网格 |

### 18.2 仓位限制

生产环境必须配置：

```text
max_position_ratio
max_grid_capital
max_loss_per_symbol
max_daily_loss
max_drawdown
```

### 18.3 异常行情保护

以下情况进入强制保护：

```text
短时间跌幅超过阈值
ATR 分位数 > 95%
volume_ratio > 3 且价格破位
数据延迟超过阈值
K线缺失或乱序
交易所 API 异常
```

保护动作：

```text
暂停新开网格
取消未成交补仓单
前端展示风险状态
记录告警
```

---

## 19. 回测要求

上线前必须回测。

### 19.1 回测数据

至少覆盖：

```text
多个交易对：BTC、ETH、主流山寨
多个周期：5m、15m、30m、1h
多种行情：震荡、慢涨、急涨、慢跌、暴跌、插针、低波动蓄势
至少 6~12 个月历史数据
```

### 19.2 成本模型

必须包含：

```text
手续费
滑点
买卖价差
最小下单金额
成交延迟
部分成交
```

### 19.3 对照组

至少比较：

```text
A. 固定网格，不加指标过滤
B. BOLL 区间网格
C. BOLL + MACD + ATR + ADX 过滤网格
D. 生产版维度评分 + 多周期 + 状态机网格
```

### 19.4 核心评价指标

```text
总收益
年化收益
最大回撤
收益回撤比
夏普/索提诺，可选
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
```

---

## 20. 测试要求

### 20.1 单元测试

必须覆盖：

```text
CSV 标准字段解析
CSV 别名字段解析
API data 包裹解析
API 数组式 K线解析
时间字符串解析
非法 OHLC 报错
BOLL 计算
MACD 计算
ATR 计算
ADX 计算
RSI 计算
Volume Ratio 计算
分位数计算
```

### 20.2 集成测试

必须覆盖：

```text
调用 K线 API 成功
API 返回空数据
API 返回非法数据
状态机确认期
状态机冷却期
多周期合并判断
输出 JSON contract 稳定性
```

### 20.3 回归测试

每次调整权重或阈值必须跑固定回测集，输出指标对比。

---

## 21. 可观测性与告警

### 21.1 关键日志

```text
symbol
interval
open_time
state
scores
grid_plan
risk_action
reason
input_kline_count
missing_kline_count
calculation_latency_ms
```

### 21.2 指标监控

```text
分析延迟
API 请求失败率
K线缺失数量
状态切换频率
downtrend_risk 触发次数
网格暂停次数
异常波动触发次数
```

### 21.3 告警条件

```text
K线延迟超过 2 个周期
K线连续性缺失
分析任务失败
状态切换异常频繁
多个交易对同时 downtrend_risk
API 错误率超过阈值
```

---

## 22. 发布与灰度

### 22.1 阶段一：只读分析

```text
只计算指标和状态
只前端展示
不影响真实交易
```

### 22.2 阶段二：影子回测

```text
实时生成信号
不下单
记录如果按信号执行会怎样
与真实行情对比
```

### 22.3 阶段三：小资金灰度

```text
只允许少量交易对
限制资金比例
启用严格止损
人工确认重要状态切换
```

### 22.4 阶段四：生产策略接入

```text
接入自动风控
接入监控告警
定期回测校准
每次参数变更需要回归测试
```

---

## 23. 版本演进计划

### v1.0

```text
BOLL / MACD / ATR / ADX / MA / RSI / Volume
维度评分
单周期状态机
CLI 输出 JSON
TradingView 展示支持
```

### v1.1

```text
多周期共振
状态确认期和冷却期
历史信号入库
```

### v1.2

```text
分位数阈值
按 symbol + interval 独立参数
回测模块
```

### v2.0

```text
自动参数校准
机器学习辅助分类，可选
组合风控
准实盘策略引擎接入
```

---

## 24. 关键结论

1. 指标体系不是完备预测系统，但可以构成可解释、可回测、可生产化的行情状态识别系统。
2. BOLL、ATR、MACD、ADX、MA 等指标存在重叠，必须按维度评分，避免重复计分。
3. 评分没有通用标准，默认权重只是初始经验值，必须用历史数据回测校准。
4. 生产版必须加入多周期、成交量、价格结构、成本约束、状态确认期、冷却期和风控动作。
5. 网格策略的重点不是“找到最低点买入”，而是“识别什么时候不能继续普通网格”。
