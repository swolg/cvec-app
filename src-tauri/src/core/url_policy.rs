use crate::Error;

pub fn external_url(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if is_external_https_url(trimmed) {
        Some(trimmed.to_string())
    } else {
        None
    }
}

pub fn validate_external_url(value: &str) -> Result<String, Error> {
    external_url(value).ok_or_else(|| Error::App("only valid HTTPS URLs can be opened".into()))
}

fn is_external_https_url(value: &str) -> bool {
    if value.is_empty() || value.chars().any(is_forbidden_url_char) {
        return false;
    }

    let Some(rest) = value.strip_prefix("https://") else {
        return false;
    };

    let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..authority_end];
    if authority.is_empty() || authority.contains('@') || authority.contains('\\') {
        return false;
    }

    if let Some(stripped) = authority.strip_prefix('[') {
        let Some((host, remainder)) = stripped.split_once(']') else {
            return false;
        };
        if !remainder.is_empty() && !remainder.strip_prefix(':').is_some_and(is_valid_port) {
            return false;
        }
        is_valid_ipv6_host(host)
    } else {
        let (host, port) = authority
            .split_once(':')
            .map(|(host, port)| (host, Some(port)))
            .unwrap_or((authority, None));
        if port.is_some_and(|port| port.contains(':') || !is_valid_port(port)) {
            return false;
        }
        is_valid_dns_or_ipv4_host(host)
    }
}

fn is_forbidden_url_char(character: char) -> bool {
    character.is_whitespace()
        || character.is_control()
        || matches!(
            character,
            '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{206F}' | '\u{FEFF}'
        )
}

fn is_valid_ipv6_host(host: &str) -> bool {
    !host.is_empty()
        && host.contains(':')
        && host
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() || byte == b':' || byte == b'.')
}

fn is_valid_dns_or_ipv4_host(host: &str) -> bool {
    !host.is_empty()
        && !host.starts_with('.')
        && !host.ends_with('.')
        && host.split('.').all(|label| {
            !label.is_empty()
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        })
}

fn is_valid_port(value: &str) -> bool {
    !value.is_empty()
        && value.bytes().all(|byte| byte.is_ascii_digit())
        && value.parse::<u16>().is_ok_and(|port| port > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_https_urls() {
        assert_eq!(
            external_url(" https://github.com/advisories/GHSA-abcd-efgh-ijkl "),
            Some("https://github.com/advisories/GHSA-abcd-efgh-ijkl".to_string())
        );
        assert!(external_url("https://[::1]/path").is_some());
        assert!(external_url("https://cvec.app:443/docs").is_some());
    }

    #[test]
    fn rejects_non_https_or_ambiguous_urls() {
        for value in [
            "",
            "http://example.com",
            "javascript:alert(1)",
            "https://",
            "https://user@example.com/path",
            "https://example.com:",
            "https://example.com:0",
            "https://example.com:abc",
            "https://example.com:123:456",
            "https://example.com:65536",
            "https://[::1]:0/path",
            "https://[::1]:abc/path",
            "https://[::1]:123:456/path",
            "https://[not-ipv6]/path",
            "https://example..com/path",
            "https://-bad.example.com/path",
            "https://a\"b.example.com/path",
            "https://example.com/\u{202E}evil",
            "https://example.com/\nhttps://evil.test",
            "https://example.com\\@evil.test",
        ] {
            assert!(external_url(value).is_none(), "{value}");
        }
    }
}
