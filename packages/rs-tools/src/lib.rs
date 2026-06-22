mod indicators;
mod types;

pub use types::*;

use indicators::{adx, atr, bollinger, macd, percentile_rank};

pub fn analyze_klines(
    klines: &[Kline],
    config: &AnalysisConfig,
    symbol: Option<String>,
    interval: Option<String>,
) -> AnalysisReport {
    let mut rows = klines.to_vec();
    rows.sort_by_key(|k| k.open_time);

    let boll = bollinger(&rows, config.boll_period, config.boll_mult);
    let macd_points = macd(&rows, config.macd_fast, config.macd_slow, config.macd_signal);
    let atr_points = atr(&rows, config.atr_period);
    let adx_points = adx(&rows, config.adx_period);

    let indicators = build_indicator_points(&rows, &boll, &macd_points, &atr_points, &adx_points);
    let market = classify_market(&rows, &boll, &macd_points, &adx_points);
    let grid_plan = build_grid_plan(&market, &rows, &boll, &atr_points, config.grid_count);
    let signals = build_signals(&rows, &boll, &macd_points, &market);

    AnalysisReport {
        symbol,
        interval,
        latest: indicators.last().cloned(),
        market,
        grid_plan,
        signals,
    }
}

fn build_indicator_points(
    klines: &[Kline],
    boll: &[Option<BollPoint>],
    macd: &[Option<MacdPoint>],
    atr: &[Option<f64>],
    adx: &[Option<AdxPoint>],
) -> Vec<IndicatorPoint> {
    klines
        .iter()
        .enumerate()
        .map(|(i, k)| IndicatorPoint {
            time: k.open_time,
            close: k.close,
            boll: boll.get(i).cloned().flatten(),
            macd: macd.get(i).cloned().flatten(),
            atr: atr.get(i).cloned().flatten(),
            adx: adx.get(i).cloned().flatten(),
        })
        .collect()
}

fn classify_market(
    klines: &[Kline],
    boll: &[Option<BollPoint>],
    macd: &[Option<MacdPoint>],
    adx: &[Option<AdxPoint>],
) -> MarketStateReport {
    if klines.len() < 40 {
        return MarketStateReport {
            state: MarketState::Wait,
            scores: MarketScores {
                range_score: 0,
                up_score: 0,
                down_score: 0,
            },
            reasons: vec!["K线数量不足，先等待更多样本".to_string()],
        };
    }

    let i = klines.len() - 1;
    let latest = &klines[i];
    let Some(last_boll) = boll[i].as_ref() else {
        return MarketStateReport {
            state: MarketState::Wait,
            scores: MarketScores {
                range_score: 0,
                up_score: 0,
                down_score: 0,
            },
            reasons: vec!["BOLL 尚未形成".to_string()],
        };
    };

    let prev_i = i.saturating_sub(3);
    let mid_slope = boll[prev_i]
        .as_ref()
        .map(|b| (last_boll.mid - b.mid) / b.mid.max(1e-12))
        .unwrap_or(0.0);
    let bandwidths: Vec<f64> = boll.iter().filter_map(|b| b.as_ref().map(|v| v.bandwidth)).collect();
    let bbw_rank = percentile_rank(&bandwidths, last_boll.bandwidth);

    let last_macd = macd[i].as_ref();
    let prev_macd = macd[i.saturating_sub(1)].as_ref();
    let last_adx = adx[i].as_ref();
    let prev_adx = adx[i.saturating_sub(1)].as_ref();

    let crosses_mid = count_mid_crosses(klines, boll, 24);
    let sign_flips = count_macd_hist_flips(macd, 24);
    let (higher_low, lower_high) = recent_structure(klines, 12);

    let mut range_score = 0u8;
    let mut up_score = 0u8;
    let mut down_score = 0u8;
    let mut reasons = Vec::new();

    if mid_slope.abs() < 0.002 {
        range_score += 20;
        reasons.push("BOLL 中轨接近走平".to_string());
    }
    if (0.15..=0.70).contains(&bbw_rank) {
        range_score += 20;
        reasons.push("BOLL 带宽处于非极端区间".to_string());
    }
    if crosses_mid >= 3 {
        range_score += 20;
        reasons.push("价格近期多次穿越中轨".to_string());
    }
    if sign_flips >= 2 {
        range_score += 15;
        reasons.push("MACD 柱近期反复正负切换".to_string());
    }
    if last_adx.map(|v| v.adx < 23.0).unwrap_or(false) {
        range_score += 25;
        reasons.push("ADX 较低，趋势强度偏弱".to_string());
    }

    if latest.close > last_boll.mid {
        up_score += 15;
    }
    if mid_slope > 0.0015 {
        up_score += 20;
        reasons.push("BOLL 中轨开始上行".to_string());
    }
    if bbw_rank > 0.55 && last_boll.bandwidth > bandwidths.iter().rev().skip(1).take(5).sum::<f64>() / 5.0_f64.max(1.0) {
        up_score += 10;
        down_score += 10;
        reasons.push("BOLL 带宽开始扩张，震荡可能失效".to_string());
    }
    if let (Some(m), Some(p)) = (last_macd, prev_macd) {
        if m.hist > p.hist {
            up_score += 15;
        }
        if m.hist < p.hist {
            down_score += 15;
        }
        if m.hist > 0.0 && m.dif > m.dea {
            up_score += 15;
            reasons.push("MACD 多头动能偏强".to_string());
        }
        if m.hist < 0.0 && m.dif < m.dea {
            down_score += 15;
            reasons.push("MACD 空头动能偏强".to_string());
        }
    }
    if let (Some(a), Some(pa)) = (last_adx, prev_adx) {
        if a.plus_di > a.minus_di && a.adx >= pa.adx {
            up_score += 20;
            reasons.push("+DI 大于 -DI，且 ADX 没有走弱".to_string());
        }
        if a.minus_di > a.plus_di && a.adx >= pa.adx {
            down_score += 20;
            reasons.push("-DI 大于 +DI，且 ADX 没有走弱".to_string());
        }
    }
    if higher_low {
        up_score += 15;
    }
    if lower_high {
        down_score += 15;
    }
    if latest.close > last_boll.upper {
        up_score += 15;
        reasons.push("收盘价突破 BOLL 上轨".to_string());
    }
    if latest.close < last_boll.lower {
        down_score += 15;
        reasons.push("收盘价跌破 BOLL 下轨".to_string());
    }

    let state = if range_score >= 65 && up_score < 55 && down_score < 55 {
        MarketState::RangeGrid
    } else if up_score >= 70 {
        MarketState::UptrendFollow
    } else if down_score >= 70 {
        MarketState::DowntrendRisk
    } else if up_score >= 55 {
        MarketState::UpBreakWarning
    } else if down_score >= 55 {
        MarketState::DownBreakWarning
    } else {
        MarketState::Wait
    };

    MarketStateReport {
        state,
        scores: MarketScores {
            range_score: range_score.min(100),
            up_score: up_score.min(100),
            down_score: down_score.min(100),
        },
        reasons,
    }
}

fn build_grid_plan(
    market: &MarketStateReport,
    klines: &[Kline],
    boll: &[Option<BollPoint>],
    atr: &[Option<f64>],
    grid_count: usize,
) -> GridPlan {
    let i = klines.len().saturating_sub(1);
    let latest_boll = boll.get(i).and_then(|v| v.as_ref());
    let latest_atr = atr.get(i).and_then(|v| *v);

    match market.state {
        MarketState::RangeGrid => {
            let (lower, upper, center) = latest_boll
                .map(|b| (Some(b.lower), Some(b.upper), Some(b.mid)))
                .unwrap_or((None, None, None));
            let step = match (lower, upper) {
                (Some(l), Some(u)) if grid_count > 0 => Some((u - l) / grid_count as f64),
                _ => None,
            };
            GridPlan {
                enabled: step.map(|s| s > 0.0).unwrap_or(false),
                mode: "range_grid".to_string(),
                lower,
                upper,
                center,
                grid_count,
                grid_step: step,
                risk_action: "normal".to_string(),
            }
        }
        MarketState::UptrendFollow | MarketState::UpBreakWarning => {
            let center = latest_boll.map(|b| b.mid).or_else(|| klines.last().map(|k| k.close));
            let atr = latest_atr.unwrap_or_else(|| klines.last().map(|k| k.close * 0.01).unwrap_or(0.0));
            let lower = center.map(|c| c - 1.5 * atr);
            let upper = center.map(|c| c + 2.5 * atr);
            GridPlan {
                enabled: true,
                mode: "uptrend_follow".to_string(),
                lower,
                upper,
                center,
                grid_count,
                grid_step: match (lower, upper) {
                    (Some(l), Some(u)) => Some((u - l) / grid_count.max(1) as f64),
                    _ => None,
                },
                risk_action: "reduce_sell_strength_and_move_grid_up_on_pullback".to_string(),
            }
        }
        MarketState::DowntrendRisk | MarketState::DownBreakWarning => GridPlan {
            enabled: false,
            mode: "risk_control".to_string(),
            lower: None,
            upper: None,
            center: latest_boll.map(|b| b.mid),
            grid_count,
            grid_step: None,
            risk_action: "pause_new_buy_orders".to_string(),
        },
        MarketState::Wait => GridPlan {
            enabled: false,
            mode: "wait".to_string(),
            lower: None,
            upper: None,
            center: latest_boll.map(|b| b.mid),
            grid_count,
            grid_step: None,
            risk_action: "wait_for_clearer_state".to_string(),
        },
    }
}

fn build_signals(
    klines: &[Kline],
    boll: &[Option<BollPoint>],
    macd: &[Option<MacdPoint>],
    market: &MarketStateReport,
) -> Vec<Signal> {
    let mut signals = Vec::new();
    if klines.len() < 2 {
        return signals;
    }

    let i = klines.len() - 1;
    let k = &klines[i];
    let Some(b) = boll[i].as_ref() else { return signals; };
    let m = macd[i].as_ref();
    let p = macd[i - 1].as_ref();

    if k.close <= b.lower * 1.005 && m.zip(p).map(|(m, p)| m.hist > p.hist).unwrap_or(false) {
        signals.push(Signal {
            time: k.open_time,
            price: k.close,
            signal_type: "grid_buy_watch".to_string(),
            strength: 0.7,
            text: "价格接近 BOLL 下轨，且 MACD 空头动能减弱".to_string(),
        });
    }

    if k.close >= b.upper * 0.995 && m.zip(p).map(|(m, p)| m.hist < p.hist).unwrap_or(false) {
        signals.push(Signal {
            time: k.open_time,
            price: k.close,
            signal_type: "grid_sell_watch".to_string(),
            strength: 0.7,
            text: "价格接近 BOLL 上轨，且 MACD 多头动能减弱".to_string(),
        });
    }

    match market.state {
        MarketState::UpBreakWarning | MarketState::UptrendFollow => signals.push(Signal {
            time: k.open_time,
            price: k.close,
            signal_type: "up_break_warning".to_string(),
            strength: market.scores.up_score as f64 / 100.0,
            text: "震荡可能向上失效，普通网格应减少卖出或准备上移".to_string(),
        }),
        MarketState::DownBreakWarning | MarketState::DowntrendRisk => signals.push(Signal {
            time: k.open_time,
            price: k.close,
            signal_type: "down_break_warning".to_string(),
            strength: market.scores.down_score as f64 / 100.0,
            text: "震荡可能向下失效，普通网格应暂停新增买单".to_string(),
        }),
        _ => {}
    }

    signals
}

fn count_mid_crosses(klines: &[Kline], boll: &[Option<BollPoint>], lookback: usize) -> usize {
    if klines.len() < 2 {
        return 0;
    }
    let start = klines.len().saturating_sub(lookback);
    let mut count = 0;
    let mut prev_side: Option<i8> = None;
    for i in start..klines.len() {
        if let Some(b) = boll[i].as_ref() {
            let side = if klines[i].close >= b.mid { 1 } else { -1 };
            if let Some(prev) = prev_side {
                if prev != side {
                    count += 1;
                }
            }
            prev_side = Some(side);
        }
    }
    count
}

fn count_macd_hist_flips(macd: &[Option<MacdPoint>], lookback: usize) -> usize {
    let start = macd.len().saturating_sub(lookback);
    let mut count = 0;
    let mut prev_side: Option<i8> = None;
    for item in macd.iter().skip(start).filter_map(|v| v.as_ref()) {
        let side = if item.hist >= 0.0 { 1 } else { -1 };
        if let Some(prev) = prev_side {
            if prev != side {
                count += 1;
            }
        }
        prev_side = Some(side);
    }
    count
}

fn recent_structure(klines: &[Kline], window: usize) -> (bool, bool) {
    if klines.len() < window * 2 {
        return (false, false);
    }
    let end = klines.len();
    let prev = &klines[end - window * 2..end - window];
    let curr = &klines[end - window..end];

    let prev_low = prev.iter().map(|k| k.low).fold(f64::INFINITY, f64::min);
    let curr_low = curr.iter().map(|k| k.low).fold(f64::INFINITY, f64::min);
    let prev_high = prev.iter().map(|k| k.high).fold(f64::NEG_INFINITY, f64::max);
    let curr_high = curr.iter().map(|k| k.high).fold(f64::NEG_INFINITY, f64::max);

    (curr_low > prev_low, curr_high < prev_high)
}
