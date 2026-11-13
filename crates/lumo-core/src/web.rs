//! Preferences for the two on-demand embedded web surfaces.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChatSite {
    pub id: u64,
    pub name: String,
    pub url: String,
}
pub fn default_sites() -> Vec<ChatSite> {
    [
        ("ChatGPT", "https://chatgpt.com/"),
        ("Gemini", "https://gemini.google.com/app"),
        ("Claude", "https://claude.ai/"),
        ("Muse", "https://muse.ai/"),
    ]
    .into_iter()
    .enumerate()
    .map(|(i, (name, url))| ChatSite {
        id: i as u64 + 1,
        name: name.into(),
        url: url.into(),
    })
    .collect()
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    #[serde(skip, default="default_sites")]
    pub chat_sites: Vec<ChatSite>,
    #[serde(alias="filtering")]
    pub adblocking: bool,
}
impl Default for Config {
    fn default()->Self {Self{chat_sites:default_sites(),adblocking:false}}
}
/// Explicit HTTP(S) addresses only. Exclude credentials and ambiguous hosts.
pub fn valid_url(url: &str) -> bool {
    let Some((scheme, rest)) = url.split_once("://") else {
        return false;
    };
    if !matches!(scheme, "https" | "http")
        || url
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || c == '\\')
    {
        return false;
    }
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let port_ok = |port: &str| {
        port.is_empty()
            || port
                .strip_prefix(':')
                .is_some_and(|p| !p.is_empty() && p.parse::<u16>().is_ok_and(|n| n > 0))
    };
    if let Some(ip) = authority.strip_prefix('[') {
        return ip
            .split_once(']')
            .is_some_and(|(ip, port)| ip.parse::<std::net::Ipv6Addr>().is_ok() && port_ok(port));
    }
    let (host, port) = authority
        .find(':')
        .map_or((authority, ""), |i| (&authority[..i], &authority[i..]));
    let host = host.trim_end_matches('.');
    !host.is_empty()
        && host.len() <= 253
        && port_ok(port)
        && host.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'-')
        })
}
impl Config {
    pub fn sanitize(&mut self) {
        self.chat_sites=default_sites();

    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn old_filter_list_settings_are_retired_but_the_switch_survives() {
        let c:Config=serde_json::from_str(r#"{"filtering":true,"custom_filters":"old","subscriptions":[{"url":"https://old.test/","enabled":true}]}"#).unwrap();
        assert!(c.adblocking);
        let saved=serde_json::to_string(&c).unwrap();
        for old in ["custom_filters","subscriptions","filtering","default_lists_version"] {assert!(!saved.contains(old));}
    }
    #[test]
    fn removed_sites_stay_removed_and_old_settings_get_defaults() {
        let mut config: Config = serde_json::from_str("{}").unwrap();
        assert_eq!(config.chat_sites.len(), 4);
        config.chat_sites.clear();
        let mut restored: Config =
            serde_json::from_str(&serde_json::to_string(&config).unwrap()).unwrap();
        restored.sanitize();
        assert_eq!(restored.chat_sites, default_sites());
        for bad in [
            "javascript:alert(1)",
            "https://",
            "https://a@b/",
            "https://a b/",
            "https://:443/",
            "https://[invalid]/",
            "https://example.com:99999/",
            "https://example.com\\evil.test/",
        ] {
            assert!(!valid_url(bad));
        }
        for good in [
            "https://example.com/room",
            "http://localhost:8080/",
            "https://[::1]:8443/",
        ] {
            assert!(valid_url(good));
        }
    }
}
