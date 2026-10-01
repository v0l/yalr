use crate::providers::StreamingChunk;

/// Returns true if a streaming chunk carries any meaningful assistant
/// output: non-empty text, non-empty reasoning content, or tool calls.
/// Role-only, usage-only, and finish-reason-only chunks return false.
pub(crate) fn chunk_has_content(chunk: &StreamingChunk) -> bool {
    chunk.choices.iter().any(|choice| {
        let delta = &choice.delta;
        let has_text = delta
            .content
            .as_deref()
            .map(|c| !c.is_empty())
            .unwrap_or(false);
        let has_reasoning = delta
            .reasoning_content
            .as_deref()
            .map(|c| !c.is_empty())
            .unwrap_or(false);
        // vLLM's Qwen reasoning parser (and some other OpenAI-compatible
        // backends) stream reasoning under the raw key `"reasoning"`
        // rather than the `reasoning_content` field async-openai expects.
        // It lands in `extra_fields` untouched (so it still passes
        // through to the client unchanged) but must also count as
        // content here, or a reasoning-only chunk gets misclassified as
        // empty and the whole stream fails over/errors even though the
        // provider is actively generating.
        let has_raw_reasoning = delta
            .extra_fields
            .get("reasoning")
            .and_then(|v| v.as_str())
            .map(|c| !c.is_empty())
            .unwrap_or(false);
        let has_tool_calls = delta
            .tool_calls
            .as_ref()
            .map(|t| !t.is_empty())
            .unwrap_or(false);
        let has_media = ["images", "audio"].iter().any(|key| {
            delta
                .extra_fields
                .get(*key)
                .is_some_and(|v| !v.is_null() && v.as_array().is_none_or(|a| !a.is_empty()))
        });
        has_text || has_reasoning || has_raw_reasoning || has_tool_calls || has_media
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(delta: serde_json::Value) -> StreamingChunk {
        serde_json::from_value(serde_json::json!({
            "id": "c", "object": "chat.completion.chunk", "created": 1, "model": "m",
            "choices": [{"index": 0, "delta": delta}]
        }))
        .unwrap()
    }

    #[test]
    fn role_only_chunk_is_empty() {
        assert!(!chunk_has_content(&chunk(serde_json::json!({"role": "assistant"}))));
        assert!(!chunk_has_content(&chunk(serde_json::json!({"content": "", "images": []}))));
    }

    #[test]
    fn text_and_raw_reasoning_count() {
        assert!(chunk_has_content(&chunk(serde_json::json!({"content": "hi"}))));
        assert!(chunk_has_content(&chunk(serde_json::json!({"reasoning": "hmm"}))));
    }

    #[test]
    fn image_or_audio_only_chunk_counts() {
        assert!(chunk_has_content(&chunk(serde_json::json!({
            "images": [{"type": "image_url", "image_url": {"url": "data:image/png;base64,AA"}}]
        }))));
        assert!(chunk_has_content(&chunk(serde_json::json!({
            "audio": {"id": "a", "data": "UklGRg=="}
        }))));
    }
}
