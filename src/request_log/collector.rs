use crate::providers::StreamingChunk;
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Default)]
pub struct Collector {
    text: String,
    reasoning: String,
    tools: BTreeMap<u32, ToolCall>,
    finish_reason: Option<String>,
    id: Option<String>,
    model: Option<String>,
    raw: Option<Vec<u8>>,
}

#[derive(Default, Serialize)]
struct ToolCall {
    id: Option<String>,
    name: Option<String>,
    arguments: String,
}

#[derive(Serialize)]
struct Assembled<'a> {
    id: &'a Option<String>,
    model: &'a Option<String>,
    finish_reason: &'a Option<String>,
    reasoning: &'a str,
    text: &'a str,
    tool_calls: Vec<&'a ToolCall>,
}

impl Collector {
    pub fn push(&mut self, chunk: &StreamingChunk) {
        self.id.get_or_insert_with(|| chunk.id.clone());
        self.model.get_or_insert_with(|| chunk.model.clone());
        for choice in &chunk.choices {
            if let Some(text) = &choice.delta.content {
                self.text.push_str(text);
            }
            if let Some(reasoning) = &choice.delta.reasoning_content {
                self.reasoning.push_str(reasoning);
            }
            if let Some(calls) = &choice.delta.tool_calls {
                for call in calls {
                    let entry = self.tools.entry(call.index).or_default();
                    if let Some(id) = &call.id {
                        entry.id = Some(id.clone());
                    }
                    if let Some(function) = &call.function {
                        if let Some(name) = &function.name {
                            entry.name = Some(name.clone());
                        }
                        if let Some(arguments) = &function.arguments {
                            entry.arguments.push_str(arguments);
                        }
                    }
                }
            }
            if let Some(reason) = &choice.finish_reason {
                self.finish_reason = Some(reason.clone());
            }
        }
    }

    pub fn set_raw(&mut self, raw: Vec<u8>, finish_reason: Option<String>) {
        self.raw = Some(raw);
        self.finish_reason = finish_reason;
    }

    pub fn finish_reason(&self) -> Option<String> {
        self.finish_reason.clone()
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty() && self.reasoning.is_empty() && self.tools.is_empty()
    }

    pub fn assembled(&self) -> Vec<u8> {
        if let Some(raw) = &self.raw {
            return raw.clone();
        }
        serde_json::to_vec(&Assembled {
            id: &self.id,
            model: &self.model,
            finish_reason: &self.finish_reason,
            reasoning: &self.reasoning,
            text: &self.text,
            tool_calls: self.tools.values().collect(),
        })
        .unwrap_or_default()
    }
}
