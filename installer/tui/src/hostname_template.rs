use std::fmt;

use rand::Rng;

// r[impl installer.config.hostname-template]

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Segment {
    Literal(String),
    Hex(u32),
    Num(u32),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostnameTemplate {
    pub segments: Vec<Segment>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TemplateError {
    EmptyTemplate,
    NoPlaceholders,
    InvalidLiteralChar(char),
    UnknownPlaceholder(String),
    MissingColon(String),
    InvalidN(String),
    HexNOutOfRange(u32),
    NumNOutOfRange(u32),
    UnclosedBrace,
    EmptyBrace,
    LeadingHyphen,
    TrailingHyphen,
    ExceedsMaxLength(usize),
}

impl fmt::Display for TemplateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TemplateError::EmptyTemplate => write!(f, "hostname template is empty"),
            TemplateError::NoPlaceholders => {
                write!(f, "hostname template must contain at least one placeholder")
            }
            TemplateError::InvalidLiteralChar(c) => {
                write!(
                    f,
                    "hostname template contains invalid character '{c}' (only a-z, 0-9, - allowed)"
                )
            }
            TemplateError::UnknownPlaceholder(name) => {
                write!(
                    f,
                    "unknown placeholder type '{name}' (expected 'hex' or 'num')"
                )
            }
            TemplateError::MissingColon(content) => {
                write!(f, "placeholder '{{{content}}}' is missing ':N' parameter")
            }
            TemplateError::InvalidN(content) => {
                write!(
                    f,
                    "placeholder parameter is not a valid integer: '{content}'"
                )
            }
            TemplateError::HexNOutOfRange(n) => {
                write!(f, "hex:N parameter {n} is out of range (must be 1..=32)")
            }
            TemplateError::NumNOutOfRange(n) => {
                write!(f, "num:N parameter {n} is out of range (must be 1..=10)")
            }
            TemplateError::UnclosedBrace => write!(f, "unclosed '{{' in hostname template"),
            TemplateError::EmptyBrace => write!(f, "empty '{{}}' in hostname template"),
            TemplateError::LeadingHyphen => {
                write!(
                    f,
                    "hostname template would produce a hostname starting with a hyphen"
                )
            }
            TemplateError::TrailingHyphen => {
                write!(
                    f,
                    "hostname template would produce a hostname ending with a hyphen"
                )
            }
            TemplateError::ExceedsMaxLength(len) => {
                write!(f, "hostname template expands to {len} characters (max 63)")
            }
        }
    }
}

impl std::error::Error for TemplateError {}

pub fn parse(template: &str) -> Result<HostnameTemplate, TemplateError> {
    if template.is_empty() {
        return Err(TemplateError::EmptyTemplate);
    }

    let mut segments = Vec::new();
    let mut literal = String::new();
    let mut chars = template.chars().peekable();

    while let Some(&c) = chars.peek() {
        if c == '{' {
            chars.next();
            if !literal.is_empty() {
                segments.push(Segment::Literal(std::mem::take(&mut literal)));
            }
            let mut placeholder = String::new();
            let mut found_close = false;
            for ch in chars.by_ref() {
                if ch == '}' {
                    found_close = true;
                    break;
                }
                placeholder.push(ch);
            }
            if !found_close {
                return Err(TemplateError::UnclosedBrace);
            }
            if placeholder.is_empty() {
                return Err(TemplateError::EmptyBrace);
            }
            let segment = parse_placeholder(&placeholder)?;
            segments.push(segment);
        } else {
            chars.next();
            if !c.is_ascii_lowercase() && !c.is_ascii_digit() && c != '-' {
                return Err(TemplateError::InvalidLiteralChar(c));
            }
            literal.push(c);
        }
    }

    if !literal.is_empty() {
        segments.push(Segment::Literal(literal));
    }

    let has_placeholder = segments.iter().any(|s| !matches!(s, Segment::Literal(_)));
    if !has_placeholder {
        return Err(TemplateError::NoPlaceholders);
    }

    validate_template(&segments)?;

    Ok(HostnameTemplate { segments })
}

fn parse_placeholder(content: &str) -> Result<Segment, TemplateError> {
    let Some((name, n_str)) = content.split_once(':') else {
        return Err(TemplateError::MissingColon(content.to_string()));
    };

    let n: u32 = n_str
        .parse()
        .map_err(|_| TemplateError::InvalidN(n_str.to_string()))?;

    match name {
        "hex" => {
            if !(1..=32).contains(&n) {
                return Err(TemplateError::HexNOutOfRange(n));
            }
            Ok(Segment::Hex(n))
        }
        "num" => {
            if !(1..=10).contains(&n) {
                return Err(TemplateError::NumNOutOfRange(n));
            }
            Ok(Segment::Num(n))
        }
        other => Err(TemplateError::UnknownPlaceholder(other.to_string())),
    }
}

fn validate_template(segments: &[Segment]) -> Result<(), TemplateError> {
    let max_len: usize = segments
        .iter()
        .map(|s| match s {
            Segment::Literal(lit) => lit.len(),
            Segment::Hex(n) | Segment::Num(n) => *n as usize,
        })
        .sum();

    if max_len > 63 {
        return Err(TemplateError::ExceedsMaxLength(max_len));
    }

    if let Some(Segment::Literal(lit)) = segments.first()
        && lit.starts_with('-')
    {
        return Err(TemplateError::LeadingHyphen);
    }

    if let Some(Segment::Literal(lit)) = segments.last()
        && lit.ends_with('-')
    {
        return Err(TemplateError::TrailingHyphen);
    }

    Ok(())
}

pub fn resolve(template: &HostnameTemplate) -> String {
    let mut rng = rand::rng();
    resolve_with_rng(template, &mut rng)
}

fn resolve_with_rng(template: &HostnameTemplate, rng: &mut impl Rng) -> String {
    let mut result = String::new();

    for segment in &template.segments {
        match segment {
            Segment::Literal(lit) => result.push_str(lit),
            Segment::Hex(n) => {
                let n = *n as usize;
                for _ in 0..n {
                    let nibble: u8 = rng.random_range(0..16);
                    result.push(char::from(if nibble < 10 {
                        b'0' + nibble
                    } else {
                        b'a' + nibble - 10
                    }));
                }
            }
            Segment::Num(n) => {
                let n = *n as usize;
                let max_val = 10u64.pow(n as u32);
                let val: u64 = rng.random_range(0..max_val);
                let formatted = format!("{val:0>width$}", width = n);
                result.push_str(&formatted);
            }
        }
    }

    result
}

pub fn parse_and_resolve(template: &str) -> Result<String, TemplateError> {
    let parsed = parse(template)?;
    Ok(resolve(&parsed))
}

#[cfg(test)]
mod tests {
    use super::*;

    // r[verify installer.config.hostname-template]
    #[test]
    fn parse_simple_hex() {
        let t = parse("srv-{hex:6}").unwrap();
        assert_eq!(
            t.segments,
            vec![Segment::Literal("srv-".into()), Segment::Hex(6),]
        );
    }

    // r[verify installer.config.hostname-template]
    #[test]
    fn parse_simple_num() {
        let t = parse("node-{num:4}").unwrap();
        assert_eq!(
            t.segments,
            vec![Segment::Literal("node-".into()), Segment::Num(4),]
        );
    }

    // r[verify installer.config.hostname-template]
    #[test]
    fn parse_multiple_placeholders() {
        let t = parse("srv-{hex:4}-{num:3}").unwrap();
        assert_eq!(
            t.segments,
            vec![
                Segment::Literal("srv-".into()),
                Segment::Hex(4),
                Segment::Literal("-".into()),
                Segment::Num(3),
            ]
        );
    }

    // r[verify installer.config.hostname-template]
    #[test]
    fn parse_only_placeholder() {
        let t = parse("{hex:12}").unwrap();
        assert_eq!(t.segments, vec![Segment::Hex(12)]);
    }

    // r[verify installer.config.hostname-template]
    #[test]
    fn error_empty() {
        assert_eq!(parse(""), Err(TemplateError::EmptyTemplate));
    }

    // r[verify installer.config.hostname-template]
    #[test]
    fn error_no_placeholders() {
        assert_eq!(parse("just-a-hostname"), Err(TemplateError::NoPlaceholders));
    }

    // r[verify installer.config.hostname-template]
    #[test]
    fn error_invalid_literal_char() {
        assert_eq!(
            parse("SRV-{hex:4}"),
            Err(TemplateError::InvalidLiteralChar('S'))
        );
    }

    // r[verify installer.config.hostname-template]
    #[test]
    fn error_unknown_placeholder() {
        assert_eq!(
            parse("{foo:3}"),
            Err(TemplateError::UnknownPlaceholder("foo".into()))
        );
    }

    // r[verify installer.config.hostname-template]
    #[test]
    fn error_missing_colon() {
        assert_eq!(
            parse("{hex}"),
            Err(TemplateError::MissingColon("hex".into()))
        );
    }

    // r[verify installer.config.hostname-template]
    #[test]
    fn error_invalid_n() {
        assert_eq!(
            parse("{hex:abc}"),
            Err(TemplateError::InvalidN("abc".into()))
        );
    }

    // r[verify installer.config.hostname-template]
    #[test]
    fn error_hex_n_zero() {
        assert_eq!(parse("{hex:0}"), Err(TemplateError::HexNOutOfRange(0)));
    }

    // r[verify installer.config.hostname-template]
    #[test]
    fn error_hex_n_too_large() {
        assert_eq!(parse("{hex:33}"), Err(TemplateError::HexNOutOfRange(33)));
    }

    // r[verify installer.config.hostname-template]
    #[test]
    fn error_num_n_zero() {
        assert_eq!(parse("{num:0}"), Err(TemplateError::NumNOutOfRange(0)));
    }

    // r[verify installer.config.hostname-template]
    #[test]
    fn error_num_n_too_large() {
        assert_eq!(parse("{num:11}"), Err(TemplateError::NumNOutOfRange(11)));
    }

    // r[verify installer.config.hostname-template]
    #[test]
    fn error_unclosed_brace() {
        assert_eq!(parse("srv-{hex:4"), Err(TemplateError::UnclosedBrace));
    }

    // r[verify installer.config.hostname-template]
    #[test]
    fn error_empty_brace() {
        assert_eq!(parse("srv-{}"), Err(TemplateError::EmptyBrace));
    }

    // r[verify installer.config.hostname-template]
    #[test]
    fn error_leading_hyphen() {
        assert_eq!(parse("-{hex:4}"), Err(TemplateError::LeadingHyphen));
    }

    // r[verify installer.config.hostname-template]
    #[test]
    fn error_trailing_hyphen() {
        assert_eq!(parse("{hex:4}-"), Err(TemplateError::TrailingHyphen));
    }

    // r[verify installer.config.hostname-template]
    #[test]
    fn error_exceeds_max_length() {
        // 60-char literal + {hex:4} = 64 > 63
        let long_prefix = "a".repeat(60);
        let template = format!("{long_prefix}{{hex:4}}");
        let err = parse(&template).unwrap_err();
        assert!(matches!(err, TemplateError::ExceedsMaxLength(64)));
    }

    // r[verify installer.config.hostname-template]
    #[test]
    fn hex_boundary_values() {
        parse("{hex:1}").unwrap();
        parse("{hex:32}").unwrap();
    }

    // r[verify installer.config.hostname-template]
    #[test]
    fn num_boundary_values() {
        parse("{num:1}").unwrap();
        parse("{num:10}").unwrap();
    }

    // r[verify installer.config.hostname-template]
    #[test]
    fn resolve_hex_length() {
        let t = parse("srv-{hex:6}").unwrap();
        let result = resolve(&t);
        assert_eq!(result.len(), 10); // "srv-" (4) + 6
        assert!(result.starts_with("srv-"));
        let hex_part = &result[4..];
        assert!(hex_part.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(hex_part, hex_part.to_lowercase());
    }

    // r[verify installer.config.hostname-template]
    #[test]
    fn resolve_num_length_and_padding() {
        let t = parse("node-{num:4}").unwrap();
        let result = resolve(&t);
        assert_eq!(result.len(), 9); // "node-" (5) + 4
        assert!(result.starts_with("node-"));
        let num_part = &result[5..];
        assert!(num_part.chars().all(|c| c.is_ascii_digit()));
        assert_eq!(num_part.len(), 4);
    }

    // r[verify installer.config.hostname-template]
    #[test]
    fn resolve_multiple_placeholders() {
        let t = parse("srv-{hex:4}-{num:3}").unwrap();
        let result = resolve(&t);
        assert_eq!(result.len(), 12); // "srv-" (4) + 4 + "-" (1) + 3
        let parts: Vec<&str> = result.splitn(3, '-').collect();
        assert_eq!(parts[0], "srv");
        assert_eq!(parts[1].len(), 4);
        assert!(parts[1].chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(parts[2].len(), 3);
        assert!(parts[2].chars().all(|c| c.is_ascii_digit()));
    }

    // r[verify installer.config.hostname-template]
    #[test]
    fn resolve_produces_unique_results() {
        let t = parse("test-{hex:8}").unwrap();
        let r1 = resolve(&t);
        let r2 = resolve(&t);
        // Extremely unlikely to collide with 8 hex chars (2^32 possibilities)
        // but not impossible. If this flakes, the test is still useful.
        assert_ne!(
            r1, r2,
            "two successive resolves should differ (extremely unlikely collision)"
        );
    }

    // r[verify installer.config.hostname-template]
    #[test]
    fn parse_and_resolve_valid() {
        let result = parse_and_resolve("tamanu-{hex:6}").unwrap();
        assert_eq!(result.len(), 13);
        assert!(result.starts_with("tamanu-"));
    }

    // r[verify installer.config.hostname-template]
    #[test]
    fn parse_and_resolve_invalid() {
        let result = parse_and_resolve("");
        assert!(result.is_err());
    }

    // r[verify installer.config.hostname-template]
    #[test]
    fn max_length_exactly_63() {
        // 31-char literal + {hex:32} = 63, should be valid
        let prefix = "a".repeat(31);
        let template = format!("{prefix}{{hex:32}}");
        parse(&template).unwrap();
    }

    // r[verify installer.config.hostname-template]
    #[test]
    fn resolve_num_1_single_digit() {
        let t = parse("{num:1}").unwrap();
        let result = resolve(&t);
        assert_eq!(result.len(), 1);
        assert!(result.chars().all(|c| c.is_ascii_digit()));
    }

    // r[verify installer.config.hostname-template]
    #[test]
    fn resolve_hex_1_single_char() {
        let t = parse("{hex:1}").unwrap();
        let result = resolve(&t);
        assert_eq!(result.len(), 1);
        assert!(result.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn deterministic_resolve_with_seeded_rng() {
        use rand::SeedableRng;
        let t = parse("test-{hex:4}").unwrap();
        let mut rng1 = rand::rngs::StdRng::seed_from_u64(42);
        let mut rng2 = rand::rngs::StdRng::seed_from_u64(42);
        let r1 = resolve_with_rng(&t, &mut rng1);
        let r2 = resolve_with_rng(&t, &mut rng2);
        assert_eq!(r1, r2);
    }

    #[test]
    fn display_all_error_variants() {
        let errors = vec![
            TemplateError::EmptyTemplate,
            TemplateError::NoPlaceholders,
            TemplateError::InvalidLiteralChar('X'),
            TemplateError::UnknownPlaceholder("foo".into()),
            TemplateError::MissingColon("hex".into()),
            TemplateError::InvalidN("abc".into()),
            TemplateError::HexNOutOfRange(0),
            TemplateError::NumNOutOfRange(0),
            TemplateError::UnclosedBrace,
            TemplateError::EmptyBrace,
            TemplateError::LeadingHyphen,
            TemplateError::TrailingHyphen,
            TemplateError::ExceedsMaxLength(100),
        ];
        for e in errors {
            let msg = e.to_string();
            assert!(
                !msg.is_empty(),
                "error Display should produce non-empty string"
            );
        }
    }
}
