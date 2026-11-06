//! Small, on-demand effort metadata. Catalog declarations take precedence.
//! Native fallbacks verified against provider docs on 2026-11-03; no model list
//! or tokenizer is bundled and unknown models never acquire guessed controls.
use crate::ai::Provider;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Reasoning {
    pub listed: bool,
    pub levels: Vec<String>,
    pub default_effort: Option<String>,
    pub mandatory: bool,
}
pub fn label(value: &str) -> String {
    match value {
        "none" => "Off".into(),
        "xhigh" => "Extra high".into(),
        "max" => "Maximum".into(),
        v => {
            let mut c = v.chars();
            c.next()
                .map(|first| first.to_uppercase().collect::<String>() + c.as_str())
                .unwrap_or_default()
                .replace('_', " ")
        }
    }
}
fn rank(s: &str) -> usize {
    [
        "none", "minimal", "low", "medium", "high", "xhigh", "max", "ultra",
    ]
    .iter()
    .position(|v| *v == s)
    .unwrap_or(8)
}
impl Reasoning {
    pub fn parse(v: &Value) -> Option<Self> {
        let r = v.get("reasoning")?.as_object()?;
        let mut result = Self {
            listed: true,
            mandatory: r.get("mandatory") == Some(&Value::Bool(true)),
            ..Default::default()
        };
        if let Some(values) = r.get("supported_efforts") {
            result.levels = if values.is_null() {
                ["none", "minimal", "low", "medium", "high", "xhigh", "max"]
                    .iter()
                    .map(|v| v.to_string())
                    .collect()
            } else {
                values
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .take(16)
                    .filter(|s| {
                        s.len() <= 32
                            && !s.is_empty()
                            && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
                    })
                    .map(|s| s.to_ascii_lowercase())
                    .collect()
            };
        }
        if result.mandatory {
            result.levels.retain(|v| v != "none");
        }
        result.levels.sort_by_key(|s| rank(s));
        result.levels.dedup();
        result.default_effort = r
            .get("default_effort")
            .and_then(Value::as_str)
            .filter(|s| result.levels.iter().any(|v| v == s))
            .map(str::to_owned);
        if r.get("default_enabled") == Some(&Value::Bool(false)) {
            result.default_effort = result
                .levels
                .iter()
                .any(|v| v == "none")
                .then(|| "none".into());
        }
        Some(result)
    }
    pub fn resolve(&mut self, id: &str, provider: Provider) {
        if self.listed {
            self.restrict(provider);
            return;
        }
        if provider == Provider::Compatible {
            return;
        }
        let mut id = id.trim_start_matches("models/");
        // Remove dated snapshots, never variants such as -pro or -codex.
        if id.len() > 11
            && id.as_bytes()[id.len() - 11] == b'-'
            && id[id.len() - 10..]
                .bytes()
                .filter(|b| *b != b'-')
                .all(|b| b.is_ascii_digit())
            && id[id.len() - 10..].bytes().filter(|b| *b == b'-').count() == 2
        {
            id = &id[..id.len() - 11];
        }
        if id.len() > 9
            && id.as_bytes()[id.len() - 9] == b'-'
            && id[id.len() - 8..].bytes().all(|b| b.is_ascii_digit())
        {
            id = &id[..id.len() - 9];
        }
        let (levels, default): (&[&str], Option<&str>) = match (provider, id) {
            (Provider::OpenAI, "gpt-6.1-sol") => {
                (&["low", "medium", "high", "xhigh", "max"], Some("medium"))
            }
            (Provider::OpenAI, "gpt-6-astra") => (&["low", "medium", "high", "xhigh", "max"], None),
            (Provider::OpenAI, "gpt-6.1-terra") => (
                &["none", "low", "medium", "high", "xhigh", "max"],
                Some("medium"),
            ),
            (Provider::OpenAI, "gpt-5.2") => {
                (&["none", "low", "medium", "high", "xhigh"], Some("none"))
            }
            (Provider::OpenAI, "gpt-5" | "gpt-5-mini" | "gpt-5-nano") => {
                (&["minimal", "low", "medium", "high"], Some("medium"))
            }
            (Provider::Claude, "claude-opus-4-5") => (&["low", "medium", "high"], Some("high")),
            (Provider::Claude, "claude-opus-4-6" | "claude-sonnet-4-6") => {
                (&["low", "medium", "high", "max"], Some("high"))
            }
            (
                Provider::Claude,
                "claude-opus-4-7" | "claude-opus-4-8" | "claude-opus-5" | "claude-sonnet-5"
                | "claude-sonnet-5-5" | "claude-fable-5" | "claude-fable-5-1" | "claude-mythos-5"
                | "claude-mythos-5-1",
            ) => (&["low", "medium", "high", "xhigh", "max"], Some("high")),
            (Provider::Claude, "claude-opus-5-5" | "claude-haiku-5-5") => {
                (&["low", "medium", "high", "xhigh", "max"], Some("medium"))
            }
            (Provider::Gemini, "gemini-4-argon" | "gemini-3.8-flash") => {
                (&["low", "medium", "high"], Some("medium"))
            }
            (Provider::Gemini, "gemini-3.6-flash") => {
                (&["minimal", "low", "medium", "high"], Some("medium"))
            }
            (Provider::Gemini, "gemini-3.5-flash-lite") => {
                (&["minimal", "low", "medium", "high"], Some("minimal"))
            }
            (Provider::Gemini, "gemini-3.1-pro-preview") => {
                (&["low", "medium", "high"], Some("high"))
            }
            (Provider::Gemini, "gemini-3-flash-preview") => {
                (&["minimal", "low", "medium", "high"], Some("high"))
            }
            (Provider::Gemini, "gemini-3.1-flash-lite-image") => {
                (&["minimal", "high"], Some("minimal"))
            }
            _ => return,
        };
        self.listed = true;
        self.levels = levels.iter().map(|v| v.to_string()).collect();
        self.default_effort = default.map(str::to_owned);
        self.mandatory = !self.levels.iter().any(|v| v == "none");
        self.restrict(provider);
    }
    fn restrict(&mut self, provider: Provider) {
        if provider == Provider::Gemini {
            self.levels
                .retain(|v| ["minimal", "low", "medium", "high"].contains(&v.as_str()));
        }
        if provider == Provider::Claude {
            self.levels
                .retain(|v| ["low", "medium", "high", "xhigh", "max"].contains(&v.as_str()));
        }
        if self
            .default_effort
            .as_ref()
            .is_some_and(|v| !self.levels.contains(v))
        {
            self.default_effort = None;
        }
    }
    pub fn adjustable(&self) -> bool {
        self.levels.len() > 1
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn catalog_levels_are_authoritative_and_mandatory_models_cannot_be_disabled() {
        let mut r=Reasoning::parse(&json!({"reasoning":{"mandatory":true,"supported_efforts":["max","high","low","none","high"],"default_effort":"high"}})).unwrap();
        assert_eq!(r.levels, ["low", "high", "max"]);
        assert_eq!(r.default_effort.as_deref(), Some("high"));
        r.resolve("gpt-6.1-sol", Provider::OpenAI);
        assert_eq!(r.levels, ["low", "high", "max"]);
        let mut fixed = Reasoning::parse(&json!({"reasoning":{"mandatory":true}})).unwrap();
        fixed.resolve("gpt-6.1-sol", Provider::OpenAI);
        assert!(!fixed.adjustable());
        let disabled=Reasoning::parse(&json!({"reasoning":{"default_enabled":false,"supported_efforts":["high","low"],"default_effort":"high"}})).unwrap();
        assert!(disabled.default_effort.is_none());
        let all = Reasoning::parse(&json!({"reasoning":{"supported_efforts":null}})).unwrap();
        assert_eq!(all.levels.len(), 7);
    }
    #[test]
    fn native_profiles_preserve_model_specific_levels_without_guessing_variants() {
        let mut r = Reasoning::default();
        r.resolve("gpt-6.1-sol-2026-10-01", Provider::OpenAI);
        assert_eq!(r.levels, ["low", "medium", "high", "xhigh", "max"]);
        let mut r = Reasoning::default();
        r.resolve("claude-sonnet-4-6", Provider::Claude);
        assert!(r.levels.contains(&"max".into()));
        assert!(!r.levels.contains(&"xhigh".into()));
        for (id, p) in [
            ("gpt-6.1-sol-pro", Provider::OpenAI),
            ("gpt-6.1-sol", Provider::Compatible),
            ("claude-haiku-4-5", Provider::Claude),
            ("gemini-2.5-pro", Provider::Gemini),
            ("unknown-🎧-long-model", Provider::OpenAI),
        ] {
            let mut r = Reasoning::default();
            r.resolve(id, p);
            assert!(!r.adjustable(), "{id}");
        }
    }
}
