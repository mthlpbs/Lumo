//! Provider-neutral companion data and wire adapters. No timers or networking.
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Provider {
    #[default]
    OpenAI,
    Claude,
    Gemini,
    Compatible,
}
impl Provider {
    pub const ALL: [Self; 4] = [Self::OpenAI, Self::Claude, Self::Gemini, Self::Compatible];
    pub fn name(self) -> &'static str {
        match self {
            Self::OpenAI => "OpenAI",
            Self::Claude => "Claude",
            Self::Gemini => "Gemini",
            Self::Compatible => "OpenAI Compatible",
        }
    }
    pub fn completion(self) -> &'static str {
        match self {
            Self::OpenAI => "https://api.openai.com/v1/chat/completions",
            Self::Claude => "https://api.anthropic.com/v1/messages",
            Self::Gemini => {
                "https://generativelanguage.googleapis.com/v1beta/models/{model}:generateContent"
            }
            Self::Compatible => "",
        }
    }
    pub fn models(self) -> &'static str {
        match self {
            Self::OpenAI => "https://api.openai.com/v1/models",
            Self::Claude => "https://api.anthropic.com/v1/models",
            Self::Gemini => "https://generativelanguage.googleapis.com/v1beta/models",
            Self::Compatible => "",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub keep_open: bool,
    pub resume: bool,
    pub horizontal_scroll: bool,
    pub mini_width: u16,
    pub mini_height: u16,
    pub profiles: std::collections::BTreeMap<String, Endpoints>,
    pub provider: Provider,
    pub completion_url: String,
    pub models_url: String,
    pub default_model: String,
    pub reasoning_efforts: std::collections::BTreeMap<String, String>,
    pub save_models: bool,
    pub hide_unavailable_models: bool,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            keep_open: false,
            resume: false,
            horizontal_scroll: true,
            mini_width: 560,
            mini_height: 680,
            profiles: Default::default(),
            provider: Provider::default(),
            completion_url: String::new(),
            models_url: String::new(),
            default_model: String::new(),
            reasoning_efforts: Default::default(),
            save_models: true,
            hide_unavailable_models: true,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Endpoints {
    pub completion_url: String,
    pub models_url: String,
    pub default_model: String,
    #[serde(default)]
    pub reasoning_efforts: std::collections::BTreeMap<String, String>,
    #[serde(default = "save_models_default")]
    pub save_models: bool,
    #[serde(default = "save_models_default")]
    pub hide_unavailable_models: bool,
}
fn save_models_default() -> bool {
    true
}
impl Default for Endpoints {
    fn default() -> Self {
        Self {
            completion_url: String::new(),
            models_url: String::new(),
            default_model: String::new(),
            reasoning_efforts: Default::default(),
            save_models: true,
            hide_unavailable_models: true,
        }
    }
}
impl Config {
    pub fn switch(&mut self, p: Provider) {
        self.profiles.insert(
            self.provider.name().into(),
            Endpoints {
                completion_url: std::mem::take(&mut self.completion_url),
                models_url: std::mem::take(&mut self.models_url),
                default_model: std::mem::take(&mut self.default_model),
                reasoning_efforts: std::mem::take(&mut self.reasoning_efforts),
                save_models: self.save_models,
                hide_unavailable_models: self.hide_unavailable_models,
            },
        );
        let e = self.profiles.get(p.name()).cloned().unwrap_or_default();
        self.provider = p;
        self.completion_url = e.completion_url;
        self.models_url = e.models_url;
        self.default_model = e.default_model;
        self.reasoning_efforts = e.reasoning_efforts;
        self.save_models = e.save_models;
        self.hide_unavailable_models = e.hide_unavailable_models;
    }
    pub fn completion(&self) -> &str {
        if self.completion_url.trim().is_empty() {
            self.provider.completion()
        } else {
            self.completion_url.trim()
        }
    }
    pub fn models(&self) -> &str {
        if self.models_url.trim().is_empty() {
            self.provider.models()
        } else {
            self.models_url.trim()
        }
    }
    pub fn hackclub(&self) -> bool {
        is_hackclub(self.completion()) || is_hackclub(self.models())
    }
}
pub fn is_hackclub(url: &str) -> bool {
    url.strip_prefix("https://")
        .and_then(|s| s.split('/').next())
        .is_some_and(|h| h.eq_ignore_ascii_case("ai.hackclub.com") || h.eq_ignore_ascii_case("ai.hackclub.com:443"))
}
/// Never put a bearer key on an insecure remote endpoint.
pub fn validate_url(url: &str) -> Result<(), String> {
    let (scheme, rest) = url.split_once("://").ok_or("Enter a complete HTTPS URL")?;
    let host = rest.split('/').next().unwrap_or("");
    if host.is_empty() || host.contains('@') || url.contains(['\r', '\n', '#']) {
        return Err("Invalid endpoint URL".into());
    }
    if scheme == "https"
        || (scheme == "http"
            && (host == "localhost"
                || host.starts_with("localhost:")
                || host == "127.0.0.1"
                || host.starts_with("127.0.0.1:")))
    {
        Ok(())
    } else {
        Err("Use HTTPS, or HTTP on localhost".into())
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Kind {
    Language,
    Image,
    Embedding,
}
impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Self::Language => "Language",
            Self::Image => "Image",
            Self::Embedding => "Embedding",
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Model {
    pub id: String,
    pub name: String,
    pub provider: String,
    pub description: String,
    pub kind: Kind,
    pub free: Option<bool>,
    pub inputs: Vec<String>,
    pub outputs: Vec<String>,
    pub capabilities: Vec<String>,
    pub context: u64,
    pub input_price: Option<f64>,
    pub output_price: Option<f64>,
    #[serde(default)]
    pub reasoning: crate::ai_reasoning::Reasoning,
    /// Provider-supplied status only. Unknown availability is not a health check.
    #[serde(default)]
    pub unavailable: Option<String>,
}

fn catalog_unavailable(v: &Value) -> Option<String> {
    // A published retirement date is a deprecation announcement, including
    // future dates. Do not infer status from a model's name or description.
    for field in ["shutdown_date", "expiration_date"] {
        if let Some(date) = v[field].as_str().filter(|s| valid_model_date(s)) {
            return Some(format!("Deprecated · retirement date {date}"));
        }
    }
    if v["deprecated"].as_bool() == Some(true) {
        return Some("Deprecated by the provider".into());
    }
    if v["available"].as_bool() == Some(false) || v["enabled"].as_bool() == Some(false) {
        return Some("Unavailable in this provider's catalog".into());
    }
    match v["status"].as_str().unwrap_or("").to_ascii_lowercase().as_str() {
        "deprecated" | "retired" | "disabled" | "unavailable" | "decommissioned" =>
            Some("Deprecated or unavailable in this provider's catalog".into()),
        _ => None,
    }
}
fn valid_model_date(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-'
        || !b.iter().enumerate().all(|(i,c)| i == 4 || i == 7 || c.is_ascii_digit()) {
        return false;
    }
    let y: i32 = s[..4].parse().unwrap_or(0);
    let m: u8 = s[5..7].parse().unwrap_or(0);
    let d: u8 = s[8..].parse().unwrap_or(0);
    y >= 1970 && (1..=12).contains(&m) && d > 0 && d <= crate::datetime::days_in_month(y,m)
}
/// Only explicit model-specific refusals disable a model for this endpoint.
/// Authentication, capacity, rate limits and network errors remain retryable.
pub fn model_refused(status: u16, v: &Value, id: &str) -> bool {
    if id.is_empty() || !matches!(status, 400 | 404 | 410 | 422) { return false; }
    let error = &v["error"];
    let code = error["code"].as_str().or_else(|| error["type"].as_str()).unwrap_or("");
    if matches!(code, "model_not_found" | "model_not_available" | "model_deprecated" | "model_retired" | "model_disabled") {
        return true;
    }
    let message = error["message"].as_str().or_else(|| error.as_str()).unwrap_or("").to_ascii_lowercase();
    if ["this model has been deprecated", "this model has been retired", "this model is no longer available"].iter().any(|s| message.contains(s)) {
        return true;
    }
    // Gemini and some compatible services put the ID in their message only.
    let specific = message.contains(&id.to_ascii_lowercase());
    specific && (message.contains("model") || error["param"] == "model")
        && ["does not exist", "not found", "not available", "deprecated", "decommissioned", "retired", "no longer available", "not a valid model id", "not supported for generatecontent"]
            .iter().any(|s| message.contains(s))
}
fn strings(v: &Value) -> Vec<String> {
    v.as_array()
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .take(32)
                .map(|s| s.chars().take(64).collect())
                .collect()
        })
        .unwrap_or_default()
}
fn price(v: &Value) -> Option<f64> {
    v.as_f64()
        .or_else(|| v.as_str()?.parse().ok())
        .filter(|n| n.is_finite() && *n >= 0.0)
}
impl Model {
    pub fn provider_name(&self) -> &str {
        match self.provider.as_str() {
            "openai" | "OpenAI" => "OpenAI",
            "anthropic" | "Anthropic" | "Claude" => "Anthropic",
            "google" | "Google" => "Google",
            "liquid" | "LiquidAI" => "LiquidAI",
            "meta-llama" => "Meta",
            "mistralai" => "Mistral AI",
            "deepseek" => "DeepSeek",
            "x-ai" => "xAI",
            "moonshotai" => "Moonshot AI",
            "z-ai" => "Z.ai",
            "qwen" => "Qwen",
            _ => &self.provider,
        }
    }
    pub fn display_name(&self) -> &str {
        let name = self.name.trim_end_matches(" (free)");
        if let Some((prefix, rest)) = name.split_once(": ") {
            if prefix.eq_ignore_ascii_case(self.provider_name())
                || prefix.eq_ignore_ascii_case(&self.provider)
            {
                return rest;
            }
        }
        name
    }
    pub fn manual(id: &str) -> Self {
        Self {
            id: id.into(),
            name: id.into(),
            provider: "Custom".into(),
            description: "Capabilities were not supplied by this endpoint.".into(),
            kind: if id.to_ascii_lowercase().contains("embed") {
                Kind::Embedding
            } else if id.contains("gpt-image") || id.contains("-image") || id.contains("dall-e") {
                Kind::Image
            } else {
                Kind::Language
            },
            free: None,
            inputs: vec![],
            outputs: vec![],
            capabilities: vec![],
            context: 0,
            input_price: None,
            output_price: None,
            reasoning: Default::default(),
            unavailable: None,
        }
    }
    pub fn cost_group(&self) -> &'static str {
        match self.free {
            Some(true) => "Free",
            Some(false) => "Paid",
            None => "Price not supplied",
        }
    }
    pub fn details(&self) -> String {
        let mut s = format!(
            "{}\n{} · {} · {}\n{}",
            self.name,
            self.provider,
            self.kind.name(),
            self.cost_group(),
            self.description.chars().take(240).collect::<String>()
        );
        if !self.inputs.is_empty() {
            s += &format!("\nAccepts: {}", self.inputs.join(", "));
        }
        if !self.outputs.is_empty() {
            s += &format!("\nProduces: {}", self.outputs.join(", "));
        }
        if self.context > 0 {
            s += &format!("\nContext: {} tokens", self.context);
        }
        if let Some(p) = self.input_price {
            s += &format!("\nInput: ${:.3} / million tokens", p * 1e6);
        }
        if let Some(p) = self.output_price {
            s += &format!(" · Output: ${:.3} / million", p * 1e6);
        }
        if !self.capabilities.is_empty() {
            s += &format!(
                "\nOptions: {}",
                self.capabilities
                    .iter()
                    .take(8)
                    .map(|s| match s.as_str() {
                        "tools" | "tool_choice" => "Tool use".to_owned(),
                        "structured_outputs" | "response_format" => "Structured replies".to_owned(),
                        "reasoning" | "include_reasoning" | "reasoning_effort" =>
                            "Reasoning".to_owned(),
                        "temperature" | "top_p" => "Creativity control".to_owned(),
                        "max_tokens" => "Response length".to_owned(),
                        "generateContent" => "Chat & multimodal replies".to_owned(),
                        "embedContent" => "Search vectors".to_owned(),
                        _ => s.replace('_', " "),
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        s
    }
}
pub fn parse_models(value: &Value) -> Result<Vec<Model>, String> {
    let a = value
        .get("data")
        .or_else(|| value.get("models"))
        .unwrap_or(value)
        .as_array()
        .ok_or("The model list isn't an array")?;
    let mut models = Vec::new();
    for v in a.iter().take(5000) {
        let Some(id) = v
            .get("id")
            .or_else(|| v.get("name"))
            .and_then(Value::as_str)
        else {
            continue;
        };
        let id = id.strip_prefix("models/").unwrap_or(id);
        if id.len() > 512 || id.is_empty() {
            continue;
        }
        let mut m = Model::manual(id);
        m.unavailable = catalog_unavailable(v);
        m.name = v
            .get("displayName")
            .or_else(|| v.get("name"))
            .and_then(Value::as_str)
            .filter(|n| !n.starts_with("models/"))
            .unwrap_or(id)
            .chars()
            .take(120)
            .collect();
        m.provider = v
            .get("owned_by")
            .or_else(|| v.get("provider"))
            .and_then(Value::as_str)
            .unwrap_or_else(|| id.split_once('/').map_or("Other", |p| p.0))
            .chars()
            .take(64)
            .collect();
        m.description = v["description"]
            .as_str()
            .unwrap_or("This endpoint supplies no description.")
            .chars()
            .take(1800)
            .collect();
        m.inputs = strings(&v["architecture"]["input_modalities"]);
        m.outputs = strings(&v["architecture"]["output_modalities"]);
        m.capabilities = strings(&v["supported_parameters"]);
        m.capabilities
            .extend(strings(&v["supportedGenerationMethods"]));
        m.capabilities.sort();
        m.capabilities.dedup();
        m.reasoning = crate::ai_reasoning::Reasoning::parse(v).unwrap_or_default();
        m.context = v["context_length"]
            .as_u64()
            .or_else(|| v["inputTokenLimit"].as_u64())
            .unwrap_or(0);
        m.input_price = price(&v["pricing"]["prompt"]);
        m.output_price = price(&v["pricing"]["completion"]);
        m.free = match (m.input_price, m.output_price) {
            (Some(a), Some(b)) => Some(a == 0.0 && b == 0.0),
            _ if id.ends_with(":free") => Some(true),
            _ => None,
        };
        let lower = id.to_lowercase();
        m.kind = if lower.contains("embed") || v["type"] == "embedding" {
            Kind::Embedding
        } else if m.outputs.iter().any(|s| s == "image")
            || lower.contains("dall-e")
            || lower.contains("gpt-image")
            || lower.contains("imagen")
            || lower.contains("-image")
        {
            Kind::Image
        } else {
            Kind::Language
        };
        models.push(m);
    }
    models.sort_by(|a, b| {
        (&a.provider, a.kind, a.cost_group(), &a.name).cmp(&(
            &b.provider,
            b.kind,
            b.cost_group(),
            &b.name,
        ))
    });
    models.dedup_by(|a, b| a.id == b.id);
    if models.is_empty() {
        Err("No models were returned".into())
    } else {
        Ok(models)
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Attachment {
    pub name: String,
    pub mime: String,
    pub data: String,
}
impl Attachment {
    pub fn uri(&self) -> String {
        format!("data:{};base64,{}", self.mime, self.data)
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub text: String,
    #[serde(default)]
    pub attachments: Vec<Attachment>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Chat {
    pub id: u64,
    pub title: String,
    pub model: String,
    pub messages: Vec<Message>,
}
pub fn request(
    config: &Config,
    model: &Model,
    messages: &[Message],
) -> Result<(String, Value), String> {
    if let Some(reason) = &model.unavailable { return Err(reason.clone()); }
    if model.kind == Kind::Embedding {
        return Err(
            "Embedding models turn text into vectors. Select a language or image model to chat."
                .into(),
        );
    }
    if messages.is_empty() {
        return Err("Write a message first".into());
    }
    let url = config
        .completion()
        .replace("{model}", &crate::urlenc::encode(&model.id));
    validate_url(&url)?;
    let parts = |m: &Message| -> Result<Vec<Value>, String> {
        let mut p = vec![match config.provider {
            Provider::Gemini => json!({"text":m.text}),
            _ => json!({"type":"text","text":m.text}),
        }];
        // OpenAI-style assistant messages carry text. An image reused with the
        // composer's action is sent in the new user message, where vision input
        // is supported; do not duplicate output images in assistant content.
        if m.role == "assistant" && config.provider != Provider::Gemini {
            return Ok(p);
        }
        for a in &m.attachments {
            if a.mime == "text/plain" {
                let bytes = unbase64(&a.data).ok_or("Couldn't read the attached text file")?;
                let content = std::str::from_utf8(&bytes).map_err(|_| "Text attachments must use UTF-8")?;
                let content = format!("File: {}\n{}", a.name, content.trim_start_matches('\u{feff}'));
                p.push(match config.provider {
                    Provider::Gemini => json!({"text":content}),
                    _ => json!({"type":"text","text":content}),
                });
                continue;
            }
            p.push(match config.provider {
            Provider::Claude=>json!({"type":if a.mime=="application/pdf"{"document"}else{"image"},"source":{"type":"base64","media_type":a.mime,"data":a.data}}),
            Provider::Gemini=>json!({"inlineData":{"mimeType":a.mime,"data":a.data}}),
            _ if a.mime=="application/pdf"=>json!({"type":"file","file":{"filename":a.name,"file_data":a.uri()}}),
            _=>json!({"type":"image_url","image_url":{"url":a.uri()}}),
        });
        }
        Ok(p)
    };
    let ms: Vec<Value> = messages
        .iter()
        .map(|m| {
            let content = parts(m)?;
            Ok(match config.provider {
            Provider::Gemini => {
                json!({"role":if m.role=="assistant"{"model"}else{"user"},"parts":content})
            }
            _ => json!({"role":m.role,"content":content}),
            })
        })
        .collect::<Result<_, String>>()?;
    let mut body = match config.provider {
        Provider::Claude => json!({"model":model.id,"max_tokens":4096,"messages":ms}),
        Provider::Gemini => json!({"contents":ms,"generationConfig":{"maxOutputTokens":4096}}),
        _ => json!({"model":model.id,"messages":ms,"stream":false}),
    };
    let mut url = url;
    let mut reasoning = model.reasoning.clone();
    reasoning.resolve(&model.id, config.provider);
    if let Some(effort) = config
        .reasoning_efforts
        .get(&model.id)
        .filter(|e| reasoning.levels.contains(e))
    {
        match config.provider {
            Provider::Claude => body["output_config"] = json!({"effort":effort}),
            Provider::Gemini => {
                body["generationConfig"]["thinkingConfig"] =
                    json!({"thinkingLevel":effort.to_ascii_uppercase()})
            }
            Provider::Compatible if model.capabilities.iter().any(|c| c == "reasoning") => {
                body["reasoning"] = json!({"effort":effort})
            }
            _ => body["reasoning_effort"] = json!(effort),
        }
    }
    if model.kind == Kind::Image {
        match config.provider {
            Provider::Claude=>return Err("Claude can read images but does not generate them. Choose an image model from another provider.".into()),
            Provider::Gemini=>body["generationConfig"]["responseModalities"]=json!(["TEXT","IMAGE"]),
            Provider::OpenAI=>{
                let last=messages.last().unwrap();
                if last.attachments.iter().any(|a|a.mime=="application/pdf"){return Err("Image generation accepts image references. Choose a language model to read a PDF.".into());}
                let edit=!last.attachments.is_empty();
                url=url.strip_suffix("/chat/completions").ok_or("OpenAI image generation needs a /chat/completions endpoint")?.to_owned()+if edit{"/images/edits"}else{"/images/generations"};
                body=json!({"model":model.id,"prompt":last.text,"n":1});
                if edit{body["images"]=json!(last.attachments.iter().map(|a|json!({"image_url":a.uri()})).collect::<Vec<_>>());}
            }
            Provider::Compatible=>body["modalities"]=json!(["image","text"]),
        }
    }
    Ok((url, body))
}
pub fn parse_reply(provider: Provider, v: &Value) -> Result<Message, String> {
    if !v["error"].is_null() {
        return Err(v["error"]["message"]
            .as_str()
            .or_else(|| v["error"].as_str())
            .unwrap_or("The provider returned an error")
            .chars()
            .take(1000)
            .collect());
    }
    let mut m = Message {
        role: "assistant".into(),
        ..Default::default()
    };
    let mut image = |url: &str| {
        if let Some((head, data)) = url.split_once(",") {
            if m.attachments.len() < 4
                && [
                    "data:image/png;base64",
                    "data:image/jpeg;base64",
                    "data:image/webp;base64",
                    "data:image/gif;base64",
                ]
                .contains(&head)
                && head.ends_with(";base64")
                && data.len() < 32 * 1024 * 1024
            {
                m.attachments.push(Attachment {
                    name: "Generated image.png".into(),
                    mime: head[5..head.len() - 7].into(),
                    data: data.into(),
                });
            }
        }
    };
    match provider {
        Provider::Claude => {
            for p in v["content"].as_array().into_iter().flatten() {
                if let Some(t) = p["text"].as_str() {
                    m.text.push_str(t);
                }
            }
        }
        Provider::Gemini => {
            for p in v["candidates"][0]["content"]["parts"]
                .as_array()
                .into_iter()
                .flatten()
            {
                if let Some(t) = p["text"].as_str() {
                    m.text.push_str(t);
                }
                let d = &p["inlineData"];
                if let (Some(mime), Some(data)) = (d["mimeType"].as_str(), d["data"].as_str()) {
                    image(&format!("data:{mime};base64,{data}"));
                }
            }
        }
        _ => {
            let msg = &v["choices"][0]["message"];
            if let Some(t) = msg["content"].as_str() {
                m.text = t.into();
            }
            for p in msg["content"].as_array().into_iter().flatten() {
                if let Some(t) = p["text"].as_str() {
                    m.text.push_str(t);
                }
                if let Some(u) = p["image_url"]["url"].as_str() {
                    image(u);
                }
            }
            for p in msg["images"].as_array().into_iter().flatten() {
                if let Some(u) = p["image_url"]["url"].as_str() {
                    image(u);
                }
            }
            for p in v["data"].as_array().into_iter().flatten() {
                if let Some(b) = p["b64_json"].as_str() {
                    image(&format!("data:image/png;base64,{b}"));
                }
            }
        }
    }
    if m.text.len() > 128 * 1024 {
        return Err("The reply is too large to display. Ask for a shorter answer.".into());
    }
    if m.text.is_empty() && m.attachments.is_empty() {
        Err(
            "The provider returned no text or image. Check that the selected model supports chat."
                .into(),
        )
    } else {
        Ok(m)
    }
}
/// Readable clipboard text for the common Markdown emitted by chat models.
/// Code spans/fences keep their original characters; links retain their URLs.
pub fn plain_text(markdown: &str) -> String {
    fn inline(mut s: &str) -> String {
        let mut out = String::new();
        let mut emphasis = None;
        while !s.is_empty() {
            if let Some(rest) = s.strip_prefix('\\') {
                if let Some(c) = rest.chars().next() {
                    out.push(c);
                    s = &rest[c.len_utf8()..];
                    continue;
                }
            }
            if let Some(rest) = s.strip_prefix('`') {
                if let Some(end) = rest.find('`') {
                    out.push_str(&rest[..end]);
                    s = &rest[end + 1..];
                    continue;
                }
            }
            let link = s.strip_prefix('!').unwrap_or(s);
            if let Some(rest) = link.strip_prefix('[') {
                if let Some(end) = rest.find("](") {
                    if let Some(close) = rest[end + 2..].find(')') {
                        out.push_str(&rest[..end].replace("**", "").replace("__", ""));
                        out.push_str(" (");
                        out.push_str(&rest[end + 2..end + 2 + close]);
                        out.push(')');
                        s = &rest[end + 3 + close..];
                        continue;
                    }
                }
            }
            if s.starts_with("**") || s.starts_with("__") || s.starts_with("~~") {
                s = &s[2..];
                continue;
            }
            let c = s.chars().next().unwrap();
            let rest = &s[c.len_utf8()..];
            if c == '*' || c == '_' {
                if emphasis == Some(c) {
                    emphasis = None;
                    s = rest;
                    continue;
                }
                if !rest.starts_with(char::is_whitespace)
                    && rest.contains(c)
                    && !(c == '_' && out.chars().last().is_some_and(char::is_alphanumeric))
                {
                    emphasis = Some(c);
                    s = rest;
                    continue;
                }
            }
            out.push(c);
            s = rest;
        }
        out
    }
    let mut out = String::new();
    let mut fence = None;
    for line in markdown.lines() {
        let trimmed = line.trim_start();
        let marker = if trimmed.starts_with("```") {
            Some('`')
        } else if trimmed.starts_with("~~~") {
            Some('~')
        } else {
            None
        };
        if let Some(marker) = marker {
            if fence.is_none() {
                fence = Some(marker);
                continue;
            }
            if fence == Some(marker) {
                fence = None;
                continue;
            }
        }
        let s = if fence.is_some() {
            line.to_owned()
        } else {
            let mut s = line
                .trim_start_matches('#')
                .trim_start_matches('>')
                .trim_start();
            let bullet = s.starts_with("* ");
            if bullet {
                s = &s[2..];
            }
            let mut text = inline(s);
            if bullet {
                text = format!("- {text}");
            }
            text
        };
        out.push_str(&s);
        out.push('\n');
    }
    out.trim().to_owned()
}

pub fn base64(bytes: &[u8]) -> String {
    crate::pairdrop::keys::b64u(bytes)
        .replace('-', "+")
        .replace('_', "/")
        + match bytes.len() % 3 {
            1 => "==",
            2 => "=",
            _ => "",
        }
}
pub fn unbase64(s: &str) -> Option<Vec<u8>> {
    crate::pairdrop::keys::b64u_decode(&s.trim_end_matches('=').replace('+', "-").replace('/', "_"))
}

#[cfg(test)]
mod tests {
    #[test]
    fn catalog_availability_uses_explicit_signals_and_survives_old_caches() {
        let models = parse_models(&json!({"data":[
            {"id":"announced","expiration_date":"2099-12-31"},
            {"id":"shutdown","shutdown_date":"2026-01-01"},
            {"id":"flag","deprecated":true}, {"id":"off","available":false},
            {"id":"status","status":"retired"},
            {"id":"unknown","expiration_date":null},
            {"id":"invalid","shutdown_date":"2026-02-30"},
            {"id":"name-deprecated","description":"Compare deprecated models","deprecated":false}
        ]})).unwrap();
        for m in &models {
            assert_eq!(m.unavailable.is_some(), matches!(m.id.as_str(),"announced"|"shutdown"|"flag"|"off"|"status"),"{}",m.id);
        }
        let mut old = serde_json::to_value(Model::manual("old")).unwrap();
        old.as_object_mut().unwrap().remove("unavailable");
        assert!(serde_json::from_value::<Model>(old).unwrap().unavailable.is_none());
        let retired = models.iter().find(|m|m.id=="flag").unwrap();
        assert!(request(&Config::default(),retired,&[]).unwrap_err().contains("Deprecated"));
    }
    #[test]
    fn model_errors_distinguish_retirement_from_temporary_and_request_errors() {
        for (status,error,expected) in [
            (404,json!({"code":"model_not_found","message":"Unavailable"}),true),
            (400,json!({"message":"This model has been deprecated"}),true),
            (404,json!({"message":"models/p/model is not found or is not supported for generateContent"}),true),
            (400,json!({"message":"p/model is not a valid model ID"}),true),
            (404,json!({"message":"Route not found"}),false),
            (404,json!({"message":"No endpoints found that support image input"}),false),
            (400,json!({"message":"p/model: unsupported temperature parameter"}),false),
            (401,json!({"code":"model_not_found"}),false),
            (403,json!({"message":"Forbidden"}),false),
            (429,json!({"code":"model_not_found"}),false),
            (503,json!({"message":"p/model is not available"}),false)
        ] { assert_eq!(model_refused(status,&json!({"error":error}),"p/model"),expected,"{status}: {error}"); }
    }
    #[test]
    fn hackclub_hide_preference_defaults_on_and_is_scoped_to_profiles() {
        let mut c:Config=serde_json::from_str("{}").unwrap();
        assert!(c.hide_unavailable_models && !c.hackclub());
        c.switch(Provider::Compatible);
        c.completion_url="https://ai.hackclub.com/proxy/v1/chat/completions".into();
        assert!(c.hackclub());c.hide_unavailable_models=false;
        c.switch(Provider::Claude);assert!(!c.hackclub());
        c.switch(Provider::Compatible);assert!(c.hackclub() && !c.hide_unavailable_models);
        for bad in ["https://ai.hackclub.com.evil.org/models","https://ai.hackclub.com@evil.org/models","http://ai.hackclub.com/models","https://evil.org/ai.hackclub.com/models"] { assert!(!is_hackclub(bad)); }
        assert!(is_hackclub("https://AI.HACKCLUB.COM:443/proxy/v1/models"));
    }
    #[test]
    fn workspace_preferences_are_backward_compatible_and_round_trip() {
        let old:Config=serde_json::from_str(r#"{"default_model":"local/model"}"#).unwrap();
        assert!(!old.keep_open&&!old.resume&&old.horizontal_scroll);
        assert_eq!((old.mini_width,old.mini_height),(560,680));
        let mut current=old;current.keep_open=true;current.resume=true;current.horizontal_scroll=false;current.mini_width=421;current.mini_height=517;
        let restored:Config=serde_json::from_str(&serde_json::to_string(&current).unwrap()).unwrap();assert_eq!(restored,current);
    }
    use super::*;
    #[test]
    fn effort_is_encoded_for_each_provider_and_invalid_overrides_are_omitted() {
        let messages = [Message {
            role: "user".into(),
            text: "Explain".into(),
            ..Default::default()
        }];
        for (provider, id) in [
            (Provider::OpenAI, "gpt-6.1-sol"),
            (Provider::Claude, "claude-opus-4-6"),
            (Provider::Gemini, "gemini-3.1-pro-preview"),
        ] {
            let mut c = Config {
                provider,
                ..Default::default()
            };
            c.reasoning_efforts.insert(id.into(), "high".into());
            let (_, body) = request(&c, &Model::manual(id), &messages).unwrap();
            match provider {
                Provider::OpenAI => assert_eq!(body["reasoning_effort"], "high"),
                Provider::Claude => assert_eq!(body["output_config"]["effort"], "high"),
                Provider::Gemini => assert_eq!(
                    body["generationConfig"]["thinkingConfig"]["thinkingLevel"],
                    "HIGH"
                ),
                _ => unreachable!(),
            }
        }
        let mut c = Config {
            provider: Provider::Compatible,
            completion_url: "https://example.org/v1/chat/completions".into(),
            ..Default::default()
        };
        let m=parse_models(&json!({"data":[{"id":"some/model","supported_parameters":["reasoning"],"reasoning":{"supported_efforts":["high","low"]}}]})).unwrap().remove(0);
        c.reasoning_efforts.insert(m.id.clone(), "low".into());
        let (_, body) = request(&c, &m, &messages).unwrap();
        assert_eq!(body["reasoning"]["effort"], "low");
        assert!(body.get("reasoning_effort").is_none());
        c.reasoning_efforts.insert(m.id.clone(), "max".into());
        let (_, body) = request(&c, &m, &messages).unwrap();
        assert!(body.get("reasoning").is_none());
    }
    #[test]
    fn saved_efforts_follow_provider_profiles_and_old_settings_still_load() {
        let mut c:Config=serde_json::from_value(json!({"default_model":"my-model","profiles":{"Claude":{"completion_url":"","models_url":"","default_model":""}}})).unwrap();
        assert!(c.reasoning_efforts.is_empty());
        assert!(
            c.save_models,
            "old configurations enable the reusable model list"
        );
        c.save_models = false;
        c.reasoning_efforts
            .insert("gpt-6.1-sol".into(), "high".into());
        c.switch(Provider::Claude);
        assert!(c.save_models);
        c.save_models = true;
        assert!(c.reasoning_efforts.is_empty());
        c.reasoning_efforts
            .insert("claude-opus-4-6".into(), "low".into());
        c.switch(Provider::OpenAI);
        assert!(
            !c.save_models,
            "model-list preference follows the provider profile"
        );
        assert_eq!(
            c.reasoning_efforts.get("gpt-6.1-sol").map(String::as_str),
            Some("high")
        );
        let loaded: Config = serde_json::from_str(&serde_json::to_string(&c).unwrap()).unwrap();
        assert_eq!(loaded, c);
    }
    #[test]
    fn catalog_groups_metadata_and_unknown_prices() {
        let m=parse_models(&json!({"data":[{"id":"p/chat:free","architecture":{"input_modalities":["text","image"],"output_modalities":["text"]},"description":"Reads pictures"},{"id":"p/embed","pricing":{"prompt":"0.1","completion":"0"}},{"id":"p/art","architecture":{"output_modalities":["image"]}}]})).unwrap();
        assert_eq!(m[0].free, Some(true));
        assert_eq!(m[1].kind, Kind::Image);
        assert_eq!(m[1].free, None);
        assert!(m[0].details().contains("Accepts: text, image"));
        assert_eq!(m[2].kind, Kind::Embedding);
    }
    #[test]
    fn adapters_preserve_pdf_and_images() {
        for p in Provider::ALL {
            let c = Config {
                provider: p,
                completion_url: if p == Provider::Compatible {
                    "https://example.org/v1/chat/completions".into()
                } else {
                    String::new()
                },
                ..Default::default()
            };
            let msg = Message {
                role: "user".into(),
                text: "Read".into(),
                attachments: vec![Attachment {
                    name: "a.pdf".into(),
                    mime: "application/pdf".into(),
                    data: "YWJj".into(),
                }],
            };
            let (_, b) = request(&c, &Model::manual("x"), &[msg]).unwrap();
            assert!(b.to_string().contains("YWJj"));
            assert!(!b.to_string().contains("api_key"));
        }
    }
    #[test]
    fn validates_hosts_and_roundtrips() {
        assert!(!is_hackclub("https://ai.hackclub.com.attacker.org"));
        assert!(!is_hackclub("https://ai.hackclub.com@bad.org"));
        assert!(is_hackclub("https://ai.hackclub.com/proxy/v1/models"));
        assert!(validate_url("http://bad.org").is_err());
        for b in [b"a".as_slice(), b"ab", b"abc"] {
            assert_eq!(unbase64(&base64(b)).unwrap(), b);
        }
    }
    #[test]
    fn plain_copy_keeps_code_and_link_destinations() {
        assert_eq!(plain_text("## Hello\n**Bold** and *gentle* [docs](https://example.org)\n`snake_case ** x`\n```rust\nlet a_b = 2;\n```"),"Hello\nBold and gentle docs (https://example.org)\nsnake_case ** x\nlet a_b = 2;");
    }
    #[test]
    fn image_reuse_stays_in_the_new_user_message() {
        let a = Attachment {
            name: "image.png".into(),
            mime: "image/png".into(),
            data: "YWJj".into(),
        };
        let messages = [
            Message {
                role: "assistant".into(),
                text: "Generated image".into(),
                attachments: vec![a.clone()],
            },
            Message {
                role: "user".into(),
                text: "Make it blue".into(),
                attachments: vec![a],
            },
        ];
        let c = Config {
            provider: Provider::Compatible,
            completion_url: "https://example.org/v1/chat/completions".into(),
            ..Default::default()
        };
        let (_, b) = request(&c, &Model::manual("sample-image"), &messages).unwrap();
        assert_eq!(b["messages"][0]["content"].as_array().unwrap().len(), 1);
        assert_eq!(
            b["messages"][1]["content"][1]["image_url"]["url"],
            "data:image/png;base64,YWJj"
        );
        assert_eq!(b["modalities"], json!(["image", "text"]));
    }
    #[test]
    fn text_documents_use_text_content_for_every_provider_and_reject_bad_encoding() {
        let mut messages = vec![Message { role: "user".into(), text: "Review this".into(), attachments: vec![Attachment {
            name: "SHA256SUMS.txt".into(), mime: "text/plain".into(), data: base64("\u{feff}abc123  lumo.exe".as_bytes()),
        }] }];
        for provider in [Provider::Compatible,Provider::OpenAI,Provider::Claude,Provider::Gemini] {
            let c = Config { provider, completion_url: "https://example.org/v1/chat/completions".into(), ..Default::default() };
            let (_, body) = request(&c, &Model::manual("sample-model"), &messages).unwrap();
            let text = if provider == Provider::Gemini { &body["contents"][0]["parts"][1]["text"] } else { &body["messages"][0]["content"][1]["text"] };
            assert_eq!(text, "File: SHA256SUMS.txt\nabc123  lumo.exe");
            messages[0].attachments[0].data = base64(&[0xff,0xfe]);
            assert!(request(&c, &Model::manual("sample-model"), &messages).is_err());
            messages[0].attachments[0].data = base64("\u{feff}abc123  lumo.exe".as_bytes());
        }
    }
    #[test]
    fn image_followups_use_the_edit_endpoint_and_keep_the_supplied_file() {
        let c = Config::default();
        let m = Model::manual("gpt-image-test");
        let messages = [Message {
            role: "user".into(),
            text: "Make it blue".into(),
            attachments: vec![Attachment {
                name: "original.png".into(),
                mime: "image/png".into(),
                data: "YWJj".into(),
            }],
        }];
        let (url, body) = request(&c, &m, &messages).unwrap();
        assert!(url.ends_with("/images/edits"));
        assert_eq!(body["images"][0]["image_url"], "data:image/png;base64,YWJj");
        assert_eq!(body["prompt"], "Make it blue");
        let mut c = c;
        c.default_model = "my/model".into();
        c.completion_url = "https://example.org/v1/chat/completions".into();
        c.switch(Provider::Claude);
        c.switch(Provider::OpenAI);
        assert_eq!(c.default_model, "my/model");
        assert!(c.completion_url.starts_with("https://example.org"));
    }
    #[test]
    fn parses_images_and_empty_response() {
        let m=parse_reply(Provider::Compatible,&json!({"choices":[{"message":{"content":"Here","images":[{"image_url":{"url":"data:image/png;base64,YWJj"}}]}}]})).unwrap();
        assert_eq!(m.attachments.len(), 1);
        assert!(parse_reply(Provider::Claude, &json!({})).is_err());
    }
}
