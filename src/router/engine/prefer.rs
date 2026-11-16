use super::*;
use crate::api::public_models_math::{latency, price_of};
use crate::router::preference::{reorder, resolve, Preference, Stat, MIN_SPEED_SAMPLES};

const SPEED_TTL: Duration = Duration::from_secs(60);
const SPEED_WINDOW_MS: i64 = 24 * 3_600_000;
const SAMPLE_LIMIT: i64 = 50_000;

#[derive(Default)]
pub(crate) struct SpeedBoard {
    refreshed: Option<Instant>,
    by_route: HashMap<(String, String), (Option<f64>, Option<f64>)>,
}

impl SpeedBoard {
    fn stale(&self) -> bool {
        self.refreshed.map_or(true, |t| t.elapsed() > SPEED_TTL)
    }
}

impl Router {
    async fn refresh_speeds(&self) {
        if !self.speeds.read().await.stale() {
            return;
        }
        let now = chrono::Utc::now().timestamp_millis();
        let samples = self.db.request_samples_since(now - SPEED_WINDOW_MS, SAMPLE_LIMIT).await.unwrap_or_default();
        let mut grouped: HashMap<(String, String), Vec<&crate::db::RequestSample>> = HashMap::new();
        for s in &samples {
            let Some(provider) = s.provider.clone() else { continue };
            grouped.entry((provider, s.model.to_lowercase())).or_default().push(s);
        }
        let by_route = grouped
            .into_iter()
            .filter(|(_, rows)| rows.len() as i64 >= MIN_SPEED_SAMPLES)
            .map(|(key, rows)| {
                let l = latency(&rows);
                (key, (l.tokens_per_second_p50, l.ttft_p50_ms))
            })
            .collect();
        *self.speeds.write().await = SpeedBoard { refreshed: Some(Instant::now()), by_route };
    }

    async fn pool_prices(&self, model: &str, ids: &HashMap<String, i64>) -> HashMap<i64, f64> {
        let rows = self.db.list_model_pricings_by_name(model).await.unwrap_or_default();
        let fallback = rows.iter().find(|r| r.provider_id.is_none());
        let priced: Vec<(i64, crate::api::public_models_math::Price)> = ids
            .values()
            .filter_map(|id| {
                let row = rows.iter().find(|r| r.provider_id == Some(*id)).or(fallback)?;
                price_of(row).map(|p| (*id, p))
            })
            .collect();
        let mut counts: HashMap<&str, usize> = HashMap::new();
        for (_, p) in &priced {
            *counts.entry(p.currency.as_str()).or_default() += 1;
        }
        let Some(currency) = counts.into_iter().max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(a.0))).map(|c| c.0.to_string())
        else {
            return HashMap::new();
        };
        priced
            .into_iter()
            .filter(|(_, p)| p.currency == currency)
            .map(|(id, p)| (id, p.input_per_1m + p.output_per_1m))
            .collect()
    }

    async fn stored_preference(&self, viewer: Option<i64>, model: &str) -> (Option<Preference>, Option<Preference>) {
        let Some(user) = viewer else { return (None, None) };
        let Ok(prefs) = self.db.routing_preferences(user).await else { return (None, None) };
        let per_model = prefs
            .models
            .iter()
            .find(|(m, _)| m.eq_ignore_ascii_case(model))
            .and_then(|(_, p)| Preference::from_setting(p));
        (per_model, prefs.default.as_deref().and_then(Preference::from_setting))
    }

    pub(crate) async fn preference_for(
        &self,
        viewer: Option<i64>,
        model: &str,
        requested: Option<Preference>,
    ) -> Preference {
        let (per_model, default) = self.stored_preference(viewer, model).await;
        resolve(requested, per_model, default)
    }

    pub(crate) async fn apply_preference(
        &self,
        model: &str,
        ids: &HashMap<String, i64>,
        viewer: Option<i64>,
        requested: Option<Preference>,
        session: Option<&SessionKey>,
        candidates: &mut Vec<Candidate>,
    ) {
        let preference = self.preference_for(viewer, model, requested).await;
        let (prices, speeds) = match preference {
            Preference::Balanced => (HashMap::new(), None),
            Preference::Cheapest => (self.pool_prices(model, ids).await, None),
            Preference::Fastest | Preference::Latency => {
                self.refresh_speeds().await;
                (HashMap::new(), Some(()))
            }
        };
        let board = self.speeds.read().await;
        let key = model.to_lowercase();
        let limits = self.provider_limits.read().unwrap().clone();
        let mut stats = Vec::with_capacity(candidates.len());
        for c in candidates.iter() {
            let name = c.provider.name();
            let mut stat = Stat::default();
            if let Some(id) = ids.get(name) {
                stat.price = prices.get(id).copied();
            }
            if speeds.is_some() {
                if let Some((tps, ttft)) = board.by_route.get(&(name.to_string(), key.clone())) {
                    stat.tokens_per_second = *tps;
                    stat.ttft_ms = *ttft;
                }
            }
            if let Some(limit) = limits.get(c.provider.slug()) {
                stat.at_capacity = self.metrics_store.get_in_flight(name).await >= *limit;
            }
            stats.push(stat);
        }
        let pinned = session
            .and_then(|k| self.sticky.provider_for(k))
            .and_then(|pin| candidates.iter().position(|c| c.provider.name() == pin));
        let order = reorder(preference, &stats, pinned);
        let mut slots: Vec<Option<Candidate>> = std::mem::take(candidates).into_iter().map(Some).collect();
        *candidates = order.into_iter().filter_map(|i| slots[i].take()).collect();
    }
}
