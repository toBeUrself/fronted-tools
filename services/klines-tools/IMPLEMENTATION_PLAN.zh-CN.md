# Klines Tools 实施计划与正确性护栏

本文档是 `services/klines-tools/SPEC.zh-CN.md` 的实施计划。主 Spec 是完整的 production-ready 设计；本文件约束实际研发如何分阶段落地，避免一次性引入过多规则导致正确性下降、过拟合、状态机冲突或回测污染。

---

## 1. 核心原则

```text
文档可以完整；实现必须分层。
规则可以存在；生产必须 gated。
指标可以扩展；上线必须消融回测。
状态可以复杂；迁移必须 golden test 覆盖。
风控可以保守；收益优化不能优先于灾难风险控制。
```

一句话：

```text
先做正确性，再做复杂度。
```

---

## 2. 为什么不能一次性全部实现

一次性实现全部规则会放大以下风险：

```text
1. 指标之间重复表达同一类信息，导致重复计分。
2. 规则之间互相覆盖，导致状态机路径不稳定。
3. 多周期引用未闭合高周期 K线，造成未来函数。
4. 假突破过滤、评分平滑、状态冷却叠加后，调试困难。
5. 参数数量过多，回测更容易过拟合。
6. 规则越多，越容易出现“看起来更科学，但样本外更差”。
```

因此必须使用模块开关、阶段性验收和消融回测。

---

## 3. 实施分层

### 3.1 Phase 0：数据和指标正确性

目标：先保证输入数据、时间语义和核心指标正确，不做复杂策略。

必须实现：

```text
K线解析
OHLCV 校验
排序、去重、缺失检查
闭合 K线识别
warmup / unavailable
NaN / Inf 拒绝或 unavailable
BOLL
MACD
ATR
ADX / DMI
MA20 / MA60
RSI
Volume Ratio
DataQuality
IndicatorAvailability
```

禁止实现：

```text
复杂状态机
多周期合并
假突破过滤器
复杂网格计划
交易所订单级约束
```

验收标准：

```text
非法数据不进入评分
未闭合 K线不推进任何状态
NaN / Inf 不进入评分
warmup 不足只输出 wait
MACD unavailable 不得用 0 静默代替
```

---

### 3.2 Phase 1：单周期 MVP 状态机

目标：跑通单周期核心闭环。

必须实现：

```text
raw_scores
smoothed_scores
range_score / up_score / down_score
六状态状态机
StatePhase
RiskOverride
RiskLevel
RiskDecision
ConfidenceBreakdown
JSON contract
全局硬止损 override 接口
Golden tests
```

暂不实现：

```text
多周期合并
Donchian
%B
EMA20 偏离率
评分动能
假突破过滤器
交易所 GridLevel
机器学习
盘口结构
```

验收标准：

```text
状态机能稳定输出 wait / range_grid / warning / risk 状态
状态切换原因可解释
RiskOverride 与 MarketState 不混用
全局硬止损只覆盖 risk_decision，不错误改写市场状态语义
JSON required 字段与示例一致
固定 golden cases 全部通过
```

---

### 3.3 Phase 2：状态稳定性增强

目标：降低假突破和状态抖动。

新增：

```text
Donchian Channel
%B
EMA20 偏离率
score_momentum
score_conflict_adjustment
fake_breakout_filter
pin-bar / wick filter
state_transition 持久化
```

必须通过消融回测：

```text
MVP baseline
MVP + Donchian
MVP + %B
MVP + EMA20 deviation
MVP + score_momentum
MVP + fake_breakout_filter
MVP + all Phase 2 features
```

进入下一阶段条件：

```text
误杀次数下降
状态切换频率下降或不显著上升
最大回撤下降或不显著上升
样本外不变差
```

---

### 3.4 Phase 3：准实盘能力

目标：进入准实盘设计。

新增：

```text
多周期时间对齐
多周期合并决策矩阵
PortfolioRiskInput
ExchangeConstraints
Decimal / tick-level rounding
GridLevel price + qty + notional
conservative 回测成交模型
config_version + config_hash + enabled_features
analysis_config_versions
shadow run
```

验收标准：

```text
不得引用未闭合高周期 K线
回测与实时使用同一套时间对齐规则
所有 price / qty 都符合交易所约束
执行契约 price / qty / notional 使用 Decimal 或字符串
保守成交模型下仍满足上线验收
```

---

### 3.5 Phase 4：后置增强

这些功能默认不进入 MVP，也不进入第一版 production：

```text
Keltner Channel
OBV
VWAP 偏离
TradingView marks
历史信号入库
WebSocket 推送
参数自动校准
机器学习分类器
Kalman / HP 滤波
盘口微观结构
```

这些功能必须满足：

```text
有明确使用场景
有消融回测收益
不降低解释性
不增加无法调试的状态冲突
```

---

## 4. Feature Flag 要求

所有增强规则必须有 feature flag，并且 feature flags 必须进入 `config_version` 与 `config_hash`。

示例：

```json
{
  "config_version": "grid-analysis-v1.0.3",
  "features": {
    "enable_multi_timeframe": false,
    "enable_donchian": false,
    "enable_percent_b": false,
    "enable_ema20_deviation": false,
    "enable_score_momentum": false,
    "enable_score_conflict_adjustment": false,
    "enable_fake_breakout_filter": false,
    "enable_exchange_constraints": false,
    "enable_keltner": false,
    "enable_obv": false,
    "enable_vwap_deviation": false,
    "enable_ml_classifier": false,
    "enable_orderbook_features": false
  }
}
```

生产默认：

```text
只开启经过回测、样本外验证、shadow、gray 验证的 feature。
```

---

## 5. 消融回测 Gate

每个新增规则进入 production 前，必须做消融回测。

### 5.1 对照组

```text
A. 固定网格，不加过滤
B. MVP 单周期状态机
C. MVP + 单个新增规则
D. MVP + 同阶段全部新增规则
E. 当前 production 参数
```

### 5.2 判断指标

```text
最大回撤
收益回撤比
状态切换次数
误杀次数：退出后快速反弹
漏判次数：未退出后继续大跌
假突破过滤命中率
手续费占毛收益比例
灾难性连续补仓次数
样本外表现
```

### 5.3 进入生产条件

新增规则必须至少满足一项核心收益，且不能明显恶化其他关键指标：

```text
最大回撤降低
误杀或漏判降低
状态切换频率降低
灾难性补仓降低
样本外表现稳定
```

如果新增规则只提升收益但显著提高最大回撤，不得进入 production。

---

## 6. Golden Tests 要求

状态机必须维护固定 golden cases。

至少包括：

```text
标准震荡 -> range_grid
震荡中上沿测试但缩量 -> up_break_warning 后回归 range_grid / wait
放量上破 -> uptrend_follow candidate / confirmed
下沿插针后收回 -> warning signal，但不 confirmed downtrend_risk
放量下破 -> down_break_warning / downtrend_risk
数据缺失 -> wait + data_quality issue
未闭合 K线 -> realtime only，不推进状态机
高周期未闭合 -> lower timeframe 不得引用
全局硬止损 -> risk_override = global_hard_stop，不改变 MarketState 语义
up_score 与 down_score 同时高 -> wait 或 soft_block
MACD warmup 不足 -> MACD unavailable，降低 confidence
```

每次改状态机、阈值、评分函数或 risk_decision 契约必须跑 golden tests。

---

## 7. 正确性红线

以下规则不得被任何收益优化覆盖：

```text
1. 未闭合 K线不得推进 confirmed 状态。
2. 多周期不得引用未来高周期 K线。
3. warmup 不满足不得输出 confirmed 状态。
4. NaN / Inf 不得进入评分。
5. 全局硬止损不得被建模为 MarketState，只能作为 RiskOverride / RiskDecision 覆盖层。
6. risk_decision 必须可执行、无歧义。
7. 互斥修正、数据质量降级、指标 unavailable 必须写入 reasons。
8. 回测必须明确成交路径假设。
9. 参数变更必须有 config_version / config_hash / enabled_features。
10. 新规则必须先通过消融回测，再进入 production。
11. 执行契约中的 price / qty / notional 不得使用 f64。
12. required JSON 字段必须出现在示例、Rust struct、DB schema 中。
```

---

## 8. 推荐默认实施路线

```text
第 1 步：数据质量 + 核心指标
第 2 步：单周期 raw_scores / smoothed_scores
第 3 步：基础六状态机 + StatePhase + RiskDecision
第 4 步：RiskOverride + ConfidenceBreakdown + JSON contract
第 5 步：Golden tests + MVP 回测
第 6 步：Donchian / %B / EMA20 偏离率
第 7 步：评分动能 / 互斥修正 / 假突破过滤
第 8 步：多周期时间对齐和合并
第 9 步：PortfolioRiskInput / 交易所约束 / conservative 回测
第 10 步：shadow -> gray -> production
```

---

## 9. 最终建议

研发执行时按以下原则推进：

```text
先做正确性，再做复杂度。
先做单周期，再做多周期。
先做风控，再做收益优化。
先做可解释规则，再做高级模型。
先做消融验证，再进 production。
```

如果某个增强项不能在样本外降低回撤、降低误判或提升稳定性，就保持关闭。
