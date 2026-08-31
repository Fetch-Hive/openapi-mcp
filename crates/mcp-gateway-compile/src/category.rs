//! Category assignment.

const GENERIC_SEGMENTS: &[&str] = &["v1", "v2", "v3", "api", "rest"];

pub fn category(tags: &[String], path_template: &str, schema_ref_name: Option<&str>) -> String {
    if let Some(tag) = tags.iter().map(|t| t.trim()).find(|t| !t.is_empty()) {
        return tag.to_owned();
    }
    if let Some(seg) = path_prefix(path_template) {
        return seg;
    }
    if let Some(name) = schema_ref_name.map(str::trim).filter(|s| !s.is_empty()) {
        return crate::names::normalize(name, false);
    }
    "untagged".to_owned()
}

fn path_prefix(path: &str) -> Option<String> {
    let segs = path
        .split('/')
        .filter(|s| !s.is_empty() && !s.starts_with('{'));
    for seg in segs {
        let lower = seg.to_ascii_lowercase();
        if GENERIC_SEGMENTS.contains(&lower.as_str()) {
            continue;
        }
        return Some(seg.to_owned());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_tag_wins() {
        assert_eq!(
            category(&["repos".into(), "other".into()], "/x", None),
            "repos"
        );
    }

    #[test]
    fn skips_v1() {
        assert_eq!(category(&[], "/v1/customers", None), "customers");
    }

    #[test]
    fn repos_path() {
        assert_eq!(category(&[], "/repos/{owner}/{repo}", None), "repos");
    }

    #[test]
    fn schema_name_fallback() {
        assert_eq!(category(&[], "/{id}", Some("Charge")), "charge");
    }

    #[test]
    fn untagged() {
        assert_eq!(category(&[], "/{id}", None), "untagged");
    }
}
