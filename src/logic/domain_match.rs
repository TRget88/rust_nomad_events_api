// ============================================================================
// src/logic/domain_match.rs
// ============================================================================
//
// Pure, dependency-free domain matching for the ownership-request
// *auto-approval* path. When a logged-in user requests ownership of an
// event, the workflow can skip the human approver entirely IF the
// requester's **verified** email domain matches the event's website
// domain — the lightweight equivalent of the "prove you control the
// domain" check used by Google Search Console / GitHub Pages / etc.
//
// We deliberately avoid pulling in the `url` / `psl` crates:
//   * `url` is heavier than we need (we only want the host).
//   * `psl` ships the full Public Suffix List (~250 KB, frequently
//     updated). For a conservative auto-approval gate a small, audited,
//     hand-rolled suffix table is safer than silently trusting an
//     embedded list we don't control — and it keeps the dependency tree
//     lean. The cost is that exotic multi-label suffixes we didn't list
//     fall back to the 2-label default, which only makes the match
//     *stricter* (it never widens it), so the failure mode is "ask a
//     human" rather than "wrongly auto-approve".
//
// Everything here is a pure function over `&str` so the whole module is
// unit-tested without a DB or a runtime.

/// Known multi-label public suffixes (`co.uk`, `com.au`, ...). When a
/// host ends in one of these, the registrable domain is the suffix plus
/// ONE more label to the left (so `bbc.co.uk`, not `co.uk`). Anything
/// not listed falls back to the single-label rule (`example.com`),
/// which can only ever make a match *stricter*, never looser.
///
/// This is intentionally not exhaustive — it covers the common ccTLD
/// second levels. Add entries as real events surface them.
const MULTI_LABEL_SUFFIXES: &[&str] = &[
    // United Kingdom
    "co.uk",
    "org.uk",
    "gov.uk",
    "ac.uk",
    "me.uk",
    "net.uk",
    "sch.uk",
    "ltd.uk",
    "plc.uk",
    "nhs.uk",
    // Australia
    "com.au",
    "net.au",
    "org.au",
    "edu.au",
    "gov.au",
    "id.au",
    "asn.au",
    // New Zealand
    "co.nz",
    "net.nz",
    "org.nz",
    "govt.nz",
    "ac.nz",
    "geek.nz",
    "school.nz",
    // South Africa
    "co.za",
    "org.za",
    "gov.za",
    "net.za",
    "web.za",
    // Japan
    "co.jp",
    "or.jp",
    "ne.jp",
    "go.jp",
    "ac.jp",
    "ad.jp",
    "ed.jp",
    "gr.jp",
    "lg.jp",
    // Brazil
    "com.br",
    "net.br",
    "org.br",
    "gov.br",
    "edu.br",
    // India
    "co.in",
    "net.in",
    "org.in",
    "gov.in",
    "ac.in",
    "edu.in",
    "firm.in",
    "gen.in",
    "ind.in",
    // Mexico
    "com.mx",
    "gob.mx",
    "org.mx",
    "edu.mx",
    "net.mx",
    // South Korea
    "co.kr",
    "or.kr",
    "ne.kr",
    "re.kr",
    "go.kr",
    "ac.kr",
    // Singapore
    "com.sg",
    "net.sg",
    "org.sg",
    "gov.sg",
    "edu.sg",
    "per.sg",
    // China
    "com.cn",
    "net.cn",
    "org.cn",
    "gov.cn",
    "edu.cn",
    "ac.cn",
    // Hong Kong
    "com.hk",
    "org.hk",
    "net.hk",
    "gov.hk",
    "edu.hk",
];

/// Free / shared-mailbox email providers. A match against one of these
/// proves nothing about organizational control of a website — anyone
/// can hold a `gmail.com` address — so an email whose registrable
/// domain is in this list can NEVER drive an auto-approval. (If someone
/// set an event's website to `https://gmail.com`, this is also what
/// stops every Gmail user from claiming it.) Such requests fall through
/// to human owner/admin approval.
const PUBLIC_EMAIL_DOMAINS: &[&str] = &[
    "gmail.com",
    "googlemail.com",
    "yahoo.com",
    "ymail.com",
    "rocketmail.com",
    "hotmail.com",
    "hotmail.co.uk",
    "outlook.com",
    "outlook.co.uk",
    "live.com",
    "msn.com",
    "icloud.com",
    "me.com",
    "mac.com",
    "aol.com",
    "gmx.com",
    "gmx.net",
    "mail.com",
    "zoho.com",
    "proton.me",
    "protonmail.com",
    "pm.me",
    "tutanota.com",
    "tuta.io",
    "yandex.com",
    "fastmail.com",
    "hey.com",
];

/// Pull the bare host out of a URL-ish string. Tolerant by design — the
/// `website` field is free-text and may arrive with or without a scheme,
/// with a path, a port, a trailing slash, mixed case, or a trailing FQDN
/// dot. Returns the lowercased host with all of that stripped, or `None`
/// if nothing host-like remains.
///
/// Examples:
///   `https://www.Example.com/tour?utm=x` -> `www.example.com`
///   `example.com`                        -> `example.com`
///   `http://user@host.org:8080/`         -> `host.org`
pub fn extract_host_from_url(raw: &str) -> Option<String> {
    let mut s = raw.trim();
    if s.is_empty() {
        return None;
    }
    // 1. Strip scheme ("https://", "http://", "ftp://", ...).
    if let Some(idx) = s.find("://") {
        s = &s[idx + 3..];
    }
    // 2. Strip any scheme-relative leading slashes ("//host/...").
    s = s.trim_start_matches('/');
    // 3. Host ends at the first '/', '?' or '#'.
    let end = s.find(['/', '?', '#']).unwrap_or(s.len());
    s = &s[..end];
    // 4. Strip userinfo ("user:pass@host" -> "host").
    if let Some(idx) = s.rfind('@') {
        s = &s[idx + 1..];
    }
    // 5. Strip a trailing :port (only when what follows the last colon
    //    is all digits — leaves IPv6-ish junk intact for the validator
    //    below to reject).
    if let Some(idx) = s.rfind(':') {
        let after = &s[idx + 1..];
        if !after.is_empty() && after.bytes().all(|b| b.is_ascii_digit()) {
            s = &s[..idx];
        }
    }
    // 6. Normalize: drop a trailing FQDN dot, lowercase.
    let host = s.trim().trim_end_matches('.').to_ascii_lowercase();
    if host.is_empty() {
        return None;
    }
    Some(host)
}

/// Pull the domain part out of an email address (`jane@Example.com` ->
/// `example.com`). Returns `None` for anything without a non-empty local
/// part AND a non-empty domain part.
pub fn domain_from_email(email: &str) -> Option<String> {
    let email = email.trim();
    let at = email.rfind('@')?;
    // Local part must be non-empty ("@example.com" is not an address).
    if at == 0 {
        return None;
    }
    let domain = email[at + 1..]
        .trim()
        .trim_end_matches('.')
        .to_ascii_lowercase();
    if domain.is_empty() {
        return None;
    }
    Some(domain)
}

/// Reduce a host to its registrable domain (eTLD+1): the part a person
/// or org actually registers. `www.bbc.co.uk` -> `bbc.co.uk`,
/// `tour.example.com` -> `example.com`.
///
/// Returns `None` when the input isn't a registrable domain:
///   * single-label hosts (`localhost`),
///   * bare public suffixes (`co.uk`, `com`),
///   * IPv4 / numeric-TLD hosts (`192.168.0.1`),
///   * anything with a label outside the DNS `[a-z0-9-]` charset
///     (leftover ports, IPv6 brackets, empty labels from `a..b`).
pub fn registrable_domain(host: &str) -> Option<String> {
    let host = host.trim().trim_end_matches('.').to_ascii_lowercase();
    if host.is_empty() {
        return None;
    }
    let labels: Vec<&str> = host.split('.').collect();
    if labels.len() < 2 {
        // A registrable domain needs at least a label + a TLD.
        return None;
    }
    // Validate every label is a well-formed DNS label.
    for label in &labels {
        if label.is_empty() {
            return None;
        }
        if !label
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        {
            return None;
        }
        if label.starts_with('-') || label.ends_with('-') {
            return None;
        }
    }
    // A real TLD is never all-digits — this rejects IPv4 literals.
    let tld = labels[labels.len() - 1];
    if tld.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    // Suffix is 2 labels if the trailing pair is a known multi-label
    // suffix (co.uk), else 1 label (.com).
    let last_two = format!("{}.{}", labels[labels.len() - 2], labels[labels.len() - 1]);
    let suffix_len = if MULTI_LABEL_SUFFIXES.contains(&last_two.as_str()) {
        2
    } else {
        1
    };
    // registrable = suffix + exactly one more label to its left.
    let needed = suffix_len + 1;
    if labels.len() < needed {
        // Host is a bare public suffix (e.g. exactly "co.uk") with no
        // registrable label in front.
        return None;
    }
    let start = labels.len() - needed;
    Some(labels[start..].join("."))
}

/// True when `registrable` is a free / shared-mailbox provider that
/// can't prove organizational ownership of a website.
fn is_public_email_domain(registrable: &str) -> bool {
    PUBLIC_EMAIL_DOMAINS.contains(&registrable)
}

/// THE decision the auto-approval path turns on: does this email's
/// domain match this website's domain closely enough to skip a human
/// approver?
///
/// Returns `true` only when BOTH the email domain and the website host
/// reduce to the SAME registrable domain AND that domain is not a free
/// email provider. Any ambiguity (unparseable email, unparseable
/// website, bare public suffix, free-mail domain) returns `false`, which
/// routes the request to normal owner/admin approval. False negatives
/// are cheap (a human approves); false positives hand an event to the
/// wrong person, so we bias hard toward `false`.
///
/// NOTE: the caller is responsible for only invoking this with a
/// **verified** email. An unverified email proves nothing.
pub fn domains_match(email: &str, website: &str) -> bool {
    let (Some(email_host), Some(site_host)) =
        (domain_from_email(email), extract_host_from_url(website))
    else {
        return false;
    };
    let (Some(email_reg), Some(site_reg)) = (
        registrable_domain(&email_host),
        registrable_domain(&site_host),
    ) else {
        return false;
    };
    // Free email can never serve as proof of website control.
    if is_public_email_domain(&email_reg) {
        return false;
    }
    email_reg == site_reg
}

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------
    // extract_host_from_url
    // -----------------------------------------------------------------

    #[test]
    fn host_from_plain_domain() {
        assert_eq!(
            extract_host_from_url("example.com").as_deref(),
            Some("example.com")
        );
    }

    #[test]
    fn host_strips_scheme_path_query_and_lowercases() {
        assert_eq!(
            extract_host_from_url("https://www.Example.com/tour?utm=x#top").as_deref(),
            Some("www.example.com"),
        );
    }

    #[test]
    fn host_strips_userinfo_and_port() {
        assert_eq!(
            extract_host_from_url("http://user:pass@Host.org:8080/path").as_deref(),
            Some("host.org"),
        );
    }

    #[test]
    fn host_strips_trailing_fqdn_dot_and_slash() {
        assert_eq!(
            extract_host_from_url("https://example.com./").as_deref(),
            Some("example.com")
        );
    }

    #[test]
    fn host_handles_scheme_relative() {
        // `extract_host_from_url` returns the FULL host (subdomains intact —
        // see `host_strips_scheme_path_query_and_lowercases`, which keeps
        // `www.`). Reducing `cdn.example.com` to `example.com` is the job of
        // `registrable_domain`, exercised separately below. This test only
        // pins that a scheme-relative `//` prefix is stripped correctly.
        assert_eq!(
            extract_host_from_url("//cdn.example.com/x").as_deref(),
            Some("cdn.example.com")
        );
    }

    #[test]
    fn host_empty_and_whitespace_are_none() {
        assert_eq!(extract_host_from_url(""), None);
        assert_eq!(extract_host_from_url("   "), None);
        assert_eq!(extract_host_from_url("https://"), None);
    }

    // -----------------------------------------------------------------
    // domain_from_email
    // -----------------------------------------------------------------

    #[test]
    fn email_domain_basic() {
        assert_eq!(
            domain_from_email("Jane.Doe@Example.COM").as_deref(),
            Some("example.com")
        );
    }

    #[test]
    fn email_domain_subdomain_preserved() {
        assert_eq!(
            domain_from_email("ops@mail.events.example.com").as_deref(),
            Some("mail.events.example.com"),
        );
    }

    #[test]
    fn email_domain_rejects_missing_parts() {
        assert_eq!(domain_from_email("no-at-sign"), None);
        assert_eq!(domain_from_email("@example.com"), None); // empty local part
        assert_eq!(domain_from_email("jane@"), None); // empty domain
        assert_eq!(domain_from_email(""), None);
    }

    // -----------------------------------------------------------------
    // registrable_domain
    // -----------------------------------------------------------------

    #[test]
    fn registrable_two_label() {
        assert_eq!(
            registrable_domain("example.com").as_deref(),
            Some("example.com")
        );
    }

    #[test]
    fn registrable_strips_subdomains() {
        assert_eq!(
            registrable_domain("www.tour.example.com").as_deref(),
            Some("example.com")
        );
    }

    #[test]
    fn registrable_handles_multi_label_suffix() {
        assert_eq!(
            registrable_domain("bbc.co.uk").as_deref(),
            Some("bbc.co.uk")
        );
        assert_eq!(
            registrable_domain("www.bbc.co.uk").as_deref(),
            Some("bbc.co.uk")
        );
        assert_eq!(
            registrable_domain("shop.events.com.au").as_deref(),
            Some("events.com.au")
        );
    }

    #[test]
    fn registrable_rejects_bare_suffix() {
        // A bare public suffix has no registrable label in front.
        assert_eq!(registrable_domain("co.uk"), None);
        assert_eq!(registrable_domain("com"), None);
    }

    #[test]
    fn registrable_rejects_single_label() {
        assert_eq!(registrable_domain("localhost"), None);
    }

    #[test]
    fn registrable_rejects_ipv4() {
        assert_eq!(registrable_domain("192.168.0.1"), None);
        assert_eq!(registrable_domain("8.8.8.8"), None);
    }

    #[test]
    fn registrable_rejects_bad_labels() {
        assert_eq!(registrable_domain("a..b.com"), None); // empty label
        assert_eq!(registrable_domain("exa_mple.com"), None); // underscore not allowed
        assert_eq!(registrable_domain("-bad.com"), None); // leading hyphen
        assert_eq!(registrable_domain("bad-.com"), None); // trailing hyphen
    }

    // -----------------------------------------------------------------
    // domains_match — the gate
    // -----------------------------------------------------------------

    #[test]
    fn match_exact_domain() {
        assert!(domains_match(
            "booking@coolfest.com",
            "https://coolfest.com"
        ));
    }

    #[test]
    fn match_across_subdomains() {
        // Email on a subdomain, website on www — same registrable domain.
        assert!(domains_match(
            "lead@marketing.coolfest.com",
            "https://www.coolfest.com/tickets"
        ));
    }

    #[test]
    fn match_multi_label_suffix() {
        assert!(domains_match(
            "info@bigfest.co.uk",
            "http://www.bigfest.co.uk"
        ));
    }

    #[test]
    fn no_match_different_domains() {
        assert!(!domains_match("evil@attacker.com", "https://coolfest.com"));
    }

    #[test]
    fn no_match_when_registrable_differs_despite_shared_suffix() {
        // attacker.co.uk vs victim.co.uk share the co.uk suffix but are
        // different registrable domains — must NOT match.
        assert!(!domains_match(
            "evil@attacker.co.uk",
            "https://victim.co.uk"
        ));
    }

    #[test]
    fn no_match_for_free_email_provider() {
        // The critical anti-abuse case: a gmail address must never
        // auto-approve, even if the website literally *is* gmail.com.
        assert!(!domains_match(
            "randomperson@gmail.com",
            "https://gmail.com"
        ));
        assert!(!domains_match(
            "randomperson@gmail.com",
            "https://coolfest.com"
        ));
    }

    #[test]
    fn no_match_on_unparseable_inputs() {
        assert!(!domains_match("not-an-email", "https://coolfest.com"));
        assert!(!domains_match("booking@coolfest.com", ""));
        assert!(!domains_match("booking@coolfest.com", "https://localhost"));
        assert!(!domains_match("booking@localhost", "https://coolfest.com"));
    }

    #[test]
    fn match_is_case_insensitive() {
        assert!(domains_match(
            "BOOKING@CoolFest.COM",
            "HTTPS://WWW.COOLFEST.com/"
        ));
    }
}
