use super::*;

const INDEX_TTL: Duration = Duration::from_secs(60);
const LIST_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone)]
pub(crate) struct IndexedModel {
    pub(crate) slug: String,
    pub(crate) model_id: String,
    pub(crate) provider_id: i64,
    pub(crate) owner_user_id: Option<i64>,
}

#[derive(Default)]
pub(crate) struct FullNameIndex {
    models: HashMap<String, Vec<IndexedModel>>,
    counters: HashMap<String, Arc<AtomicUsize>>,
    refreshed: Option<Instant>,
}

pub(crate) fn is_full_name(model: &str) -> bool {
    model.contains('/')
}

impl Router {
    pub async fn refresh_full_name_index(&self) {
        let records = self.db.list_providers().await.unwrap_or_default();
        let providers: Vec<(Arc<dyn Provider>, i64, Option<i64>)> = {
            let loaded = self.providers.read().await;
            records
                .iter()
                .filter_map(|r| loaded.get(&r.slug).map(|p| (p.clone(), r.id, r.owner_user_id)))
                .collect()
        };
        let listings = futures::future::join_all(providers.into_iter().map(|(provider, id, owner)| async move {
            let models = tokio::time::timeout(LIST_TIMEOUT, provider.list_models()).await.ok().and_then(Result::ok);
            (provider.slug().to_string(), id, owner, models.unwrap_or_default())
        }))
        .await;

        let mut models: HashMap<String, Vec<IndexedModel>> = HashMap::new();
        for (slug, provider_id, owner_user_id, listed) in listings {
            for model in listed.into_iter().filter(|m| is_full_name(&m.id)) {
                models.entry(model.id.to_lowercase()).or_default().push(IndexedModel {
                    slug: slug.clone(),
                    model_id: model.id,
                    provider_id,
                    owner_user_id,
                });
            }
        }
        let mut index = self.full_names.write().await;
        index.models = models;
        index.refreshed = Some(Instant::now());
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

    async fn pool_members(&self, model: &str) -> Vec<(IndexedModel, Arc<dyn Provider>)> {
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
            .filter_map(|m| providers.get(&m.slug).cloned().map(|p| (m, p)))
            .collect()
    }

    pub(crate) async fn full_name_backends(&self, model: &str) -> Vec<(Arc<dyn Provider>, String)> {
        self.pool_members(model).await.into_iter().map(|(m, p)| (p, m.model_id)).collect()
    }

    pub(crate) async fn rank_full_name(
        &self,
        model: &str,
        session: Option<&SessionKey>,
    ) -> Result<Vec<Candidate>, RouterError> {
        let members = self.pool_members(model).await;
        if members.is_empty() {
            return Ok(vec![]);
        }
        let key = model.to_lowercase();
        let counter = self.full_names.write().await.counters.entry(key.clone()).or_default().clone();
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
        self.sticky.move_pinned_first(session, &mut candidates, |c| c.provider.name());
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
        self.pool_members(model).await.into_iter().map(|(m, _)| (m.slug, m.provider_id, m.owner_user_id)).collect()
    }
}
