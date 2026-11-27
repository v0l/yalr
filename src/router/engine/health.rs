use super::*;

pub(super) type ProviderMap = Arc<RwLock<HashMap<String, Arc<dyn Provider>>>>;

pub(super) struct HealthLoop {
    interval: Duration,
    timeout: Duration,
    handle: tokio::task::JoinHandle<()>,
}

#[derive(Clone)]
pub(super) struct HealthProbe {
    router: HealthCheckRouter,
    provider_ids: Arc<std::sync::RwLock<HashMap<String, i64>>>,
    access: crate::providers::guarded::Access,
    db: Arc<Database>,
    timeout: Duration,
}

pub(super) fn health_settings(configs: &[crate::db::RoutingConfig]) -> (Duration, Duration, bool) {
    let health_config = configs.iter().find(|rc| rc.health_check_enabled);
    let interval = Duration::from_secs(health_config.map_or(30, |rc| sane_health_secs(rc.health_check_interval_seconds, 30)));
    let timeout = Duration::from_secs(health_config.map_or(10, |rc| sane_health_secs(rc.health_check_timeout_seconds, 10)));
    (interval, timeout, health_config.is_some())
}

impl HealthProbe {
    fn held(&self, provider: &Arc<dyn Provider>) -> bool {
        crate::providers::guarded::GuardedProvider::is_held(&self.access, provider.slug())
    }

    async fn check(self, provider: Arc<dyn Provider>) {
        if self.held(&provider) {
            return;
        }
        let name = provider.name().to_string();
        match tokio::time::timeout(Duration::from_secs(5), provider.fetch_balance()).await {
            Ok(Some(amount)) => self.router.metrics_store.emitter().emit_balance(&name, amount, None),
            Ok(None) => {}
            Err(_) => tracing::debug!(provider = %name, "Balance fetch timeout"),
        }
        match tokio::time::timeout(Duration::from_secs(5), provider.fetch_quota()).await {
            Ok(Some(quota)) => self.router.metrics_store.emitter().emit_quota(&name, quota, None),
            Ok(None) => {}
            Err(_) => tracing::debug!(provider = %name, "Quota fetch timeout"),
        }
        let result = tokio::time::timeout(self.timeout, provider.health_check()).await;
        let id = self.provider_ids.read().unwrap().get(&name).copied();
        if let Some(id) = id {
            let healthy = matches!(result, Ok(Ok(true)));
            let _ = self.db.record_health_check(id, healthy, chrono::Utc::now().timestamp_millis()).await;
        }
        match result {
            Ok(Ok(true)) => {
                if !self.router.metrics_store.is_provider_available(&name).await {
                    tracing::info!(provider = %name, "Health check recovered");
                }
                self.router.record_success(&name).await;
            }
            Ok(Ok(false)) => {
                tracing::warn!(provider = %name, "Health check unhealthy");
                self.router
                    .record_failure(&name, crate::metrics::ErrorType::ServerError, "Health check returned unhealthy")
                    .await;
            }
            Ok(Err(e)) => {
                tracing::warn!(provider = %name, error = %e, "Health check error");
                self.router.record_failure(&name, e.error_type(), &e.to_string()).await;
            }
            Err(_) => {
                tracing::warn!(provider = %name, "Health check timeout");
                self.router
                    .record_failure(&name, crate::metrics::ErrorType::Timeout, "Health check timed out")
                    .await;
            }
        }
    }

    pub(super) fn check_now(&self, providers: Vec<Arc<dyn Provider>>) {
        for provider in providers {
            tokio::spawn(self.clone().check(provider));
        }
    }
}

async fn distinct_providers(providers: &ProviderMap) -> Vec<Arc<dyn Provider>> {
    let map = providers.read().await;
    let mut seen = std::collections::HashSet::new();
    map.values().filter(|p| seen.insert(p.name().to_string())).cloned().collect()
}

async fn run_loop(providers: ProviderMap, probe: HealthProbe, interval: Duration) {
    let mut tick = tokio::time::interval(interval);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tick.tick().await;
        let tasks: Vec<_> = distinct_providers(&providers)
            .await
            .into_iter()
            .map(|provider| tokio::spawn(probe.clone().check(provider)))
            .collect();
        for task in tasks {
            let _ = tokio::time::timeout(Duration::from_secs(60), task).await;
        }
    }
}

impl Router {
    fn health_probe(&self, timeout: Duration) -> HealthProbe {
        HealthProbe {
            router: HealthCheckRouter { metrics_store: self.metrics_store.clone() },
            provider_ids: self.provider_ids.clone(),
            access: self.provider_access.clone(),
            db: self.db.clone(),
            timeout,
        }
    }

    pub(super) fn set_provider_ids(&self, records: &[crate::db::Provider]) {
        *self.provider_ids.write().unwrap() = records.iter().map(|r| (r.name.clone(), r.id)).collect();
    }

    pub(super) fn ensure_health_loop(&self, configs: &[crate::db::RoutingConfig], rebuilt: Vec<Arc<dyn Provider>>, any_providers: bool) {
        let (interval, timeout, configured) = health_settings(configs);
        let mut slot = self.health.lock().unwrap();
        if let Some(running) = slot.as_ref().filter(|l| l.interval == interval && l.timeout == timeout && !l.handle.is_finished()) {
            self.health_probe(running.timeout).check_now(rebuilt);
            return;
        }
        if !any_providers {
            tracing::warn!("No providers loaded - health checks will not start");
            return;
        }
        let probe = self.health_probe(timeout);
        let handle = tokio::spawn(run_loop(self.providers.clone(), probe, interval));
        self.health_loops.fetch_add(1, Ordering::SeqCst);
        if let Some(old) = slot.replace(HealthLoop { interval, timeout, handle }) {
            old.handle.abort();
        }
        tracing::info!(
            has_health_config = configured,
            interval_seconds = interval.as_secs(),
            timeout_seconds = timeout.as_secs(),
            "Started health check task for all providers"
        );
    }

    pub async fn start_health_checks(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let configs = self.db.list_routing_configs().await?;
        let any = !self.providers.read().await.is_empty();
        self.ensure_health_loop(&configs, vec![], any);
        Ok(())
    }
}
