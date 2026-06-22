# Klines Tools 契约修正与边界澄清

本文档是 `services/klines-tools/SPEC.zh-CN.md` 和 `services/klines-tools/SPEC.IMPLEMENTATION-GUARDRAILS.zh-CN.md` 的契约修正文件。

目的：修复审查中发现的状态、风控、执行、JSON Schema、MVP 范围和数据输入之间的歧义，避免后续实现出现状态不一致、风控动作歧义、回测和实时不一致等问题。

优先级：

```text
本文档中的 P0 修正规则优先级高于主 Spec 和 Guardrails 中的冲突表述。
```

---

## 1. P0 修正：MACD 必须进入 Phase 1

### 1.1 问题

主 Spec 的指标体系和 `up_score` / `down_score` 默认评分依赖 MACD：

```text
up_score：MACD 金叉、hist > 0 且连续放大
down_score：MACD 死叉、hist < 0 且负值扩大
```

因此 Phase 1 若不实现 MACD，会造成评分维度缺失和评分不可比。

### 1.2 修正规则

MACD 必须进入 Phase 1 / MVP 单周期状态识别。

Guardrails 中 Phase 1 必须实现列表修正为：

```text
BOLL
MACD
ATR
ADX / DMI
MA20 / MA60
RSI
Volume Ratio
raw_scores
smoothed_scores
基础 range_score / up_score / down_score
六状态状态机
RiskLevel
RiskDecision
全局硬止损 override 接口
JSON 输出
```

### 1.3 测试要求

Golden tests 必须增加：

```text
MACD warmup 不足 -> MACD unavailable，降低 confidence
MACD unavailable -> up/down 动能维度 unavailable，不得用 0 静默代替
MACD 金叉 + 放量上破 -> up_break_warning / uptrend_follow candidate
MACD 死叉 + 放量下破 -> down_break_warning / downtrend_risk candidate
```

---

## 2. P0 修正：MarketState 与 RiskOverride 分离

### 2.1 问题

主 Spec 的状态迁移表中出现了 `risk_control`，但 `MarketState` 只定义了：

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

`risk_control` 不是市场状态，它更像风控覆盖层或网格执行模式。如果把它加入 `MarketState`，会混淆：

```text
市场状态：行情是否震荡、上涨、下跌
风控覆盖：账户/数据/人工/策略是否禁止动作
执行模式：网格是否可执行、用哪种模式执行
```

### 2.2 修正规则

不得把 `risk_control` 作为 `MarketState`。

新增 `RiskOverride`：

```rust
pub enum RiskOverride {
    None,
    GlobalHardStop,
    DataQualityBlock,
    IndicatorUnavailableBlock,
    ManualBlock,
    ExchangeConstraintBlock,
}
```

输出示例：

```json
{
  "state": "range_grid",
  "risk_override": "global_hard_stop",
  "risk_decision": {
    "risk_level": "emergency_stop",
    "allowed_grid_modes": [],
    "order_permission": "none",
    "position_action": "stop_loss",
    "reasons": ["全局硬止损触发"]
  }
}
```

### 2.3 状态迁移表修正

原表述：

```text
任意 -> risk_control：全局硬止损触发
```

修正为：

```text
任意 MarketState 保持其市场状态不变；
risk_override = GlobalHardStop；
risk_decision.risk_level = EmergencyStop；
risk_decision.allowed_grid_modes = []；
risk_decision.order_permission = None。
```

---

## 3. P0 修正：市场风险与账户风险分层

### 3.1 问题

`services/klines-tools` 明确不负责账户管理、资金划转和自动下单，但全局硬止损需要：

```text
账户权益
仓位数量
仓位名义价值
平均入场价
已实现盈亏
未实现盈亏
最大回撤
资金占用
订单数量
```

如果没有这些输入，模块不能独立判断 `emergency_stop`。

### 3.2 风险分层

必须拆成两层：

```text
MarketRiskDecision：由 K线、指标、状态机产生，只判断市场风险。
PortfolioRiskDecision：由账户、仓位、PnL、资金占用产生，判断止损、减仓、emergency_stop。
```

### 3.3 PortfolioRiskInput

若本模块需要输出账户级 `emergency_stop`，必须显式输入：

```rust
pub struct PortfolioRiskInput {
    pub account_equity: Decimal,
    pub symbol_position_qty: Decimal,
    pub symbol_position_notional: Decimal,
    pub avg_entry_price: Option<Decimal>,
    pub unrealized_pnl: Option<Decimal>,
    pub realized_pnl_today: Option<Decimal>,
    pub max_equity_drawdown: Option<f64>,
    pub grid_capital_used: Decimal,
    pub open_order_count: usize,
}
```

如果没有 `PortfolioRiskInput`，则：

```text
本模块只能输出 MarketRiskDecision；
PortfolioRiskDecision / emergency_stop 必须由外部账户风控层计算；
本模块可以消费或透传外部 risk_override，但不得凭 K线自行判断账户硬止损。
```

---

## 4. P0 修正：RiskDecision 必须无歧义

### 4.1 问题

`allow_new_grid: bool` 无法表达以下差异：

```text
不允许任何网格
允许普通震荡网格
只允许上涨跟随网格
只允许展示，不允许执行
```

### 4.2 修正规则

`RiskDecision` 不再使用单一 `allow_new_grid: bool` 作为核心契约，改为 `allowed_grid_modes`。

```rust
pub enum AllowedGridMode {
    RangeGrid,
    UptrendFollow,
}

pub enum OrderPermission {
    None,
    ReadOnly,
    NewOrdersAllowed,
    ReplaceOnly,
    ReduceOnly,
}

pub enum PositionAction {
    Hold,
    ReduceByRatio,
    StopLoss,
    CloseGridOnly,
    ManualReview,
}

pub enum MarketType {
    Spot,
    UsdMarginedFutures,
    CoinMarginedFutures,
}

pub struct RiskDecision {
    pub risk_level: RiskLevel,
    pub risk_override: RiskOverride,
    pub allowed_grid_modes: Vec<AllowedGridMode>,
    pub order_permission: OrderPermission,
    pub position_action: PositionAction,
    pub reduce_position_ratio: Option<Decimal>,
    pub reduce_reference: Option<String>, // total_position / grid_position / strategy_position
    pub require_manual_confirm: bool,
    pub action_ttl_ms: i64,
    pub expire_at: i64,
    pub reasons: Vec<String>,
}
```

### 4.3 spot / futures 语义

必须显式区分 `MarketType`：

```text
Spot：没有交易所 reduce-only order flag；ReduceOnly 只能解释为“不增加净多仓”。
Futures：可以使用交易所 reduce-only flag，但仍需区分 reduce_position_ratio。
```

---

## 5. P0 修正：多周期 Snapshot 必须带 StatePhase

### 5.1 问题

多周期矩阵使用 `downtrend_risk confirmed` 这种语义，但 `TimeframeSnapshotRef` 只包含 `state`，没有表达 confirmed / candidate / cooling down。

### 5.2 修正规则

新增：

```rust
pub enum StatePhase {
    Observing,
    Candidate,
    Confirmed,
    CoolingDown,
}

pub struct TimeframeSnapshotRef {
    pub source: String,
    pub symbol: String,
    pub interval: String,
    pub open_time: i64,
    pub close_time: i64,
    pub is_closed: bool,
    pub state: MarketState,
    pub state_phase: StatePhase,
    pub candidate_bars: usize,
    pub required_confirm_bars: usize,
    pub cooldown_remaining_bars: usize,
    pub raw_scores: Scores,
    pub smoothed_scores: Scores,
    pub confidence: f64,
}
```

多周期判断必须写成确定条件：

```text
higher.state == DowntrendRisk && higher.state_phase == Confirmed
middle.state == RangeGrid && middle.state_phase == Confirmed
lower.state == DownBreakWarning && lower.state_phase in [Candidate, Confirmed]
```

---

## 6. P0 修正：JSON Required 字段与示例一致

### 6.1 问题

主 Spec 要求输出包含：

```text
indicator_availability
score_breakdown
state_transition
```

但 JSON 示例没有完整展示这些字段。

### 6.2 修正规则

如果字段是 required，则 JSON 示例、Rust struct、serde 输出、DB schema、前端 contract 必须一致。

最小 production 示例必须包含：

```json
{
  "indicator_availability": {
    "ready": true,
    "min_required_bars": 150,
    "warmup_bars": 1000,
    "unavailable_fields": []
  },
  "score_breakdown": {
    "range": [],
    "up": [],
    "down": []
  },
  "state_transition": {
    "previous_state": "wait",
    "candidate_state": "range_grid",
    "final_state": "range_grid",
    "transition_type": "confirmed",
    "candidate_bars": 3,
    "cooldown_remaining_bars": 0,
    "reasons": ["range_score 连续满足进入条件"]
  }
}
```

如果某字段暂不输出，必须在 Schema 中标记为 optional，不能同时写“必须包含”。

---

## 7. P0 修正：Confidence 不允许被多周期放大

### 7.1 问题

当前定义允许 `multi_tf_factor` 大于 `1.0`，这可能导致低数据质量结果被多周期一致性放大。

### 7.2 修正规则

Confidence 只能被削弱，不应被多周期放大。

新增：

```rust
pub struct ConfidenceBreakdown {
    pub state_evidence: f64,
    pub data_quality: f64,
    pub indicator_availability: f64,
    pub timeframe_alignment: f64,
    pub state_stability: f64,
    pub final_confidence: f64,
}
```

推荐公式：

```text
final_confidence = min(
  state_evidence,
  data_quality,
  indicator_availability,
  timeframe_alignment
) * state_stability
```

约束：

```text
所有因子范围为 0.0 ~ 1.0。
state_stability 只能 <= 1.0。
多周期一致性只能提高 reasons 的说服力，不得把 confidence 放大到超过基础证据质量。
```

---

## 8. P1 修正：GridPlan 需要 GridLevel

### 8.1 问题

`price_levels: Vec<f64>` 无法表达每一层网格是否满足交易所约束，也无法解释不可执行原因。

### 8.2 修正规则

准实盘阶段必须使用：

```rust
pub enum OrderSide {
    Buy,
    Sell,
}

pub struct GridLevel {
    pub side: OrderSide,
    pub raw_price: Decimal,
    pub price: Decimal,
    pub raw_qty: Decimal,
    pub qty: Decimal,
    pub notional: Decimal,
    pub executable: bool,
    pub disabled_reason: Option<String>,
}

pub struct GridPlan {
    pub enabled: bool,
    pub mode: GridMode,
    pub levels: Vec<GridLevel>,
    pub total_required_capital: Decimal,
    pub executable_level_count: usize,
}
```

MVP 展示阶段可以只输出 `lower / upper / center / grid_step`，但不得把展示用 `price_levels` 当成可执行订单计划。

---

## 9. P1 修正：执行契约不得使用 f64 表示金额

### 9.1 分层规则

```text
指标计算层：可以用 f64。
评分层：可以用 f64。
执行契约层：price / qty / notional / fee / tick_size / step_size / min_notional 必须用 Decimal 或字符串。
```

JSON 建议：

```json
{
  "price": "67360.10",
  "qty": "0.002",
  "notional": "134.7202"
}
```

Rust 建议：

```rust
rust_decimal::Decimal
```

---

## 10. P1 修正：DataQuality 增加 gap 明细

建议扩展：

```rust
pub struct GapRange {
    pub expected_open_time: i64,
    pub next_seen_open_time: i64,
    pub missing_count: usize,
}

pub struct DataQuality {
    pub first_open_time: i64,
    pub last_open_time: i64,
    pub expected_interval_ms: i64,
    pub max_gap_bars: usize,
    pub missing_kline_ratio: f64,
    pub gap_ranges: Vec<GapRange>,
}
```

这样复盘时可以知道缺口在哪里，而不只是知道 `has_gap = true`。

---

## 11. P1 修正：Feature Flags 必须纳入 config_version

### 11.1 问题

增强规则必须 gated，但如果 feature flags 不进入 config_version，复盘时无法知道某次分析到底启用了哪些规则。

### 11.2 修正规则

配置必须包含：

```json
{
  "config_version": "grid-analysis-v1.0.3",
  "features": {
    "enable_multi_timeframe": false,
    "enable_donchian": true,
    "enable_percent_b": true,
    "enable_score_momentum": false,
    "enable_fake_breakout_filter": false
  },
  "indicator": {},
  "state": {},
  "grid": {},
  "risk": {}
}
```

输出必须包含：

```json
{
  "config_version": "grid-analysis-v1.0.3",
  "config_hash": "sha256:...",
  "enabled_features": ["donchian", "percent_b"]
}
```

---

## 12. P1 修正：历史数据同步必须单独设计

K线 API 单次 `limit` 最大 1000，但回测要求 6~12 个月数据。因此需要单独补充历史数据同步设计：

```text
分页拉取
断点续传
rate limit
数据补洞
raw_klines 原始表
数据版本
重算 analysis 时的 source data snapshot
```

否则回测和实时服务容易使用不同数据语义。

---

## 13. 修订后的实施优先级

### Phase 0：数据和指标正确性

```text
K线解析
OHLCV 校验
排序、去重、缺失检查
闭合 K线识别
warmup / unavailable
BOLL / MACD / ATR / ADX / MA / RSI / Volume
DataQuality
IndicatorAvailability
```

### Phase 1：单周期 MVP 状态机

```text
raw_scores
smoothed_scores
range_score / up_score / down_score
六状态状态机
StatePhase
RiskOverride
RiskDecision
ConfidenceBreakdown
JSON contract
全局硬止损 override 接口
Golden tests
```

### Phase 2：状态稳定性增强

```text
Donchian
%B
EMA20 deviation
score_momentum
score_conflict_adjustment
fake_breakout_filter
pin-bar / wick filter
state transition 持久化
```

### Phase 3：准实盘能力

```text
多周期时间对齐
多周期合并
PortfolioRiskInput
ExchangeConstraints
Decimal / tick-level rounding
GridLevel price + qty + notional
conservative backtest
config_version + feature flags
shadow run
```

---

## 14. 最终边界定义

修订后，四层概念必须保持清晰：

```text
MarketState：只表达行情状态。
RiskOverride：表达账户、数据、人工、交易所约束等覆盖层。
RiskDecision：表达执行层可以消费的动作契约。
GridPlan：表达网格计划；展示阶段和执行阶段必须分清。
```

如果后续设计或实现出现以下混用，应视为契约错误：

```text
把 emergency_stop 当成 MarketState。
用 allow_new_grid: true 表达“只允许趋势跟随网格”。
没有 PortfolioRiskInput 却自行判断账户硬止损。
用 f64 作为执行订单价格/数量契约。
required 字段不出现在 JSON 示例中。
多周期矩阵使用 confirmed，但 snapshot 没有 state_phase。
```
