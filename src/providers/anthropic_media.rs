use async_anthropic::types::ContentSource;
use async_openai::types::chat::ChatCompletionRequestUserMessageContentPart as Part;
use base64::Engine;

use super::{ChatCompletionRequestMessage, ChatCompletionRequestUserMessageContent, ProviderError};

#[derive(Debug, Clone, PartialEq)]
pub enum MediaBlock {
    Image(ContentSource),
    Document(ContentSource),
}

#[derive(serde::Deserialize)]
struct FileFields {
    #[serde(default)]
    file_data: Option<String>,
    #[serde(default)]
    file_id: Option<String>,
    #[serde(default)]
    filename: Option<String>,
}

fn split_data_url(url: &str) -> Option<(&str, &str)> {
    let rest = url.strip_prefix("data:")?;
    let (header, data) = rest.split_once(',')?;
    let media_type = header.strip_suffix(";base64")?;
    Some((media_type, data))
}

fn image_source(url: &str) -> Result<ContentSource, String> {
    if url.starts_with("http://") || url.starts_with("https://") {
        return Ok(ContentSource::Url { url: url.to_string() });
    }
    let (media_type, data) =
        split_data_url(url).ok_or_else(|| "Image must be an http(s) URL or a base64 data URL".to_string())?;
    Ok(ContentSource::Base64 {
        media_type: media_type.to_string(),
        data: data.to_string(),
    })
}

fn document_source(fields: FileFields) -> Result<ContentSource, String> {
    if fields.file_id.is_some() && fields.file_data.is_none() {
        return Err("Anthropic cannot read OpenAI file ids, send file_data instead".into());
    }
    let raw = fields.file_data.ok_or("File part has no file_data")?;
    let (media_type, data) = match split_data_url(&raw) {
        Some((media_type, data)) => (media_type.to_string(), data.to_string()),
        None if fields.filename.as_deref().is_some_and(|n| n.to_ascii_lowercase().ends_with(".pdf")) => {
            ("application/pdf".to_string(), raw)
        }
        None => return Err("File data must be a base64 data URL".into()),
    };
    if media_type == "application/pdf" {
        return Ok(ContentSource::Base64 { media_type, data });
    }
    if media_type.starts_with("text/") {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(data.as_bytes())
            .map_err(|e| format!("File data is not valid base64: {e}"))?;
        let text = String::from_utf8(bytes).map_err(|_| "Text file is not UTF-8".to_string())?;
        return Ok(ContentSource::Text {
            media_type: "text/plain".to_string(),
            data: text,
        });
    }
    Err(format!("Anthropic does not accept {media_type} documents"))
}

pub fn media_block(part: &Part) -> Result<Option<MediaBlock>, String> {
    match part {
        Part::Text(_) => Ok(None),
        Part::ImageUrl(image) => image_source(&image.image_url.url).map(|s| Some(MediaBlock::Image(s))),
        Part::InputAudio(_) => Err("Anthropic does not accept audio input".into()),
        Part::File(file) => {
            let fields: FileFields = serde_json::to_value(&file.file)
                .and_then(serde_json::from_value)
                .map_err(|e| e.to_string())?;
            document_source(fields).map(|s| Some(MediaBlock::Document(s)))
        }
    }
}

pub fn check_media(messages: &[ChatCompletionRequestMessage]) -> Result<(), ProviderError> {
    for message in messages {
        let ChatCompletionRequestMessage::User(user) = message else {
            continue;
        };
        let ChatCompletionRequestUserMessageContent::Array(parts) = &user.content else {
            continue;
        };
        for part in parts {
            media_block(part).map_err(ProviderError::Unsupported)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn part(json: &str) -> Part {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn data_url_image_becomes_base64_source() {
        let block = media_block(&part(
            r#"{"type":"image_url","image_url":{"url":"data:image/png;base64,iVBORw0K"}}"#,
        ))
        .unwrap();
        assert_eq!(
            block,
            Some(MediaBlock::Image(ContentSource::Base64 {
                media_type: "image/png".into(),
                data: "iVBORw0K".into()
            }))
        );
    }

    #[test]
    fn remote_image_becomes_url_source() {
        let block = media_block(&part(
            r#"{"type":"image_url","image_url":{"url":"https://example.com/cat.jpg"}}"#,
        ))
        .unwrap();
        assert_eq!(
            block,
            Some(MediaBlock::Image(ContentSource::Url { url: "https://example.com/cat.jpg".into() }))
        );
    }

    #[test]
    fn pdf_file_becomes_document() {
        let block = media_block(&part(
            r#"{"type":"file","file":{"filename":"a.pdf","file_data":"data:application/pdf;base64,JVBERi0="}}"#,
        ))
        .unwrap();
        assert!(matches!(
            block,
            Some(MediaBlock::Document(ContentSource::Base64 { ref media_type, .. })) if media_type == "application/pdf"
        ));
    }

    #[test]
    fn text_file_is_decoded_into_a_text_document() {
        let block = media_block(&part(
            r#"{"type":"file","file":{"filename":"a.txt","file_data":"data:text/plain;base64,aGVsbG8="}}"#,
        ))
        .unwrap();
        assert_eq!(
            block,
            Some(MediaBlock::Document(ContentSource::Text {
                media_type: "text/plain".into(),
                data: "hello".into()
            }))
        );
    }

    #[test]
    fn audio_and_file_ids_are_unsupported() {
        assert!(media_block(&part(
            r#"{"type":"input_audio","input_audio":{"data":"AA","format":"wav"}}"#
        ))
        .is_err());
        assert!(media_block(&part(r#"{"type":"file","file":{"file_id":"file-abc"}}"#)).is_err());
    }

    #[test]
    fn check_media_rejects_a_request_with_audio() {
        let request: crate::providers::ChatRequest = serde_json::from_str(
            r#"{"model":"claude","messages":[{"role":"user","content":[{"type":"input_audio","input_audio":{"data":"AA","format":"wav"}}]}]}"#,
        )
        .unwrap();
        assert!(matches!(check_media(&request.messages), Err(ProviderError::Unsupported(_))));
    }
}
