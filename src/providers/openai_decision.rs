use super::decision::{DecisionRequest, DecisionResponse};
use super::openai_audio::{http_error, map_reqwest_error, retry_after_secs};
use super::{OpenAiProvider, ProviderError};

impl OpenAiProvider {
    pub(crate) async fn systemone(
        &self,
        request: &DecisionRequest,
    ) -> Result<DecisionResponse, ProviderError> {
        let url = format!("{}/systemone", self.base_url);

        let mut req = self.http_client.post(&url).json(request);
        if !self.api_key.is_empty() {
            req = req.bearer_auth(&self.api_key);
        }

        let response = req.send().await.map_err(map_reqwest_error)?;
        let status = response.status();
        let retry_after = retry_after_secs(response.headers());
        let body = response.bytes().await.map_err(map_reqwest_error)?;

        if !status.is_success() {
            return Err(http_error(
                status,
                retry_after,
                String::from_utf8_lossy(&body).to_string(),
            ));
        }

        DecisionResponse::parse(body).map_err(ProviderError::Unsupported)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{body_string_contains, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const ANSWER: &str = r#"{"model":"typesafe/jev-1.13-20260917","answers":{"team":{"type":"choice","choice":"payments","confidence":0.75,"probabilities":{"payments":0.84,"frontend":0.16}}},"usage":{"input_tokens":476,"output_tokens":70,"cost":0.00002}}"#;

    fn request() -> DecisionRequest {
        serde_json::from_str(
            r#"{"model":"typesafe/jev-1.13","state":{"ticket":"blank checkout"},"questions":{"team":{"type":"choice","instructions":"Which team?","criteria":{"payments":"Billing","frontend":"UI"}}}}"#,
        )
        .unwrap()
    }

    #[tokio::test]
    async fn posts_to_systemone_and_passes_body_through() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/systemone"))
            .and(header("authorization", "Bearer key"))
            .and(body_string_contains(r#""criteria":{"payments":"Billing","frontend":"UI"}"#))
            .respond_with(ResponseTemplate::new(200).set_body_raw(ANSWER, "application/json"))
            .mount(&server)
            .await;

        let provider = OpenAiProvider::new("t", None, &format!("{}/v1", server.uri()), Some("key"));
        let response = provider.systemone(&request()).await.unwrap();

        assert_eq!(&response.body[..], ANSWER.as_bytes());
        assert_eq!(response.usage().unwrap().input_tokens, 476);
    }

    #[tokio::test]
    async fn missing_route_is_unsupported() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/systemone"))
            .respond_with(ResponseTemplate::new(404).set_body_string("not found"))
            .mount(&server)
            .await;

        let provider = OpenAiProvider::new("t", None, &server.uri(), None);
        let err = provider.systemone(&request()).await.unwrap_err();
        assert!(matches!(err, ProviderError::Unsupported(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn non_decision_body_is_unsupported() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/systemone"))
            .respond_with(ResponseTemplate::new(200).set_body_raw("<html>ui</html>", "text/html"))
            .mount(&server)
            .await;

        let provider = OpenAiProvider::new("t", None, &server.uri(), None);
        let err = provider.systemone(&request()).await.unwrap_err();
        assert!(matches!(err, ProviderError::Unsupported(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn busy_worker_is_transient() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/systemone"))
            .respond_with(ResponseTemplate::new(503).set_body_string("busy"))
            .mount(&server)
            .await;

        let provider = OpenAiProvider::new("t", None, &server.uri(), None);
        let err = provider.systemone(&request()).await.unwrap_err();
        assert!(err.is_transient(), "got {err:?}");
    }

    #[tokio::test]
    async fn invalid_request_is_not_retried() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/systemone"))
            .respond_with(ResponseTemplate::new(422).set_body_string("too many questions"))
            .mount(&server)
            .await;

        let provider = OpenAiProvider::new("t", None, &server.uri(), None);
        let err = provider.systemone(&request()).await.unwrap_err();
        assert!(!err.is_transient());
        assert_eq!(err.status_code(), Some(422));
    }
}
