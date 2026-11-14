mod lifecycle;
mod probes;
mod runner;
mod score;
pub mod settings;
#[cfg(test)]
mod tests;

pub use lifecycle::{mark_changed, run_due, spawn_scheduler, verify_and_apply};
pub use probes::{draw_probes, fixed_probes, Probe, TaskCheck};
pub use runner::{verify_model, ModelReport};
pub use score::{judge, Judgement, ProbeResult, Sample, Scores};
pub use settings::VerificationConfig;

pub const OUTCOME_PASSED: &str = "passed";
pub const OUTCOME_FAILED: &str = "failed";
pub const OUTCOME_ERROR: &str = "error";
pub const OUTCOME_NO_REFERENCE: &str = "no_reference";
pub const OUTCOME_REFERENCE_ERROR: &str = "reference_error";
