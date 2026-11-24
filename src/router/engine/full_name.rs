use super::*;

const INDEX_TTL: Duration = Duration::from_secs(60);
pub(super) const LIST_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone)]
pub(crate) struct IndexedModel {
    pub(crate) slug: String,
    pub(crate) model_id: String,
    pub(crate) provider_id: i64,
    pub(crate) owner_user_id: Option<i64>,
    pub(crate) status: String,
}

#[derive(Clone)]
pub(super) struct ProviderMeta {
    provider_id: i64,
    owner_user_id: Option<i64>,
    status: String,
}

pub(super) type Metas = HashMap<String, ProviderMeta>;

pub(super) fn metas(records: &[crate::db::Provider]) -> Metas {
    records
        .iter()
        .map(|r| {
            let meta = ProviderMeta { provider_id: r.id, owner_user_id: r.owner_user_id, status: r.status.clone() };
            (r.slug.clone(), meta)
        })
        .collect()
}

struct Listing {
    meta: ProviderMeta,
    models: Vec<String>,
}

#[derive(Default)]
pub(crate) struct FullNameIndex {
    models: HashMap<String, Vec<IndexedModel>>,
    listings: std::collections::BTreeMap<String, Listing>,
    counters: HashMap<String, Arc<AtomicUsize>>,
    refreshed: Option<Instant>,
}

impl FullNameIndex {
    fn apply_metas(&mut self, metas: &Metas) {
        self.listings.retain(|slug, _| metas.contains_key(slug));
        for (slug, listing) in self.listings.iter_mut() {
            listing.meta = metas[slug].clone();
        }
        self.rebuild();
    }

    fn rebuild(&mut self) {
        let mut models: HashMap<String, Vec<IndexedModel>> = HashMap::new();
        for (slug, listing) in &self.listings {
            for id in &listing.models {
                models.entry(id.to_lowercase()).or_default().push(IndexedModel {
                    slug: slug.clone(),
                    model_id: id.clone(),
                    provider_id: listing.meta.provider_id,
                    owner_user_id: listing.meta.owner_user_id,
                    status: listing.meta.status.clone(),
                });
            }
        }
        self.models = models;
    }
}

pub(super) type Pending = Arc<std::sync::Mutex<std::collections::HashSet<String>>>;

enum Listed {
    Models(Vec<String>),
    Failed(String),
    TimedOut,
}

async fn list_one(provider: Arc<dyn Provider>, limit: Duration) -> (String, Arc<dyn Provider>, Duration, Listed) {
    let started = Instant::now();
    let listed = match tokio::time::timeout(limit, provider.list_models()).await {
        Ok(Ok(models)) => Listed::Models(models.into_iter().map(|m| m.id).filter(|id| is_full_name(id)).collect()),
        Ok(Err(e)) => Listed::Failed(e.to_string()),
        Err(_) => Listed::TimedOut,
    };
    (provider.slug().to_string(), provider, started.elapsed(), listed)
}

pub(super) async fn relist(
    providers: health::ProviderMap,
    index: Arc<RwLock<FullNameIndex>>,
    metas: Metas,
    slugs: Vec<String>,
    limit: Duration,
    pending: Option<Pending>,
) {
    let started = Instant::now();
    index.write().await.apply_metas(&metas);
    let targets: Vec<Arc<dyn Provider>> = {
        let loaded = providers.read().await;
        slugs.iter().filter(|s| metas.contains_key(*s)).filter_map(|s| loaded.get(s).cloned()).collect()
    };
    let mut listing: futures::stream::FuturesUnordered<_> = targets.into_iter().map(|p| list_one(p, limit)).collect();
    let mut report = Vec::new();
    while let Some((slug, provider, took, listed)) = listing.next().await {
        let current = providers.read().await.get(&slug).is_some_and(|p| Arc::ptr_eq(p, &provider));
        let outcome = match &listed {
            Listed::Models(models) => format!("{slug}={}ms/{}", took.as_millis(), models.len()),
            Listed::Failed(e) => format!("{slug}={}ms failed: {e}", took.as_millis()),
            Listed::TimedOut => format!("{slug}=timeout after {}ms", took.as_millis()),
        };
        report.push(outcome);
        if let (true, Listed::Models(models), Some(meta)) = (current, listed, metas.get(&slug)) {
            let mut index = index.write().await;
            index.listings.insert(slug.clone(), Listing { meta: meta.clone(), models });
            index.rebuild();
        }
        if let Some(pending) = pending.as_ref().filter(|_| current) {
            pending.lock().unwrap().remove(&slug);
        }
    }
    index.write().await.refreshed = Some(Instant::now());
    tracing::info!(
        listed = report.len(),
        providers = %report.join(", "),
        "full name index refreshed in {} ms",
        started.elapsed().as_millis()
    );
}

pub(crate) fn serves(owner_user_id: Option<i64>, status: &str, viewer: Option<i64>) -> bool {
    match status {
        "approved" => true,
        "pending" => owner_user_id.is_some() && owner_user_id == viewer,
        _ => false,
    }
}

fn visible_to(member: &IndexedModel, viewer: Option<i64>) -> bool {
    serves(member.owner_user_id, &member.status, viewer)
}

impl Router {
    pub(crate) fn set_provider_access(&self, records: &[crate::db::Provider]) {
        let access = records
            .iter()
            .filter(|r| r.owner_user_id.is_some() || r.status != crate::db::APPROVED)
            .map(|r| (r.slug.clone(), (r.owner_user_id, r.status.clone())))
            .collect();
        *self.provider_access.write().unwrap() = access;
        let limits = records
            .iter()
            .filter_map(|r| r.max_concurrency.filter(|l| *l > 0).map(|l| (r.slug.clone(), l as u32)))
            .collect();
        *self.provider_limits.write().unwrap() = limits;
    }

    pub(crate) fn may_serve(&self, slug: &str, viewer: Option<i64>) -> bool {
        match self.provider_access.read().unwrap().get(slug) {
            Some((owner, status)) => serves(*owner, status, viewer),
            None => true,
        }
    }
}

pub(crate) fn is_full_name(model: &str) -> bool {
    model.contains('/')
}

impl Router {
    pub async fn refresh_full_name_index(&self) {
        let records = self.db.list_providers().await.unwrap_or_default();
        self.set_provider_access(&records);
        let slugs = records.iter().map(|r| r.slug.clone()).collect();
        relist(self.providers.clone(), self.full_names.clone(), metas(&records), slugs, self.list_timeout, None).await;
    }

    pub async fn refresh_provider_access(&self) {
        let records = self.db.list_providers().await.unwrap_or_default();
        self.set_provider_access(&records);
        self.full_names.write().await.apply_metas(&metas(&records));
    }

    pub(super) fn schedule_relist(&self, metas: Metas, rebuilt: Vec<String>) {
        let slugs: Vec<String> = {
            let mut pending = self.pending_relist.lock().unwrap();
            pending.retain(|s| metas.contains_key(s));
            pending.extend(rebuilt);
            pending.iter().cloned().collect()
        };
        let task = relist(
            self.providers.clone(),
            self.full_names.clone(),
            metas,
            slugs,
            self.list_timeout,
            Some(self.pending_relist.clone()),
        );
        let handle = tokio::spawn(task);
        if let Some(old) = self.index_refresh.lock().unwrap().replace(handle) {
            old.abort();
        }
    }

    pub async fn settle_background(&self) {
        loop {
            let handle = self.index_refresh.lock().unwrap().take();
            match handle {
                Some(handle) => {
                    let _ = handle.await;
                }
                None => break,
            }
        }
    }

    pub fn index_refreshes_running(&self) -> usize {
        self.index_refresh.lock().unwrap().as_ref().map_or(0, |h| usize::from(!h.is_finished()))
    }

    async fn ensure_index_for(&self, model: &str) {
        let key = model.to_lowercase();
        let stale = {
            let index = self.full_names.read().await;
            !index.models.contains_key(&key) && index.refreshed.map_or(true, |at| at.elapsed() > INDEX_TTL)
        };
        if stale {
            self.refresh_full_name_index().await;
        }
    }

    async fn pool_members(&self, model: &str, viewer: Option<i64>) -> Vec<(IndexedModel, Arc<dyn Provider>)> {
        self.ensure_index_for(model).await;
        let key = model.to_lowercase();
        let indexed = self.full_names.read().await.models.get(&key).cloned().unwrap_or_default();
        if indexed.is_empty() {
            return vec![];
        }
        let priced: std::collections::HashSet<i64> = self
            .db
            .list_model_pricings_by_name(model)
            .await
            .unwrap_or_default()
            .into_iter()
            .filter_map(|r| r.provider_id)
            .collect();
        let providers = self.providers.read().await;
        indexed
            .into_iter()
            .filter(|m| m.owner_user_id.is_none() || priced.contains(&m.provider_id))
            .filter(|m| visible_to(m, viewer))
            .filter_map(|m| providers.get(&m.slug).cloned().map(|p| (m, p)))
            .collect()
    }

    pub(crate) async fn full_name_backends(&self, model: &str) -> Vec<(Arc<dyn Provider>, String)> {
        self.pool_members(model, None).await.into_iter().map(|(m, p)| (p, m.model_id)).collect()
    }

    pub(crate) async fn rank_full_name(
        &self,
        model: &str,
        session: Option<&SessionKey>,
        viewer: Option<i64>,
        requested: Option<crate::router::preference::Preference>,
    ) -> Result<Vec<Candidate>, RouterError> {
        let members = self.pool_members(model, viewer).await;
        if members.is_empty() {
            return Ok(vec![]);
        }
        let key = model.to_lowercase();
        let counter = self.full_names.write().await.counters.entry(key.clone()).or_default().clone();
        let members_by_name: HashMap<String, i64> =
            members.iter().map(|(m, p)| (p.name().to_string(), m.provider_id)).collect();
        let entries = members
            .into_iter()
            .map(|(m, provider)| ProviderEntry {
                extra_body: self.provider_body(provider.as_ref()),
                provider,
                model_override: Some(m.model_id),
                weight: 1,
            })
            .collect();
        let mut tables = HashMap::new();
        tables.insert(key.clone(), RoutingTable::with_counter(entries, counter));
        let ranked = self.rank_table(&tables, &key, model, "", 1).await?;
        let mut candidates = ranked.candidates;
        if ranked.tier == Tier::Healthy {
            self.apply_preference(model, &members_by_name, viewer, requested, session, &mut candidates).await;
        } else {
            self.sticky.move_pinned_first(session, &mut candidates, |c| c.provider.name());
        }
        Ok(candidates)
    }

    pub async fn full_name_models(&self) -> Vec<String> {
        let index = self.full_names.read().await;
        let mut names: Vec<String> = index
            .models
            .values()
            .filter_map(|entries| entries.first().map(|m| m.model_id.clone()))
            .collect();
        names.sort();
        names
    }

    pub async fn full_name_providers(&self, model: &str) -> Vec<(String, i64, Option<i64>)> {
        self.pool_members(model, None).await.into_iter().map(|(m, _)| (m.slug, m.provider_id, m.owner_user_id)).collect()
    }
}
