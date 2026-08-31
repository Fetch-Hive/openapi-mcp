//! SSRF classification, IANA CIDR tables, and URL pinning.
//!
//! This module is pure except for the injected [`Resolver`]. It does not dial.

mod cidrs;
mod classify;
mod resolver;

pub use cidrs::{ipv4_denied, ipv6_denied, IPV4_DENY_ROWS, IPV6_DENY_ROWS};
pub use classify::{is_blocked_ip, pin_url, unwrap_ip, Pinned, SsrfError, SsrfPolicy};
pub use resolver::{HickoryResolver, MapResolver, Resolver, SystemResolver};

#[cfg(test)]
mod tests;
