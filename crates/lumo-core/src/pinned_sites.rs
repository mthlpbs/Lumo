//! Small user-owned browser pinned sites, stored with the existing profile data.
use serde::{Deserialize, Serialize};

pub const LIMIT: usize = 64;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PinnedSite {
    pub title: String,
    pub url: String,
}
pub fn save(
    items: &mut Vec<PinnedSite>,
    original: Option<&str>,
    title: &str,
    url: &str,
) -> Result<(), String> {
    let title = title.trim();
    let url = url.trim();
    if title.is_empty() || title.chars().count() > 96 || title.chars().any(char::is_control) {
        return Err("Enter a site name (up to 96 characters).".into());
    }
    if url.len() > 2048 || !crate::web::valid_url(url) {
        return Err("Enter a valid HTTP or HTTPS website address.".into());
    }
    let position = items
        .iter()
        .position(|b| b.url == url)
        .or_else(|| original.and_then(|old| items.iter().position(|b| b.url == old)));
    if position.is_none() && items.len() >= LIMIT {
        return Err("Up to 64 pinned sites can be saved.".into());
    }
    let value = PinnedSite {
        title: title.into(),
        url: url.into(),
    };
    if let Some(i) = position {
        items[i] = value;
    } else {
        items.push(value);
    }
    if let Some(old) = original.filter(|old| *old != url) {
        items.retain(|b| b.url != old);
    }
    // Editing onto another saved address updates it rather than duplicating it.
    let mut seen = std::collections::BTreeSet::new();
    items.retain(|b| seen.insert(b.url.clone()));
    Ok(())
}
pub fn sanitize(items: &mut Vec<PinnedSite>) {
    let old = std::mem::take(items);
    for b in old.into_iter().take(256) {
        let _ = save(items, None, &b.title, &b.url);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pinned_sites_survive_profile_roundtrip_and_support_editing() {
        let mut store = crate::store::Store::default();
        save(
            &mut store.pinned_sites,
            None,
            "Example",
            "https://example.com/",
        )
        .unwrap();
        save(
            &mut store.pinned_sites,
            Some("https://example.com/"),
            "Updated",
            "https://example.org/",
        )
        .unwrap();
        let encoded = serde_json::to_string(&store).unwrap();
        let decoded: crate::store::Store = serde_json::from_str(&encoded).unwrap();
        assert_eq!(
            decoded.pinned_sites,
            [PinnedSite {
                title: "Updated".into(),
                url: "https://example.org/".into()
            }]
        );
        assert!(serde_json::from_str::<crate::store::Store>("{}")
            .unwrap()
            .pinned_sites
            .is_empty());
    }
    #[test]
    fn invalid_and_unbounded_pinned_sites_do_not_enter_the_store() {
        let mut items = vec![];
        for url in [
            "javascript:alert(1)",
            "file:///private",
            "https://user:pass@example.com/",
            "https://bad host/",
        ] {
            assert!(save(&mut items, None, "Test", url).is_err());
        }
        for i in 0..LIMIT {
            save(
                &mut items,
                None,
                "Test",
                &format!("https://example.com/{i}"),
            )
            .unwrap();
        }
        assert!(save(&mut items, None, "Overflow", "https://example.org/").is_err());
        save(&mut items, None, "Changed", "https://example.com/0").unwrap();
        assert_eq!(items.len(), LIMIT);
    }
}
