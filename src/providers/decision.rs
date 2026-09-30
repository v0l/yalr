use bytes::Bytes;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionRequest {
    pub model: String,
    pub state: Box<RawValue>,
    pub questions: IndexMap<String, DecisionQuestion>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum QuestionType {
    Choice,
    Noul,
    Score,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionQuestion {
    #[serde(rename = "type")]
    pub kind: QuestionType,
    pub instructions: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub criteria: Option<Box<RawValue>>,
}

impl DecisionRequest {
    pub fn validate(&self) -> Result<(), String> {
        if self.questions.is_empty() {
            return Err("`questions` must contain at least one question".into());
        }
        for (key, question) in &self.questions {
            if question.kind != QuestionType::Noul && question.criteria.is_none() {
                return Err(format!("question `{key}` needs `criteria`"));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct DecisionResponse {
    pub body: Bytes,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
pub struct DecisionUsage {
    #[serde(default)]
    pub input_tokens: u32,
    #[serde(default)]
    pub output_tokens: u32,
}

#[derive(Deserialize)]
struct DecisionEnvelope {
    #[allow(dead_code)]
    answers: IndexMap<String, Box<RawValue>>,
    #[serde(default)]
    usage: Option<DecisionUsage>,
}

impl DecisionResponse {
    pub fn parse(body: Bytes) -> Result<Self, String> {
        serde_json::from_slice::<DecisionEnvelope>(&body)
            .map_err(|e| format!("not a decision response: {e}"))?;
        Ok(Self { body })
    }

    pub fn usage(&self) -> Option<DecisionUsage> {
        serde_json::from_slice::<DecisionEnvelope>(&self.body)
            .ok()
            .and_then(|e| e.usage)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const REQUEST: &str = r#"{"model":"typesafe/jev-1.13","state":{"z":1,"a":2},"questions":{"team":{"type":"choice","instructions":"Which team?","criteria":{"zeta":"last","alpha":"first"}},"is_bug":{"type":"noul","instructions":"Bug?"}},"session_id":"s1"}"#;

    #[test]
    fn round_trip_keeps_criteria_and_state_order() {
        let parsed: DecisionRequest = serde_json::from_str(REQUEST).unwrap();
        let wire = serde_json::to_string(&parsed).unwrap();
        assert!(wire.contains(r#""state":{"z":1,"a":2}"#), "{wire}");
        assert!(wire.contains(r#"{"zeta":"last","alpha":"first"}"#), "{wire}");
        let keys: Vec<_> = parsed.questions.keys().cloned().collect();
        assert_eq!(keys, vec!["team", "is_bug"]);
    }

    #[test]
    fn router_only_fields_are_not_forwarded() {
        let parsed: DecisionRequest = serde_json::from_str(REQUEST).unwrap();
        let wire = serde_json::to_string(&parsed).unwrap();
        assert!(!wire.contains("session_id"));
        assert!(!wire.contains("image"));
    }

    #[test]
    fn string_state_is_accepted() {
        let parsed: DecisionRequest = serde_json::from_str(
            r#"{"model":"m","state":"refund me","questions":{"r":{"type":"noul","instructions":"Refund?"}}}"#,
        )
        .unwrap();
        assert_eq!(parsed.state.get(), r#""refund me""#);
        assert!(parsed.validate().is_ok());
    }

    #[test]
    fn choice_without_criteria_is_invalid() {
        let parsed: DecisionRequest = serde_json::from_str(
            r#"{"model":"m","state":"x","questions":{"t":{"type":"choice","instructions":"Which?"}}}"#,
        )
        .unwrap();
        assert!(parsed.validate().unwrap_err().contains("`t`"));
    }

    #[test]
    fn empty_questions_are_invalid() {
        let parsed: DecisionRequest =
            serde_json::from_str(r#"{"model":"m","state":"x","questions":{}}"#).unwrap();
        assert!(parsed.validate().is_err());
    }

    #[test]
    fn unknown_question_type_is_rejected() {
        assert!(serde_json::from_str::<DecisionRequest>(
            r#"{"model":"m","state":"x","questions":{"t":{"type":"rank","instructions":"?"}}}"#,
        )
        .is_err());
    }

    #[test]
    fn usage_read_from_systemone_body() {
        let response = DecisionResponse::parse(Bytes::from_static(
            br#"{"model":"jev","answers":{"r":{"type":"noul","noul":0.9}},"usage":{"input_tokens":476,"output_tokens":70,"cost":0.00002}}"#,
        ))
        .unwrap();
        assert_eq!(
            response.usage(),
            Some(DecisionUsage { input_tokens: 476, output_tokens: 70 })
        );
    }

    #[test]
    fn body_without_answers_is_not_a_decision() {
        assert!(DecisionResponse::parse(Bytes::from_static(b"<html></html>")).is_err());
        assert!(DecisionResponse::parse(Bytes::from_static(br#"{"model":"x"}"#)).is_err());
    }
}
