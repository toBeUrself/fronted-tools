use crate::types::{AdxPoint, BollPoint, Kline, MacdPoint};

pub fn bollinger(klines: &[Kline], period: usize, mult: f64) -> Vec<Option<BollPoint>> {
    let mut out = vec![None; klines.len()];
    if period == 0 || klines.len() < period {
        return out;
    }

    for i in period - 1..klines.len() {
        let start = i + 1 - period;
        let closes = &klines[start..=i];
        let mid = closes.iter().map(|k| k.close).sum::<f64>() / period as f64;
        let variance = closes
            .iter()
            .map(|k| {
                let d = k.close - mid;
                d * d
            })
            .sum::<f64>()
            / period as f64;
        let sd = variance.sqrt();
        let upper = mid + mult * sd;
        let lower = mid - mult * sd;
        let bandwidth = if mid.abs() > f64::EPSILON {
            (upper - lower) / mid.abs()
        } else {
            0.0
        };
        out[i] = Some(BollPoint {
            mid,
            upper,
            lower,
            bandwidth,
        });
    }
    out
}

pub fn ema(values: &[f64], period: usize) -> Vec<Option<f64>> {
    let mut out = vec![None; values.len()];
    if period == 0 || values.is_empty() {
        return out;
    }

    let alpha = 2.0 / (period as f64 + 1.0);
    let mut prev = values[0];
    out[0] = Some(prev);

    for i in 1..values.len() {
        let next = alpha * values[i] + (1.0 - alpha) * prev;
        prev = next;
        out[i] = Some(next);
    }
    out
}

pub fn macd(klines: &[Kline], fast: usize, slow: usize, signal: usize) -> Vec<Option<MacdPoint>> {
    let closes: Vec<f64> = klines.iter().map(|k| k.close).collect();
    let fast_ema = ema(&closes, fast);
    let slow_ema = ema(&closes, slow);

    let dif_values: Vec<f64> = fast_ema
        .iter()
        .zip(slow_ema.iter())
        .map(|(f, s)| f.unwrap_or(0.0) - s.unwrap_or(0.0))
        .collect();
    let dea_values = ema(&dif_values, signal);

    dif_values
        .iter()
        .zip(dea_values.iter())
        .map(|(dif, dea)| {
            dea.map(|dea| MacdPoint {
                dif: *dif,
                dea,
                hist: *dif - dea,
            })
        })
        .collect()
}

pub fn atr(klines: &[Kline], period: usize) -> Vec<Option<f64>> {
    let mut out = vec![None; klines.len()];
    if period == 0 || klines.len() < period + 1 {
        return out;
    }

    let mut trs = Vec::with_capacity(klines.len());
    trs.push(klines[0].high - klines[0].low);
    for i in 1..klines.len() {
        let prev_close = klines[i - 1].close;
        let tr = (klines[i].high - klines[i].low)
            .max((klines[i].high - prev_close).abs())
            .max((klines[i].low - prev_close).abs());
        trs.push(tr);
    }

    let first = trs[1..=period].iter().sum::<f64>() / period as f64;
    out[period] = Some(first);
    let mut prev = first;

    for i in period + 1..klines.len() {
        let next = (prev * (period as f64 - 1.0) + trs[i]) / period as f64;
        out[i] = Some(next);
        prev = next;
    }
    out
}

pub fn adx(klines: &[Kline], period: usize) -> Vec<Option<AdxPoint>> {
    let mut out = vec![None; klines.len()];
    if period == 0 || klines.len() < period * 2 {
        return out;
    }

    let mut tr = vec![0.0; klines.len()];
    let mut plus_dm = vec![0.0; klines.len()];
    let mut minus_dm = vec![0.0; klines.len()];

    for i in 1..klines.len() {
        let up_move = klines[i].high - klines[i - 1].high;
        let down_move = klines[i - 1].low - klines[i].low;
        plus_dm[i] = if up_move > down_move && up_move > 0.0 { up_move } else { 0.0 };
        minus_dm[i] = if down_move > up_move && down_move > 0.0 { down_move } else { 0.0 };
        tr[i] = (klines[i].high - klines[i].low)
            .max((klines[i].high - klines[i - 1].close).abs())
            .max((klines[i].low - klines[i - 1].close).abs());
    }

    let mut tr_smooth = tr[1..=period].iter().sum::<f64>();
    let mut plus_smooth = plus_dm[1..=period].iter().sum::<f64>();
    let mut minus_smooth = minus_dm[1..=period].iter().sum::<f64>();

    let mut dx = vec![None; klines.len()];
    let mut di = vec![None; klines.len()];

    for i in period..klines.len() {
        if i > period {
            tr_smooth = tr_smooth - tr_smooth / period as f64 + tr[i];
            plus_smooth = plus_smooth - plus_smooth / period as f64 + plus_dm[i];
            minus_smooth = minus_smooth - minus_smooth / period as f64 + minus_dm[i];
        }

        if tr_smooth <= f64::EPSILON {
            continue;
        }
        let plus_di = 100.0 * plus_smooth / tr_smooth;
        let minus_di = 100.0 * minus_smooth / tr_smooth;
        di[i] = Some((plus_di, minus_di));

        let denom = plus_di + minus_di;
        if denom > f64::EPSILON {
            dx[i] = Some(100.0 * (plus_di - minus_di).abs() / denom);
        }
    }

    let first_adx_index = period * 2 - 1;
    let dx_window: Vec<f64> = dx[period..=first_adx_index]
        .iter()
        .filter_map(|v| *v)
        .collect();
    if dx_window.len() < period {
        return out;
    }

    let mut adx_value = dx_window.iter().sum::<f64>() / period as f64;
    if let Some((plus_di, minus_di)) = di[first_adx_index] {
        out[first_adx_index] = Some(AdxPoint {
            adx: adx_value,
            plus_di,
            minus_di,
        });
    }

    for i in first_adx_index + 1..klines.len() {
        if let Some(current_dx) = dx[i] {
            adx_value = (adx_value * (period as f64 - 1.0) + current_dx) / period as f64;
        }
        if let Some((plus_di, minus_di)) = di[i] {
            out[i] = Some(AdxPoint {
                adx: adx_value,
                plus_di,
                minus_di,
            });
        }
    }

    out
}

pub fn percentile_rank(values: &[f64], value: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let below_or_equal = values.iter().filter(|v| **v <= value).count();
    below_or_equal as f64 / values.len() as f64
}
