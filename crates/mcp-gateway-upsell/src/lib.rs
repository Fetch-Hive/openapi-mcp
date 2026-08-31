//! Cloud call-to-action strings for the open-source CLI.
//!
//! This crate must not import telemetry, PostHog, or any network client.

pub const CLOUD_URL: &str = "https://fetchhive.com/mcp";

pub fn readme_paragraph() -> String {
    format!("Prefer a hosted MCP Gateway with tokens, quotas, and a dashboard? {CLOUD_URL}")
}

pub fn serve_boot_banner() -> String {
    format!("Hosted MCP Gateway with tokens, quotas, and a dashboard: {CLOUD_URL}")
}

pub fn upgrade_success_line() -> String {
    format!("Need hosted tokens, quotas, and a dashboard? {CLOUD_URL}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cta_contains_cloud_url() {
        for s in [
            readme_paragraph(),
            serve_boot_banner(),
            upgrade_success_line(),
        ] {
            assert!(s.contains(CLOUD_URL), "{s}");
            assert!(s.contains("fetchhive.com/mcp"), "{s}");
        }
    }

    #[test]
    fn cta_has_no_telemetry() {
        for s in [
            readme_paragraph(),
            serve_boot_banner(),
            upgrade_success_line(),
        ] {
            let lower = s.to_ascii_lowercase();
            assert!(!lower.contains("posthog"), "{s}");
            assert!(!lower.contains("telemetry"), "{s}");
            assert!(!lower.contains("segment.com"), "{s}");
            assert!(!lower.contains("api.mixpanel"), "{s}");
        }
    }
}
