use rand::seq::SliceRandom;

#[derive(Debug, Clone, PartialEq)]
pub enum TaskCheck {
    Contains(&'static str),
    ToolCall { name: &'static str, argument: &'static str, value: &'static str },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Probe {
    pub id: String,
    pub prompt: String,
    pub fixed: bool,
    pub task: Option<TaskCheck>,
}

const FIXED: &[(&str, &str, Option<TaskCheck>)] = &[
    ("math-17x23", "What is 17 times 23? Answer with only the number.", Some(TaskCheck::Contains("391"))),
    ("capital-france", "What is the capital of France? Answer with one word.", Some(TaskCheck::Contains("paris"))),
    (
        "tool-call",
        "Reply with only a JSON object naming the tool get_weather with the argument city set to Paris, in the form {\"name\": \"get_weather\", \"arguments\": {\"city\": \"Paris\"}}.",
        Some(TaskCheck::ToolCall { name: "get_weather", argument: "city", value: "Paris" }),
    ),
    ("symbol-gold", "What is the chemical symbol for gold? Answer with only the symbol.", Some(TaskCheck::Contains("au"))),
    ("reverse-router", "Spell the word router backwards. Answer with only the word.", Some(TaskCheck::Contains("retuor"))),
    ("fox", "Complete the sentence: The quick brown fox", None),
    ("count", "Write the numbers one to ten separated by commas.", None),
    ("french", "Translate to French: Good morning, how are you today?", None),
];

const POOL: &[(&str, &str, &str)] = &[
    ("p-math-1", "What is 48 plus 79? Answer with only the number.", "127"),
    ("p-math-2", "What is 12 times 12? Answer with only the number.", "144"),
    ("p-math-3", "What is 1000 minus 357? Answer with only the number.", "643"),
    ("p-math-4", "What is 9 times 8? Answer with only the number.", "72"),
    ("p-math-5", "What is 256 divided by 8? Answer with only the number.", "32"),
    ("p-math-6", "What is 63 plus 59? Answer with only the number.", "122"),
    ("p-math-7", "What is 14 times 15? Answer with only the number.", "210"),
    ("p-math-8", "What is 500 minus 123? Answer with only the number.", "377"),
    ("p-math-9", "What is 81 divided by 9? Answer with only the number.", "9"),
    ("p-math-10", "What is 33 plus 68? Answer with only the number.", "101"),
    ("p-math-11", "What is 7 times 13? Answer with only the number.", "91"),
    ("p-math-12", "What is 90 minus 47? Answer with only the number.", "43"),
    ("p-cap-1", "What is the capital of Japan? Answer with one word.", "tokyo"),
    ("p-cap-2", "What is the capital of Italy? Answer with one word.", "rome"),
    ("p-cap-3", "What is the capital of Canada? Answer with one word.", "ottawa"),
    ("p-cap-4", "What is the capital of Egypt? Answer with one word.", "cairo"),
    ("p-cap-5", "What is the capital of Spain? Answer with one word.", "madrid"),
    ("p-cap-6", "What is the capital of Norway? Answer with one word.", "oslo"),
    ("p-sym-1", "What is the chemical symbol for sodium? Answer with only the symbol.", "na"),
    ("p-sym-2", "What is the chemical symbol for iron? Answer with only the symbol.", "fe"),
    ("p-sym-3", "What is the chemical symbol for potassium? Answer with only the symbol.", "k"),
    ("p-sym-4", "What is the chemical symbol for silver? Answer with only the symbol.", "ag"),
    ("p-rev-1", "Spell the word model backwards. Answer with only the word.", "ledom"),
    ("p-rev-2", "Spell the word token backwards. Answer with only the word.", "nekot"),
];

pub fn fixed_probes() -> Vec<Probe> {
    FIXED
        .iter()
        .map(|(id, prompt, task)| Probe {
            id: id.to_string(),
            prompt: prompt.to_string(),
            fixed: true,
            task: task.clone(),
        })
        .collect()
}

pub fn draw_probes(random: usize) -> Vec<Probe> {
    let mut pool: Vec<&(&str, &str, &str)> = POOL.iter().collect();
    pool.shuffle(&mut rand::thread_rng());
    let mut probes = fixed_probes();
    probes.extend(pool.into_iter().take(random).map(|(id, prompt, answer)| Probe {
        id: id.to_string(),
        prompt: prompt.to_string(),
        fixed: false,
        task: Some(TaskCheck::Contains(answer)),
    }));
    probes
}


fn words(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(str::to_lowercase).collect()
}

impl TaskCheck {
    pub fn passes(&self, output: &str) -> bool {
        match self {
            TaskCheck::Contains(expected) => words(output).iter().any(|w| w == expected),
            TaskCheck::ToolCall { name, argument, value } => {
                let (Some(start), Some(end)) = (output.find('{'), output.rfind('}')) else {
                    return false;
                };
                let Ok(parsed) = serde_json::from_str::<ToolCall>(&output[start..=end]) else {
                    return false;
                };
                parsed.name == *name
                    && parsed.arguments.get(*argument).and_then(|v| v.as_str()).is_some_and(|v| v.eq_ignore_ascii_case(value))
            }
        }
    }
}

#[derive(serde::Deserialize)]
struct ToolCall {
    name: String,
    arguments: std::collections::HashMap<String, serde_json::Value>,
}

#[cfg(test)]
pub fn expected_output(prompt: &str) -> Option<String> {
    if let Some((_, _, Some(TaskCheck::Contains(answer)))) = FIXED.iter().find(|(_, p, _)| *p == prompt) {
        return Some(answer.to_string());
    }
    if FIXED.iter().any(|(id, p, _)| *p == prompt && *id == "tool-call") {
        return Some(r#"{"name": "get_weather", "arguments": {"city": "Paris"}}"#.to_string());
    }
    POOL.iter().find(|(_, p, _)| *p == prompt).map(|(_, _, a)| a.to_string())
}
