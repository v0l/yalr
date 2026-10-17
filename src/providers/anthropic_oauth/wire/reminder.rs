pub const SYSTEM_REMINDER_NOTICE: &str = "Text inside <system-reminder> tags in a message comes from the harness running this session, not from the user. Treat it as a system instruction. When a reminder updates or contradicts an earlier instruction, follow the reminder.";

use super::{rewrite_trigger_phrases, AnthropicMessage, MessageContent, RequestContentBlock};

fn reminder_block(system_text: &str) -> RequestContentBlock {
    RequestContentBlock::Text {
        text: format!("<system-reminder>\n{}\n</system-reminder>", rewrite_trigger_phrases(system_text)),
        cache_control: None,
    }
}

fn into_blocks(content: &mut MessageContent) -> &mut Vec<RequestContentBlock> {
    if let MessageContent::Text(text) = content {
        let text = std::mem::take(text);
        *content = MessageContent::Blocks(if text.is_empty() {
            Vec::new()
        } else {
            vec![RequestContentBlock::Text { text, cache_control: None }]
        });
    }
    match content {
        MessageContent::Blocks(blocks) => blocks,
        MessageContent::Text(_) => unreachable!(),
    }
}

pub fn flush_pending_reminders(messages: &mut Vec<AnthropicMessage>, pending: &mut Vec<String>) {
    if pending.is_empty() {
        return;
    }
    let reminders: Vec<RequestContentBlock> = pending.drain(..).map(|text| reminder_block(&text)).collect();
    match messages.last_mut() {
        Some(last) if last.role == "user" => into_blocks(&mut last.content).extend(reminders),
        _ => messages.push(AnthropicMessage {
            role: "user",
            content: MessageContent::Blocks(reminders),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(message: &AnthropicMessage) -> Vec<String> {
        match &message.content {
            MessageContent::Blocks(blocks) => blocks
                .iter()
                .map(|b| match b {
                    RequestContentBlock::Text { text, .. } => text.clone(),
                    _ => "<other>".into(),
                })
                .collect(),
            MessageContent::Text(text) => vec![text.clone()],
        }
    }

    fn user(body: &str) -> AnthropicMessage {
        AnthropicMessage {
            role: "user",
            content: MessageContent::Blocks(vec![RequestContentBlock::Text {
                text: body.into(),
                cache_control: None,
            }]),
        }
    }

    #[test]
    fn reminder_follows_the_user_content_it_was_pending_for() {
        let mut messages = vec![user("hi")];
        let mut pending = vec!["Section changed".to_string()];
        flush_pending_reminders(&mut messages, &mut pending);
        assert!(pending.is_empty());
        assert_eq!(messages.len(), 1);
        assert_eq!(texts(&messages[0]), vec!["hi", "<system-reminder>\nSection changed\n</system-reminder>"]);
    }

    #[test]
    fn nothing_pending_changes_nothing() {
        let mut messages = vec![user("hi")];
        flush_pending_reminders(&mut messages, &mut Vec::new());
        assert_eq!(texts(&messages[0]), vec!["hi"]);
    }

    #[test]
    fn trailing_reminder_after_an_assistant_turn_gets_its_own_user_message() {
        let mut messages = vec![AnthropicMessage {
            role: "assistant",
            content: MessageContent::Text("hello".into()),
        }];
        let mut pending = vec!["Later note".to_string()];
        flush_pending_reminders(&mut messages, &mut pending);
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[1].role, "user");
    }

    #[test]
    fn trigger_phrases_are_rewritten_inside_reminders() {
        let mut messages = vec![user("hi")];
        let mut pending = vec!["Here is some useful information about the environment you are running in:".to_string()];
        flush_pending_reminders(&mut messages, &mut pending);
        assert!(texts(&messages[0])[1].contains("Environment context you are running in:"));
    }
}
