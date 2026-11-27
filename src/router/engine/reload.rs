use super::*;

type Fingerprint = (
    crate::db::ProviderType,
    String,
    String,
    String,
    Option<String>,
    Option<i64>,
    Option<String>,
    [Option<String>; 3],
    Option<i64>,
);

fn fingerprint(r: &crate::db::Provider) -> Fingerprint {
    (
        r.provider_type,
        r.name.clone(),
        r.slug.clone(),
        r.base_url.clone(),
        r.api_key.clone(),
        r.owner_user_id,
        r.client_version.clone(),
        [r.oauth_access_token.clone(), r.oauth_refresh_token.clone(), r.oauth_account_id.clone()],
        r.oauth_expires_at,
    )
}

pub(super) type Fingerprints = std::sync::Mutex<HashMap<String, Fingerprint>>;

struct Built {
    providers: HashMap<String, Arc<dyn Provider>>,
    fingerprints: HashMap<String, Fingerprint>,
    rebuilt: Vec<Arc<dyn Provider>>,
}

impl Router {
    async fn build_providers(&self, records: &[crate::db::Provider]) -> Built {
        let current = self.providers.read().await.clone();
        let known = self.fingerprints.lock().unwrap().clone();
        let mut built = Built { providers: HashMap::new(), fingerprints: HashMap::new(), rebuilt: vec![] };
        for record in records {
            let print = fingerprint(record);
            let kept = current.get(&record.slug).filter(|_| known.get(&record.slug) == Some(&print)).cloned();
            let provider = match kept {
                Some(p) => p,
                None => match create_provider_with_egress(record, self.db.clone(), &self.egress, self.provider_access.clone()) {
                    Some(p) => {
                        built.rebuilt.push(p.clone());
                        p
                    }
                    None => {
                        tracing::warn!(provider = %record.name, base_url = %record.base_url, "Failed to construct provider, skipping");
                        continue;
                    }
                },
            };
            self.metrics_store.register_provider(&record.name).await;
            built.fingerprints.insert(record.slug.clone(), print);
            built.providers.insert(record.slug.clone(), provider);
        }
        built
    }

    pub async fn reload_config(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let started = Instant::now();
        let records = self.db.list_providers().await?;
        let configs = self.db.list_routing_configs().await?;
        let mut links = HashMap::new();
        for rc in &configs {
            links.insert(rc.id, self.db.list_active_routing_config_providers(rc.id).await?);
        }
        let model_records = self.db.list_models().await?;
        let mp_records = self.db.list_model_providers().await?;
        let db_ms = started.elapsed().as_millis();

        let built_at = Instant::now();
        self.set_provider_access(&records);
        let built = self.build_providers(&records).await;
        let provider_bodies: HashMap<String, crate::db::ExtraBody> =
            records.iter().filter_map(|r| r.extra_body_map().map(|b| (r.slug.clone(), b))).collect();
        let tables = build_tables(&records, &built.providers, &provider_bodies, &configs, &links, &model_records, &mp_records);
        let build_ms = built_at.elapsed().as_millis();

        let swap_at = Instant::now();
        self.set_provider_tuning(&records);
        self.set_provider_ids(&records);
        let any_providers = !built.providers.is_empty();
        let table_names: Vec<String> = tables.keys().cloned().collect();
        *self.providers.write().await = built.providers;
        *self.fingerprints.lock().unwrap() = built.fingerprints;
        if let Ok(mut bodies) = self.provider_bodies.write() {
            *bodies = provider_bodies;
        }
        *self.routing_tables.write().await = tables;
        let rebuilt_slugs = built.rebuilt.iter().map(|p| p.slug().to_string()).collect();
        self.schedule_relist(full_name::metas(&records), rebuilt_slugs);
        self.ensure_health_loop(&configs, built.rebuilt.clone(), any_providers);
        let swap_ms = swap_at.elapsed().as_millis();

        tracing::info!(
            providers_loaded = records.len(),
            providers_rebuilt = built.rebuilt.len(),
            routing_tables = ?table_names,
            "reload_config done in {} ms (db {db_ms}, build {build_ms}, swap {swap_ms})",
            started.elapsed().as_millis()
        );
        Ok(())
    }
}

fn build_tables(
    records: &[crate::db::Provider],
    providers: &HashMap<String, Arc<dyn Provider>>,
    provider_bodies: &HashMap<String, crate::db::ExtraBody>,
    configs: &[crate::db::RoutingConfig],
    links: &HashMap<i64, Vec<crate::db::RoutingConfigProvider>>,
    model_records: &[crate::db::Model],
    mp_records: &[crate::db::ModelProvider],
) -> HashMap<String, RoutingTable> {
    let id_to_slug: HashMap<i64, &String> =
        records.iter().filter(|r| providers.contains_key(&r.slug)).map(|r| (r.id, &r.slug)).collect();
    let owned_ids: std::collections::HashSet<i64> =
        records.iter().filter(|r| r.owner_user_id.is_some()).map(|r| r.id).collect();
    let provider_owner: HashMap<i64, (i64, bool)> = records
        .iter()
        .filter_map(|r| r.owner_user_id.map(|owner| (r.id, (owner, r.status == crate::db::APPROVED))))
        .collect();
    let lookup = |id: Option<i64>| -> Option<(&String, &Arc<dyn Provider>)> {
        let slug = id.and_then(|id| id_to_slug.get(&id).copied())?;
        providers.get(slug).map(|p| (slug, p))
    };

    let mut tables = HashMap::new();
    for rc in configs {
        let mut entries = Vec::new();
        let mut children = Vec::new();
        for rcp in links.get(&rc.id).map(Vec::as_slice).unwrap_or_default() {
            if let Some(child_id) = rcp.child_config_id {
                if let Some(child) = configs.iter().find(|c| c.id == child_id) {
                    children.push(ChildEntry {
                        config: child.name.clone(),
                        model_override: rcp.model.clone(),
                        weight: rcp.weight,
                        extra_body: rcp.extra_body_map().unwrap_or_default(),
                    });
                }
                continue;
            }
            if let Some((owner, approved)) = rcp.provider_id.and_then(|id| provider_owner.get(&id)) {
                let system_pool = rc.owner_user_id.is_none() && *approved;
                if rc.owner_user_id != Some(*owner) && !system_pool {
                    continue;
                }
            }
            let Some((slug, provider)) = lookup(rcp.provider_id) else { continue };
            let mut extra_body = provider_bodies.get(slug).cloned().unwrap_or_default();
            if let Some(entry_body) = rcp.extra_body_map() {
                crate::db::merge_extra_body(&mut extra_body, &entry_body);
            }
            entries.push(ProviderEntry { provider: provider.clone(), model_override: rcp.model.clone(), weight: rcp.weight, extra_body });
        }
        let entry_names: Vec<&str> = entries.iter().map(|e| e.provider.name()).collect();
        tracing::debug!(
            routing_config = rc.name,
            strategy = rc.strategy,
            provider_count = entries.len(),
            child_count = children.len(),
            providers = ?entry_names,
            "Loaded routing config"
        );
        tables.insert(rc.name.clone(), RoutingTable::with_children(entries, children, StrategyKind::from_str(&rc.strategy)));
    }

    let model_names: HashMap<i64, &String> = model_records.iter().map(|m| (m.id, &m.name)).collect();
    for mp in mp_records.iter().filter(|mp| mp.is_active && !owned_ids.contains(&mp.provider_id)) {
        let Some(model_name) = model_names.get(&mp.model_id) else { continue };
        if tables.contains_key(*model_name) {
            continue;
        }
        let Some((slug, provider)) = lookup(Some(mp.provider_id)) else { continue };
        tables.entry((*model_name).clone()).or_insert_with(|| RoutingTable::new(Vec::new())).entries.push(ProviderEntry {
            provider: provider.clone(),
            model_override: None,
            weight: mp.weight,
            extra_body: provider_bodies.get(slug).cloned().unwrap_or_default(),
        });
    }
    tables
}
