use serde::Deserialize;

use super::AnthropicOAuthProvider;
use crate::providers::quota::PrepaidBalance;

#[derive(Debug, Deserialize)]
struct ProfileResponse {
    organization: ProfileOrganization,
}

#[derive(Debug, Deserialize)]
struct ProfileOrganization {
    uuid: String,
}

#[derive(Debug, Deserialize)]
struct PrepaidCreditsResponse {
    #[serde(default)]
    amount: Option<i64>,
    #[serde(default)]
    currency: Option<String>,
    #[serde(default)]
    balance: Option<PrepaidBalanceBody>,
}

#[derive(Debug, Deserialize)]
struct PrepaidBalanceBody {
    #[serde(default)]
    money: Option<Money>,
}

#[derive(Debug, Deserialize)]
struct Money {
    amount_minor: i64,
    currency: String,
    #[serde(default)]
    exponent: Option<u32>,
}

fn parse_prepaid_credits(body: &str) -> Option<PrepaidBalance> {
    let resp: PrepaidCreditsResponse = serde_json::from_str(body).ok()?;
    if let Some(money) = resp.balance.and_then(|b| b.money) {
        return Some(PrepaidBalance {
            amount_minor: money.amount_minor,
            currency: money.currency,
            decimals: money.exponent.unwrap_or(2),
        });
    }
    Some(PrepaidBalance {
        amount_minor: resp.amount?,
        currency: resp.currency?,
        decimals: 2,
    })
}

impl AnthropicOAuthProvider {
    fn api_root(&self) -> &str {
        self.base_url
            .trim_end_matches('/')
            .trim_end_matches("/v1")
            .trim_end_matches('/')
    }

    async fn organization_uuid(&self, token: &str) -> Option<String> {
        if let Some(uuid) = self.org_uuid.get() {
            return Some(uuid.clone());
        }
        let resp = self
            .apply_oauth_headers(
                self.http.get(format!("{}/api/oauth/profile", self.api_root())),
                token,
            )
            .send()
            .await
            .ok()?;
        if !resp.status().is_success() {
            return None;
        }
        let profile: ProfileResponse = resp.json().await.ok()?;
        let _ = self.org_uuid.set(profile.organization.uuid.clone());
        Some(profile.organization.uuid)
    }

    pub(super) async fn prepaid_balance(&self, token: &str) -> Option<PrepaidBalance> {
        let org = self.organization_uuid(token).await?;
        let url = format!(
            "{}/api/oauth/organizations/{}/prepaid/credits",
            self.api_root(),
            org
        );
        let resp = self
            .apply_oauth_headers(self.http.get(url), token)
            .send()
            .await
            .ok()?;
        if !resp.status().is_success() {
            return None;
        }
        parse_prepaid_credits(&resp.text().await.ok()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_money_balance() {
        let body = r#"{"amount":476,"currency":"EUR","balance":{"money":{"amount_minor":476,"currency":"EUR","exponent":2},"credits":null},"tranches":[]}"#;
        let b = parse_prepaid_credits(body).unwrap();
        assert_eq!((b.amount_minor, b.currency.as_str(), b.decimals), (476, "EUR", 2));
    }

    #[test]
    fn falls_back_to_top_level_amount() {
        let b = parse_prepaid_credits(r#"{"amount":1200,"currency":"USD","balance":null}"#).unwrap();
        assert_eq!((b.amount_minor, b.currency.as_str()), (1200, "USD"));
    }

    #[test]
    fn missing_amount_is_none() {
        assert!(parse_prepaid_credits(r#"{"balance":null}"#).is_none());
        assert!(parse_prepaid_credits("nope").is_none());
    }
}
