use std::collections::{BTreeMap, HashMap, VecDeque};
use std::io::BufRead;

use serde::Deserialize;
use yalr::metrics::{FailureDetails, MetricsEvent, MetricsUser};
use yalr::metrics_agg::HealthReplay;
use yalr::providers::attribution::FailureAttribution;
use yalr::{HealthState, ProviderError};

#[derive(Deserialize)]
struct Row {
    t: i64,
    u: Option<i64>,
    s: String,
    p: Option<String>,
    a: Option<String>,
}

#[derive(Deserialize)]
struct Attempt {
    provider: String,
    upstream_status: Option<u16>,
    error: Option<String>,
}

struct Fault {
    ts: u64,
    user: Option<i64>,
    provider: String,
    error: ProviderError,
    legacy_counts: bool,
    attribution: FailureAttribution,
    class: String,
}

fn to_error(a: &Attempt) -> ProviderError {
    let text = a.error.clone().unwrap_or_default();
    if let Some(code) = a.upstream_status {
        let message = text.strip_prefix("Server error: ").unwrap_or(&text).to_string();
        return match code {
            429 => ProviderError::RateLimit { retry_after_ms: 30_000, message },
            401 | 403 => ProviderError::Authentication(message),
            _ => ProviderError::ServerError { message, status_code: Some(code) },
        };
    }
    if text.starts_with("Rate limit exceeded") {
        return ProviderError::RateLimit { retry_after_ms: 30_000, message: text };
    }
    if text.contains("timeout") || text.contains("Timeout") {
        return ProviderError::Timeout;
    }
    if let Some(rest) = text.strip_prefix("Server error: ") {
        let status = rest.rsplit("(status: Some(").next().and_then(|t| t.split(')').next()).and_then(|n| n.parse().ok());
        return ProviderError::ServerError { message: rest.to_string(), status_code: status };
    }
    ProviderError::Other(text.into())
}

fn caller(user: Option<i64>) -> Option<MetricsUser> {
    user.map(|id| MetricsUser { id: Some(id), name: None, api_key_id: None, api_key_name: None })
}

fn main() {
    let mut faults: Vec<Fault> = Vec::new();
    let mut successes: Vec<(u64, Option<i64>, String)> = Vec::new();
    let (mut requests, mut aborted) = (0u64, 0u64);
    for line in std::io::stdin().lock().lines() {
        let Ok(row) = serde_json::from_str::<Row>(&line.unwrap()) else { continue };
        requests += 1;
        let ts = row.t as u64;
        if row.s == "aborted" {
            aborted += 1;
            continue;
        }
        let attempts: Vec<Attempt> = row
            .a
            .as_deref()
            .and_then(|a| serde_json::from_str(a).ok())
            .unwrap_or_default();
        let failed: Vec<&Attempt> = attempts.iter().filter(|a| a.error.is_some()).collect();
        let ok = row.s == "ok";
        if ok {
            if let Some(p) = &row.p {
                successes.push((ts, row.u, p.clone()));
            }
        }
        let classes: Vec<String> = failed.iter().map(|a| to_error(a).failure_class()).collect();
        for (i, a) in failed.iter().enumerate() {
            let error = to_error(a);
            let mut attribution = error.attribution();
            if ok {
                attribution = FailureAttribution::Provider;
            } else if attribution == FailureAttribution::Unknown && failed.iter().enumerate().any(|(j, o)| o.provider != a.provider && classes[j] == classes[i]) {
                attribution = FailureAttribution::Request;
            }
            faults.push(Fault {
                ts,
                user: row.u,
                provider: a.provider.clone(),
                legacy_counts: !error.is_client_error(),
                class: classes[i].clone(),
                attribution,
                error,
            });
        }
    }
    faults.sort_by_key(|f| f.ts);
    successes.sort_by_key(|s| s.0);

    let first = faults.first().map_or(0, |f| f.ts).min(successes.first().map_or(u64::MAX, |s| s.0));
    let last = faults.last().map_or(0, |f| f.ts).max(successes.last().map_or(0, |s| s.0));
    let providers: Vec<String> = {
        let mut set: Vec<String> = faults.iter().map(|f| f.provider.clone()).collect();
        set.sort();
        set.dedup();
        set
    };

    let mut new_agg: HashMap<String, HealthReplay> = HashMap::new();
    let mut old_window: HashMap<String, VecDeque<u64>> = HashMap::new();
    let mut table: BTreeMap<String, [u32; 4]> = BTreeMap::new();
    let mut prev: HashMap<String, (bool, bool)> = HashMap::new();
    let (mut fi, mut si) = (0, 0);
    let mut now = first - first % 60_000;
    while now <= last + 60_000 {
        while fi < faults.len() && faults[fi].ts <= now {
            let f = &faults[fi];
            let details = FailureDetails {
                error_type: f.error.error_type(),
                error_message: f.error.to_string(),
                retry_after_ms: None,
                status_code: f.error.status_code(),
                attribution: f.attribution,
                connection: f.error.is_connection_failure(),
                ..Default::default()
            };
            new_agg
                .entry(f.provider.clone())
                .or_insert_with(|| HealthReplay::new(Default::default()))
                .record(&MetricsEvent::Failure(details), caller(f.user).as_ref(), f.ts);
            if f.legacy_counts {
                old_window.entry(f.provider.clone()).or_default().push_back(f.ts);
            }
            fi += 1;
        }
        while si < successes.len() && successes[si].0 <= now {
            let (ts, user, provider) = &successes[si];
            new_agg
                .entry(provider.clone())
                .or_insert_with(|| HealthReplay::new(Default::default()))
                .record(&MetricsEvent::Success, caller(*user).as_ref(), *ts);
            si += 1;
        }
        for p in &providers {
            let old_degraded = old_window.get_mut(p).is_some_and(|w| {
                while w.front().is_some_and(|t| *t + 300_000 < now) {
                    w.pop_front();
                }
                w.len() >= 2
            });
            let new_degraded = new_agg.get(p).is_some_and(|a| a.health_at(now) != HealthState::Healthy);
            let row = table.entry(p.clone()).or_default();
            let before = prev.get(p).copied().unwrap_or((false, false));
            if old_degraded && !before.0 {
                row[0] += 1;
            }
            if new_degraded && !before.1 {
                row[1] += 1;
            }
            row[2] += old_degraded as u32;
            row[3] += new_degraded as u32;
            prev.insert(p.clone(), (old_degraded, new_degraded));
        }
        now += 60_000;
    }

    println!("requests replayed: {requests} (aborted, skipped: {aborted})");
    println!("| provider | old episodes | new episodes | old minutes degraded | new minutes degraded |");
    println!("|---|---|---|---|---|");
    for (p, r) in &table {
        println!("| {p} | {} | {} | {} | {} |", r[0], r[1], r[2], r[3]);
    }

    let mut moved: HashMap<(String, String), u32> = HashMap::new();
    let mut totals = [0u32; 3];
    for f in faults.iter().filter(|f| f.legacy_counts) {
        match f.attribution {
            FailureAttribution::Request => {
                totals[0] += 1;
                let msg: String = f.error.to_string().chars().take(110).collect();
                *moved.entry((f.class.clone(), msg)).or_default() += 1;
            }
            FailureAttribution::Unknown => totals[1] += 1,
            FailureAttribution::Provider => totals[2] += 1,
        }
    }
    println!("\nfaults the old rule counted: reclassified request {} / unknown {} / provider {}", totals[0], totals[1], totals[2]);
    let mut top: Vec<_> = moved.into_iter().collect();
    top.sort_by(|a, b| b.1.cmp(&a.1));
    println!("\n| count | class | message |\n|---|---|---|");
    for ((class, msg), n) in top.into_iter().take(15) {
        println!("| {n} | {class} | {} |", msg.replace('|', "/"));
    }
}
