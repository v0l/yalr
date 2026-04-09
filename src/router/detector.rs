use crate::providers::Provider;
use crate::router::{ModelDiscrepancy, ModelRuntimeInfo, ModelSyncReport, DiscrepancySeverity};
use std::collections::HashMap;
use std::sync::Arc;

pub struct ModelInfoDetector {
    providers: Vec<Arc<dyn Provider>>,
}

impl ModelInfoDetector {
    pub fn new(providers: Vec<Arc<dyn Provider>>) -> Self {
        Self { providers }
    }

    pub async fn detect_discrepancies(
        &self,
        db_model_info: &HashMap<String, DbModelInfo>,
    ) -> Vec<ModelSyncReport> {
        let mut reports = Vec::new();

        for provider in &self.providers {
            let provider_name = provider.name().to_string();
            
            match provider.list_models().await {
                Ok(models) => {
                    for model in models {
                        let model_id = model.id.clone();
                        
                        if let Some(db_info) = db_model_info.get(&model_id) {
                            match provider.get_runtime_info(&model_id).await {
                                Ok(Some(runtime_info)) => {
                                    let discrepancies = 
                                        self.compare_model_info(&model_id, &provider_name, db_info, &runtime_info);
                                    
                                    reports.push(ModelSyncReport {
                                        model_name: model_id,
                                        provider_name: provider_name.clone(),
                                        discrepancies,
                                        is_synced: true,
                                    });
                                }
                                Ok(None) => {
                                    tracing::debug!(
                                        model = &model_id,
                                        provider = &provider_name,
                                        "Provider does not support runtime info for this model"
                                    );
                                }
                                Err(e) => {
                                    tracing::warn!(
                                        model = &model_id,
                                        provider = &provider_name,
                                        error = %e,
                                        "Failed to fetch runtime model info"
                                    );
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        provider = &provider_name,
                        error = %e,
                        "Failed to list models from provider"
                    );
                }
            }
        }

        reports
    }

    fn compare_model_info(
        &self,
        model_id: &str,
        provider_name: &str,
        db_info: &DbModelInfo,
        runtime_info: &ModelRuntimeInfo,
    ) -> Vec<ModelDiscrepancy> {
        let mut discrepancies = Vec::new();

        if let Some(db_ctx) = db_info.context_window {
            if let Some(runtime_ctx) = runtime_info.context_length() {
                if db_ctx != runtime_ctx {
                    discrepancies.push(ModelDiscrepancy {
                        model_name: model_id.to_string(),
                        provider_name: provider_name.to_string(),
                        field: "context_window".to_string(),
                        database_value: Some(db_ctx.to_string()),
                        api_value: Some(runtime_ctx.to_string()),
                        severity: DiscrepancySeverity::Warning,
                    });
                }
            }
        }

        if let Some(db_quant) = &db_info.quantization {
            if let Some(runtime_quant) = runtime_info.quantization() {
                if db_quant != runtime_quant {
                    discrepancies.push(ModelDiscrepancy {
                        model_name: model_id.to_string(),
                        provider_name: provider_name.to_string(),
                        field: "quantization".to_string(),
                        database_value: Some(db_quant.clone()),
                        api_value: Some(runtime_quant.to_string()),
                        severity: DiscrepancySeverity::Info,
                    });
                }
            }
        }

        if let Some(db_max_out) = db_info.max_output_tokens {
            if let Some(runtime_max_out) = runtime_info.max_output_tokens {
                if db_max_out != runtime_max_out {
                    discrepancies.push(ModelDiscrepancy {
                        model_name: model_id.to_string(),
                        provider_name: provider_name.to_string(),
                        field: "max_output_tokens".to_string(),
                        database_value: Some(db_max_out.to_string()),
                        api_value: Some(runtime_max_out.to_string()),
                        severity: DiscrepancySeverity::Warning,
                    });
                }
            }
        }

        if let Some(db_params) = &db_info.parameter_size {
            if let Some(runtime_params) = runtime_info.parameter_size() {
                if db_params != runtime_params {
                    discrepancies.push(ModelDiscrepancy {
                        model_name: model_id.to_string(),
                        provider_name: provider_name.to_string(),
                        field: "parameter_size".to_string(),
                        database_value: Some(db_params.clone()),
                        api_value: Some(runtime_params.to_string()),
                        severity: DiscrepancySeverity::Info,
                    });
                }
            }
        }

        discrepancies
    }
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct DbModelInfo {
    pub context_window: Option<u32>,
    pub quantization: Option<String>,
    pub variant: Option<String>,
    pub parameter_size: Option<String>,
    pub max_output_tokens: Option<u32>,
}
