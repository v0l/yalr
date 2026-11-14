use serde::Serialize;

use crate::db::{Database, VerificationRow};
use crate::verify::{ProbeResult, Scores};

#[derive(Serialize)]
pub struct VerificationSummary {
    pub model: String,
    pub outcome: String,
    pub passed: bool,
    pub reason: Option<String>,
    pub scores: Option<Scores>,
    pub reference_provider_id: Option<i64>,
    pub checked_at: crate::db::Timestamp,
}

#[derive(Serialize)]
pub struct VerificationDetail {
    #[serde(flatten)]
    pub summary: VerificationSummary,
    pub run_id: String,
    pub results: Vec<ProbeResult>,
}

impl From<&VerificationRow> for VerificationSummary {
    fn from(row: &VerificationRow) -> Self {
        Self {
            model: row.model_name.clone(),
            outcome: row.outcome.clone(),
            passed: row.passed,
            reason: row.reason.clone(),
            scores: row.scores.as_deref().and_then(|s| serde_json::from_str(s).ok()),
            reference_provider_id: row.reference_provider_id,
            checked_at: row.created_at,
        }
    }
}

impl From<VerificationRow> for VerificationDetail {
    fn from(row: VerificationRow) -> Self {
        Self {
            summary: (&row).into(),
            results: row.results.as_deref().and_then(|s| serde_json::from_str(s).ok()).unwrap_or_default(),
            run_id: row.run_id,
        }
    }
}

pub async fn latest(db: &Database, provider_id: i64) -> Vec<VerificationSummary> {
    db.latest_verifications(provider_id).await.unwrap_or_default().iter().map(Into::into).collect()
}

pub async fn history(db: &Database, provider_id: i64) -> Vec<VerificationDetail> {
    db.list_verifications(provider_id, 30).await.unwrap_or_default().into_iter().map(Into::into).collect()
}
