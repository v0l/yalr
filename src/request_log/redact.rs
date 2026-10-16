const SECRET_HEADERS: &[&str] = &[
    "authorization",
    "proxy-authorization",
    "x-api-key",
    "api-key",
    "cookie",
    "set-cookie",
];

pub fn mask(value: &str) -> String {
    let chars: Vec<char> = value.chars().collect();
    if chars.len() <= 16 {
        return format!("***({})", chars.len());
    }
    let head: String = chars[..12].iter().collect();
    let tail: String = chars[chars.len() - 4..].iter().collect();
    format!("{head}...{tail}({})", chars.len())
}

pub fn header_value(name: &str, value: &str) -> String {
    if SECRET_HEADERS.contains(&name.to_ascii_lowercase().as_str()) {
        mask(value)
    } else {
        value.to_string()
    }
}

pub fn headers(map: &reqwest::header::HeaderMap) -> Vec<(String, String)> {
    map.iter()
        .map(|(name, value)| {
            let text = value.to_str().unwrap_or("<binary>");
            (name.as_str().to_string(), header_value(name.as_str(), text))
        })
        .collect()
}

pub fn axum_headers(map: &axum::http::HeaderMap) -> Vec<(String, String)> {
    map.iter()
        .map(|(name, value)| {
            let text = value.to_str().unwrap_or("<binary>");
            (name.as_str().to_string(), header_value(name.as_str(), text))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrets_are_masked_and_the_rest_pass_through() {
        let masked = header_value("Authorization", "Bearer sk-ant-oat01-abcdefghijklmnop");
        assert!(!masked.contains("abcdefghijklmn"));
        assert!(masked.ends_with("mnop(36)"));
        assert_eq!(header_value("user-agent", "claude-cli/2.1.301"), "claude-cli/2.1.301");
    }

    #[test]
    fn short_secrets_reveal_nothing() {
        assert_eq!(mask("short"), "***(5)");
    }
}
