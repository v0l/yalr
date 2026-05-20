//! Payment instructions for top-ups and refunds.
//!
//! Provides a tagged enum type for different payment methods that the admin UI
//! can display/redirect based on the instruction type.

use serde::{Deserialize, Serialize};

/// Payment instruction types that the admin UI can handle.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PaymentInstruction {
    /// Lightning Bolt11 invoice payment.
    /// The UI should display the Bolt11 string and a QR code for payment.
    LightningBolt11 {
        /// The Bolt11 invoice string.
        bolt11: String,
        /// Payment hash for tracking.
        payment_hash: String,
        /// Amount in satoshis.
        amount_sats: i64,
        /// Amount in millisatoshis.
        amount_msat: i64,
        /// Optional memo/description.
        memo: Option<String>,
        /// Unix timestamp when the invoice expires.
        expires_at: Option<i64>,
        /// Optional invoice ID for internal tracking.
        invoice_id: Option<i64>,
    },

    /// HTTP redirect to an external payment page.
    /// The UI should redirect the user to this URL for payment.
    Redirect {
        /// URL to redirect to for payment.
        url: String,
        /// Optional amount in USD.
        amount_usd: Option<f64>,
        /// Optional session/token for tracking.
        session_token: Option<String>,
    },

    /// Manual payment instructions (e.g., bank transfer, wire).
    /// The UI should display the provided instructions text.
    Manual {
        /// Human-readable payment instructions.
        instructions: String,
        /// Optional amount to pay.
        amount_usd: Option<f64>,
        /// Optional reference code for the payment.
        reference_code: Option<String>,
    },

    /// Payment link (clickable URL, not a redirect).
    /// The UI should display a button/link that the user clicks to pay.
    PaymentLink {
        /// URL to open for payment.
        url: String,
        /// Optional amount in USD.
        amount_usd: Option<f64>,
        /// Optional label for the button/link.
        label: Option<String>,
    },
}

/// Response from a topup request containing payment instructions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopupResponse {
    /// Provider information.
    pub provider: ProviderInfo,
    /// The payment instruction for the user to complete the topup.
    pub instruction: PaymentInstruction,
    /// Optional message to display to the user.
    pub message: Option<String>,
}

/// Provider information included in topup responses.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderInfo {
    /// Provider slug/identifier.
    pub slug: String,
    /// Provider display name.
    pub name: String,
}

/// Request to create a topup invoice.
#[derive(Debug, Clone, Deserialize)]
pub struct TopupRequest {
    /// Amount to top up.
    pub amount: i64,
    /// Currency unit for the amount.
    pub currency: CurrencyType,
    /// Optional: specific instruction type preference.
    #[serde(default)]
    pub preferred_method: Option<PaymentMethodPreference>,
}

/// Currency type for topup amounts.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum CurrencyType {
    Sats,
    Msats,
    UsdMicro,
}

/// Preferred payment method for topup.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum PaymentMethodPreference {
    Lightning,
    Redirect,
    Manual,
    PaymentLink,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lightning_instruction_serialization() {
        let instruction = PaymentInstruction::LightningBolt11 {
            bolt11: "lnbc100n1...".to_string(),
            payment_hash: "abc123".to_string(),
            amount_sats: 1000,
            amount_msat: 1_000_000,
            memo: Some("Top-up".to_string()),
            expires_at: Some(1234567890),
            invoice_id: Some(1),
        };

        let json = serde_json::to_string_pretty(&instruction).unwrap();
        assert!(json.contains(r#""type": "lightning_bolt11""#));
        assert!(json.contains(r#""bolt11": "lnbc100n1...""#));
        assert!(json.contains(r#""amount_sats": 1000"#));
    }

    #[test]
    fn test_redirect_instruction_serialization() {
        let instruction = PaymentInstruction::Redirect {
            url: "https://example.com/pay".to_string(),
            amount_usd: Some(10.0),
            session_token: Some("token123".to_string()),
        };

        let json = serde_json::to_string(&instruction).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["type"], "redirect");
        assert_eq!(parsed["url"], "https://example.com/pay");
        assert_eq!(parsed["amount_usd"], 10.0);
    }

    #[test]
    fn test_manual_instruction_serialization() {
        let instruction = PaymentInstruction::Manual {
            instructions: "Send wire transfer to...".to_string(),
            amount_usd: Some(100.0),
            reference_code: Some("REF123".to_string()),
        };

        let json = serde_json::to_string(&instruction).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["type"], "manual");
        assert!(parsed["instructions"].as_str().unwrap().contains("wire transfer"));
    }

    #[test]
    fn test_topup_response_serialization() {
        let response = TopupResponse {
            provider: ProviderInfo {
                slug: "ppq".to_string(),
                name: "PPQ".to_string(),
            },
            instruction: PaymentInstruction::LightningBolt11 {
                bolt11: "lnbc100n1...".to_string(),
                payment_hash: "abc123".to_string(),
                amount_sats: 1000,
                amount_msat: 1_000_000,
                memo: None,
                expires_at: None,
                invoice_id: None,
            },
            message: Some("Pay this invoice to top up".to_string()),
        };

        let json = serde_json::to_string_pretty(&response).unwrap();
        assert!(json.contains(r#""slug": "ppq""#));
        assert!(json.contains(r#""type": "lightning_bolt11""#));
        assert!(json.contains(r#""message": "Pay this invoice to top up""#));
    }

    #[test]
    fn test_topup_request_deserialization() {
        let json = r#"{"amount": 1050000, "currency": "usd_micro", "preferred_method": "lightning"}"#;
        let request: TopupRequest = serde_json::from_str(json).unwrap();
        assert_eq!(request.amount, 1050000);
        assert_eq!(request.currency, CurrencyType::UsdMicro);
        assert_eq!(request.preferred_method, Some(PaymentMethodPreference::Lightning));
    }

    #[test]
    fn test_topup_request_default_preferred_method() {
        let json = r#"{"amount": 1000, "currency": "sats"}"#;
        let request: TopupRequest = serde_json::from_str(json).unwrap();
        assert_eq!(request.amount, 1000);
        assert_eq!(request.currency, CurrencyType::Sats);
        assert_eq!(request.preferred_method, None);
    }
}
