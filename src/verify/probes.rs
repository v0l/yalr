use rand::seq::SliceRandom;

#[derive(Debug, Clone, PartialEq)]
pub enum TaskCheck {
    Contains(&'static str),
    ToolCall { name: &'static str, argument: &'static str, value: &'static str },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Probe {
    pub id: String,
    pub system: Option<String>,
    pub prompt: String,
    pub fixed: bool,
    pub task: Option<TaskCheck>,
}

struct Spec {
    id: &'static str,
    system: Option<&'static str>,
    prompt: &'static str,
    task: Option<TaskCheck>,
}

const ASSISTANT: Option<&str> = Some("You are a helpful assistant.");

const fn spec(id: &'static str, system: Option<&'static str>, prompt: &'static str, task: Option<TaskCheck>) -> Spec {
    Spec { id, system, prompt, task }
}

const FIXED: &[Spec] = &[
    spec("math-17x23", None, "What is 17 times 23? Answer with only the number.", Some(TaskCheck::Contains("391"))),
    spec("capital-france", None, "What is the capital of France? Answer with one word.", Some(TaskCheck::Contains("paris"))),
    spec(
        "tool-call",
        None,
        "Reply with only a JSON object naming the tool get_weather with the argument city set to Paris, in the form {\"name\": \"get_weather\", \"arguments\": {\"city\": \"Paris\"}}.",
        Some(TaskCheck::ToolCall { name: "get_weather", argument: "city", value: "Paris" }),
    ),
    spec("symbol-gold", None, "What is the chemical symbol for gold? Answer with only the symbol.", Some(TaskCheck::Contains("au"))),
    spec("identity", ASSISTANT, "Which model are you, and which version? Answer in one short sentence.", None),
    spec("cutoff", None, "What is your knowledge cutoff date? Answer with only a month and a year.", None),
    spec("latest-rust", None, "What is the latest Rust version you know of? Answer with only the version number.", None),
    spec("latest-qwen", None, "What is the newest Qwen model you know of? Answer with only the model name.", None),
    spec("introduce", ASSISTANT, "Introduce yourself in one sentence.", None),
    spec("persona-pirate", Some("You are Captain Pip, a pirate. Stay in character."), "Hello, who are you?", None),
    spec("joke-programmers", None, "Tell me a short joke about programmers.", None),
    spec("tip-sleep", None, "Give me one tip for sleeping better.", None),
    spec("slogan", None, "Write a slogan for a bicycle repair shop.", None),
];

const POOL: &[Spec] = &[
    spec("who-made", None, "Who created you? Answer in one short sentence.", None),
    spec("name-version", ASSISTANT, "State your exact model name and version number in one short sentence.", None),
    spec("predecessor", None, "What is the name of the model that came before you? Answer in one short sentence.", None),
    spec("metaphor", None, "Describe the internet using one metaphor, in one sentence.", None),
    spec("pet-name", None, "Suggest a name for a grey cat and explain it in one sentence.", None),
    spec("latest-python", None, "What is the latest stable Python version you know of? Answer with only the version number.", None),
    spec(
        "latest-olympics",
        None,
        "Which city hosted the most recent Olympic Games you know of? Answer with only the city and year.",
        None,
    ),
    spec("knowledge-end", ASSISTANT, "When does your knowledge end? Reply with a month and year only.", None),
    spec("persona-robot", Some("You are UNIT-7, a terse maintenance robot. Stay in character."), "Status report, please.", None),
    spec("math-1", None, "What is 48 plus 79? Answer with only the number.", Some(TaskCheck::Contains("127"))),
    spec("math-2", None, "What is 12 times 12? Answer with only the number.", Some(TaskCheck::Contains("144"))),
    spec("math-3", None, "What is 1000 minus 357? Answer with only the number.", Some(TaskCheck::Contains("643"))),
    spec("math-4", None, "What is 9 times 8? Answer with only the number.", Some(TaskCheck::Contains("72"))),
    spec("math-5", None, "What is 256 divided by 8? Answer with only the number.", Some(TaskCheck::Contains("32"))),
    spec("math-6", None, "What is 63 plus 59? Answer with only the number.", Some(TaskCheck::Contains("122"))),
    spec("math-7", None, "What is 14 times 15? Answer with only the number.", Some(TaskCheck::Contains("210"))),
    spec("math-8", None, "What is 500 minus 123? Answer with only the number.", Some(TaskCheck::Contains("377"))),
    spec("math-9", None, "What is 81 divided by 9? Answer with only the number.", Some(TaskCheck::Contains("9"))),
    spec("math-10", None, "What is 33 plus 68? Answer with only the number.", Some(TaskCheck::Contains("101"))),
    spec("math-11", None, "What is 7 times 13? Answer with only the number.", Some(TaskCheck::Contains("91"))),
    spec("math-12", None, "What is 90 minus 47? Answer with only the number.", Some(TaskCheck::Contains("43"))),
    spec("capital-1", None, "What is the capital of Japan? Answer with one word.", Some(TaskCheck::Contains("tokyo"))),
    spec("capital-2", None, "What is the capital of Italy? Answer with one word.", Some(TaskCheck::Contains("rome"))),
    spec("capital-3", None, "What is the capital of Canada? Answer with one word.", Some(TaskCheck::Contains("ottawa"))),
    spec("capital-4", None, "What is the capital of Egypt? Answer with one word.", Some(TaskCheck::Contains("cairo"))),
    spec("capital-5", None, "What is the capital of Spain? Answer with one word.", Some(TaskCheck::Contains("madrid"))),
    spec("capital-6", None, "What is the capital of Norway? Answer with one word.", Some(TaskCheck::Contains("oslo"))),
    spec("symbol-1", None, "What is the chemical symbol for sodium? Answer with only the symbol.", Some(TaskCheck::Contains("na"))),
    spec("symbol-2", None, "What is the chemical symbol for iron? Answer with only the symbol.", Some(TaskCheck::Contains("fe"))),
    spec("symbol-3", None, "What is the chemical symbol for potassium? Answer with only the symbol.", Some(TaskCheck::Contains("k"))),
    spec("symbol-4", None, "What is the chemical symbol for silver? Answer with only the symbol.", Some(TaskCheck::Contains("ag"))),
    spec("symbol-5", None, "What is the chemical symbol for calcium? Answer with only the symbol.", Some(TaskCheck::Contains("ca"))),
    spec("symbol-6", None, "What is the chemical symbol for copper? Answer with only the symbol.", Some(TaskCheck::Contains("cu"))),
];

fn probe(spec: &Spec, fixed: bool) -> Probe {
    Probe {
        id: spec.id.to_string(),
        system: spec.system.map(str::to_string),
        prompt: spec.prompt.to_string(),
        fixed,
        task: spec.task.clone(),
    }
}

pub fn fixed_probes() -> Vec<Probe> {
    FIXED.iter().map(|s| probe(s, true)).collect()
}

pub fn pool_probes() -> Vec<Probe> {
    POOL.iter().map(|s| probe(s, false)).collect()
}

pub fn draw_probes(random: usize) -> Vec<Probe> {
    let mut pool = pool_probes();
    pool.shuffle(&mut rand::thread_rng());
    let mut probes = fixed_probes();
    probes.extend(pool.into_iter().take(random));
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
    let spec = FIXED.iter().chain(POOL).find(|s| s.prompt == prompt)?;
    match &spec.task {
        Some(TaskCheck::Contains(answer)) => Some(answer.to_string()),
        Some(TaskCheck::ToolCall { .. }) => Some(r#"{"name": "get_weather", "arguments": {"city": "Paris"}}"#.to_string()),
        None => None,
    }
}
