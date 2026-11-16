use serde::Serialize;

use crate::db::{milli_per_unit, ModelPricingRow, RequestSample, SATS};

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct Price {
    pub currency: String,
    pub input_per_1m: f64,
    pub output_per_1m: f64,
}

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct ModelPriceRange {
    pub currency: String,
    pub min_input: f64,
    pub max_input: f64,
    pub min_output: f64,
    pub max_output: f64,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Latency {
    pub requests: i64,
    pub error_rate: Option<f64>,
    pub ttft_p50_ms: Option<f64>,
    pub ttft_p95_ms: Option<f64>,
    pub tokens_per_second_p50: Option<f64>,
}

fn round2(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

pub fn price_of(row: &ModelPricingRow) -> Option<Price> {
    if row.is_free {
        return Some(Price { currency: row.currency.clone(), input_per_1m: 0.0, output_per_1m: 0.0 });
    }
    let (input, output) = if row.currency == SATS {
        (row.price_per_1m_input_sats, row.price_per_1m_output_sats)
    } else {
        (row.price_per_1m_input_milli, row.price_per_1m_output_milli)
    };
    let scale = if row.currency == SATS { 1.0 } else { milli_per_unit(&row.currency) as f64 };
    if input.is_none() && output.is_none() {
        return None;
    }
    Some(Price {
        currency: row.currency.clone(),
        input_per_1m: input.unwrap_or(0) as f64 / scale,
        output_per_1m: output.unwrap_or(0) as f64 / scale,
    })
}

pub fn price_ranges<'a>(prices: impl IntoIterator<Item = &'a Price>) -> Vec<ModelPriceRange> {
    let mut ranges: Vec<ModelPriceRange> = Vec::new();
    for p in prices {
        match ranges.iter_mut().find(|r| r.currency == p.currency) {
            Some(r) => {
                r.min_input = r.min_input.min(p.input_per_1m);
                r.max_input = r.max_input.max(p.input_per_1m);
                r.min_output = r.min_output.min(p.output_per_1m);
                r.max_output = r.max_output.max(p.output_per_1m);
            }
            None => ranges.push(ModelPriceRange {
                currency: p.currency.clone(),
                min_input: p.input_per_1m,
                max_input: p.input_per_1m,
                min_output: p.output_per_1m,
                max_output: p.output_per_1m,
            }),
        }
    }
    ranges.sort_by(|a, b| a.currency.cmp(&b.currency));
    ranges
}

pub fn uptime_percent(checks: i64, healthy: i64) -> Option<f64> {
    (checks > 0).then(|| round2(healthy as f64 * 100.0 / checks as f64))
}

pub fn percentile(sorted: &[f64], p: f64) -> Option<f64> {
    if sorted.is_empty() {
        return None;
    }
    let rank = (p * sorted.len() as f64).ceil() as usize;
    Some(sorted[rank.saturating_sub(1).min(sorted.len() - 1)])
}

fn sorted(mut values: Vec<f64>) -> Vec<f64> {
    values.sort_by(|a, b| a.total_cmp(b));
    values
}

pub fn latency(samples: &[&RequestSample]) -> Latency {
    let ok: Vec<&&RequestSample> = samples.iter().filter(|s| s.status == "ok").collect();
    let ttft = sorted(ok.iter().filter_map(|s| s.first_event_ms.map(|v| v as f64)).collect());
    let tps = sorted(
        ok.iter()
            .filter_map(|s| {
                let decode_ms = s.total_ms? - s.first_event_ms.unwrap_or(0);
                let tokens = s.completion_tokens.filter(|t| *t > 0)?;
                (decode_ms > 0).then(|| tokens as f64 * 1000.0 / decode_ms as f64)
            })
            .collect(),
    );
    let requests = samples.len() as i64;
    let failures = samples.iter().filter(|s| s.status != "ok").count() as f64;
    Latency {
        requests,
        error_rate: (requests > 0).then(|| round2(failures * 100.0 / requests as f64)),
        ttft_p50_ms: percentile(&ttft, 0.5),
        ttft_p95_ms: percentile(&ttft, 0.95),
        tokens_per_second_p50: percentile(&tps, 0.5).map(round2),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn price(currency: &str, input: f64, output: f64) -> Price {
        Price { currency: currency.into(), input_per_1m: input, output_per_1m: output }
    }

    fn sample(status: &str, ttft: Option<i64>, total: Option<i64>, tokens: Option<i64>) -> RequestSample {
        RequestSample {
            model: "m".into(),
            upstream_model: None,
            provider: None,
            status: status.into(),
            started_at_ms: 0,
            first_event_ms: ttft,
            total_ms: total,
            completion_tokens: tokens,
        }
    }

    #[test]
    fn price_ranges_keep_each_currency_apart() {
        let prices = [price("usd", 3.0, 9.0), price("sats", 500.0, 900.0), price("usd", 1.0, 12.0), price("sats", 200.0, 1000.0)];
        let ranges = price_ranges(&prices);
        assert_eq!(
            ranges,
            vec![
                ModelPriceRange { currency: "sats".into(), min_input: 200.0, max_input: 500.0, min_output: 900.0, max_output: 1000.0 },
                ModelPriceRange { currency: "usd".into(), min_input: 1.0, max_input: 3.0, min_output: 9.0, max_output: 12.0 },
            ]
        );
    }

    #[test]
    fn percentiles_use_nearest_rank() {
        let values = [10.0, 20.0, 30.0, 40.0];
        assert_eq!(percentile(&values, 0.5), Some(20.0));
        assert_eq!(percentile(&values, 0.95), Some(40.0));
        assert_eq!(percentile(&[], 0.5), None);
    }

    #[test]
    fn uptime_is_a_rounded_percentage_and_absent_without_checks() {
        assert_eq!(uptime_percent(3, 2), Some(66.67));
        assert_eq!(uptime_percent(0, 0), None);
    }

    #[test]
    fn latency_ignores_failures_for_speed_and_counts_them_for_errors() {
        let samples = [
            sample("ok", Some(100), Some(1100), Some(100)),
            sample("ok", Some(200), Some(2200), Some(400)),
            sample("ok", Some(300), Some(1300), Some(50)),
            sample("error", Some(9000), Some(9000), None),
            sample("ok", None, None, None),
        ];
        let refs: Vec<&RequestSample> = samples.iter().collect();
        let l = latency(&refs);
        assert_eq!(l.requests, 5);
        assert_eq!(l.error_rate, Some(20.0));
        assert_eq!(l.ttft_p50_ms, Some(200.0));
        assert_eq!(l.ttft_p95_ms, Some(300.0));
        assert_eq!(l.tokens_per_second_p50, Some(100.0));
    }
}
