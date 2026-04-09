use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum Modality {
    Text,
    Image,
    Audio,
    Video,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModelRuntimeInfo {
    pub model_id: String,
    pub context_length: Option<u32>,
    pub quantization: Option<String>,
    pub variant: Option<String>,
    pub parameter_size: Option<String>,
    pub max_output_tokens: Option<u32>,
    pub modalities: Vec<Modality>,
    pub additional_fields: std::collections::HashMap<String, serde_json::Value>,
}

impl ModelRuntimeInfo {
    pub fn from_api_response(
        model_id: &str,
        additional_fields: std::collections::HashMap<String, serde_json::Value>,
    ) -> Self {
        Self {
            model_id: model_id.to_string(),
            context_length: None,
            quantization: None,
            variant: None,
            parameter_size: None,
            max_output_tokens: None,
            modalities: vec![Modality::Text],
            additional_fields,
        }
    }

    pub fn supports_modality(&self, modality: Modality) -> bool {
        self.modalities.contains(&modality)
    }

    pub fn supports_image(&self) -> bool {
        self.modalities.contains(&Modality::Image)
    }

    pub fn supports_audio(&self) -> bool {
        self.modalities.contains(&Modality::Audio)
    }

    pub fn supports_video(&self) -> bool {
        self.modalities.contains(&Modality::Video)
    }

    pub fn context_length(&self) -> Option<u32> {
        self.context_length
            .or_else(|| {
                self.additional_fields
                    .get("context_length")
                    .and_then(|v| v.as_u64())
                    .map(|v| v as u32)
            })
            .or_else(|| {
                self.additional_fields
                    .get("max_context_tokens")
                    .and_then(|v| v.as_u64())
                    .map(|v| v as u32)
            })
    }

    pub fn quantization(&self) -> Option<&str> {
        self.quantization.as_deref().or_else(|| {
            self.additional_fields
                .get("quantization")
                .and_then(|v| v.as_str())
        })
    }

    pub fn parameter_size(&self) -> Option<&str> {
        self.parameter_size.as_deref().or_else(|| {
            self.additional_fields
                .get("parameter_size")
                .and_then(|v| v.as_str())
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelDiscrepancy {
    pub model_name: String,
    pub provider_name: String,
    pub field: String,
    pub database_value: Option<String>,
    pub api_value: Option<String>,
    pub severity: DiscrepancySeverity,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum DiscrepancySeverity {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelSyncReport {
    pub model_name: String,
    pub provider_name: String,
    pub discrepancies: Vec<ModelDiscrepancy>,
    pub is_synced: bool,
}
