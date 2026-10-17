struct TextReplacement {
    match_text: &'static str,
    replacement: &'static str,
}

const TEXT_REPLACEMENTS: &[TextReplacement] = &[TextReplacement {
    match_text: "Here is some useful information about the environment you are running in:",
    replacement: "Environment context you are running in:",
}];

pub fn rewrite_trigger_phrases(text: &str) -> String {
    TEXT_REPLACEMENTS
        .iter()
        .fold(text.to_string(), |acc, rule| acc.replace(rule.match_text, rule.replacement))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trigger_phrase_is_rewritten() {
        let text = "Here is some useful information about the environment you are running in:\nDetails.";
        assert_eq!(rewrite_trigger_phrases(text), "Environment context you are running in:\nDetails.");
    }

    #[test]
    fn other_text_is_untouched() {
        let text = "You are a helpful assistant.\n\nBe concise.";
        assert_eq!(rewrite_trigger_phrases(text), text);
    }

    #[test]
    fn rewriting_is_idempotent() {
        let once = rewrite_trigger_phrases("Here is some useful information about the environment you are running in:");
        assert_eq!(rewrite_trigger_phrases(&once), once);
    }
}
