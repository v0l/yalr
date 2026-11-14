use std::sync::Arc;
use std::time::Duration;

use super::runner::{verify_model, ModelReport};
use super::settings::{self, VerificationConfig};
use super::{OUTCOME_NO_REFERENCE, OUTCOME_PASSED};
use crate::db::{Database, NewVerification, Provider, APPROVED, PENDING, SUSPENDED};
use crate::providers::Provider as ProviderApi;
use crate::router::engine::Router;

const SCHEDULER_TICK: Duration = Duration::from_secs(10 * 60);

async fn live(router: &Router, slug: &str) -> Option<Arc<dyn ProviderApi>> {
    router.get_providers().await.into_iter().find(|p| p.slug() == slug)
}

async fn served_name(provider: &Arc<dyn ProviderApi>, model: &str) -> String {
    match provider.list_models().await {
        Ok(models) => models
            .into_iter()
            .map(|m| m.id)
            .find(|id| id.eq_ignore_ascii_case(model))
            .unwrap_or_else(|| model.to_string()),
        Err(_) => model.to_string(),
    }
}

async fn run_models(
    db: &Database,
    router: &Router,
    provider: &Provider,
    config: &VerificationConfig,
) -> Vec<ModelReport> {
    let models = db.priced_models_for_provider(provider.id).await.unwrap_or_default();
    let Some(candidate) = live(router, &provider.slug).await else {
        return models
            .iter()
            .map(|m| ModelReport::without_result(m, super::OUTCOME_ERROR, "provider is not loaded".to_string()))
            .collect();
    };
    let mut reports = Vec::new();
    for model in models {
        let mut report = None;
        for reference in db.reference_providers_for(&model, provider.id).await.unwrap_or_default() {
            let Some(live_reference) = live(router, &reference.slug).await else { continue };
            let candidate_name = served_name(&candidate, &model).await;
            let reference_name = served_name(&live_reference, &model).await;
            let mut run = verify_model(&candidate, &candidate_name, &live_reference, &reference_name, config).await;
            run.model = model.clone();
            run.reference_provider_id = Some(reference.id);
            let conclusive = run.is_conclusive();
            report = Some(run);
            if conclusive {
                break;
            }
        }
        reports.push(report.unwrap_or_else(|| {
            ModelReport::without_result(
                &model,
                OUTCOME_NO_REFERENCE,
                "no approved reference provider is marked for this model".to_string(),
            )
        }));
    }
    reports
}

async fn store(db: &Database, provider: &Provider, reports: &[ModelReport]) {
    let run_id = uuid::Uuid::new_v4().to_string();
    for report in reports {
        let scores = serde_json::to_string(&report.scores).unwrap_or_default();
        let results = serde_json::to_string(&report.results).unwrap_or_default();
        let reason = (!report.reason.is_empty()).then_some(report.reason.as_str());
        let stored = db
            .insert_verification(NewVerification {
                run_id: &run_id,
                provider_id: provider.id,
                model_name: &report.model,
                reference_provider_id: report.reference_provider_id,
                outcome: report.outcome,
                passed: report.passed,
                reason,
                scores: &scores,
                results: &results,
            })
            .await;
        if let Err(e) = stored {
            tracing::error!(error = %e, provider = %provider.slug, "Failed to store verification");
        }
    }
}

fn failure_summary(reports: &[ModelReport]) -> String {
    reports
        .iter()
        .filter(|r| r.outcome == super::OUTCOME_FAILED)
        .map(|r| format!("{}: {}", r.model, r.reason))
        .collect::<Vec<_>>()
        .join(" | ")
}

fn decide(current: &Provider, reports: &[ModelReport]) -> Option<(&'static str, Option<String>)> {
    if current.status_locked || reports.is_empty() {
        return None;
    }
    if reports.iter().any(|r| r.outcome == super::OUTCOME_FAILED) {
        let reason = failure_summary(reports);
        return Some(if current.status == PENDING { (PENDING, Some(reason)) } else { (SUSPENDED, Some(reason)) });
    }
    if reports.iter().all(|r| r.outcome == OUTCOME_PASSED) {
        return Some((APPROVED, None));
    }
    if current.status == PENDING {
        let waiting: Vec<String> = reports
            .iter()
            .filter(|r| r.outcome != OUTCOME_PASSED)
            .map(|r| format!("{}: {}", r.model, r.reason))
            .collect();
        return Some((PENDING, Some(format!("not verified yet: {}", waiting.join(" | ")))));
    }
    None
}

pub async fn verify_and_apply(db: &Arc<Database>, router: &Arc<Router>, provider_id: i64) -> Vec<ModelReport> {
    let Ok(Some(provider)) = db.get_provider_by_id(provider_id).await else {
        return vec![];
    };
    let config = settings::get();
    let reports = run_models(db, router, &provider, &config).await;
    store(db, &provider, &reports).await;
    if let Some((status, reason)) = decide(&provider, &reports) {
        if status != provider.status || reason != provider.status_reason {
            if let Err(e) = db.set_provider_status(provider.id, status, reason.as_deref(), false).await {
                tracing::error!(error = %e, provider = %provider.slug, "Failed to update provider status");
            } else {
                tracing::info!(provider = %provider.slug, status, "Provider verification applied");
                router.refresh_full_name_index().await;
            }
        }
    }
    reports
}

pub async fn mark_changed(db: &Arc<Database>, router: &Arc<Router>, provider: &Provider) {
    if provider.owner_user_id.is_none() || provider.is_reference {
        return;
    }
    if provider.status == APPROVED && !provider.status_locked {
        let _ = db.set_provider_status(provider.id, PENDING, Some("changed, re-verifying"), false).await;
        router.refresh_full_name_index().await;
    }
    if settings::background_enabled() {
        let (db, router, id) = (db.clone(), router.clone(), provider.id);
        tokio::spawn(async move {
            verify_and_apply(&db, &router, id).await;
        });
    }
}

pub async fn run_due(db: &Arc<Database>, router: &Arc<Router>) -> usize {
    let config = settings::get();
    let due = db.providers_due_for_verification(config.provider_verify_interval_hours).await.unwrap_or_default();
    for provider in &due {
        verify_and_apply(db, router, provider.id).await;
    }
    due.len()
}

pub fn spawn_scheduler(db: Arc<Database>, router: Arc<Router>) {
    settings::enable_background();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(SCHEDULER_TICK);
        loop {
            tick.tick().await;
            if settings::get().enabled {
                run_due(&db, &router).await;
            }
        }
    });
}
