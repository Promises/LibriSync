//! Folder templates (DESIGN.md "Output layout").
//!
//! A template is a `/`-separated relative path with `{token}`s. Each component is
//! rendered and sanitised on its own, so a `/` inside a title can never create a
//! directory. Components that render empty are dropped, and separators left dangling
//! by an empty token (`{seriesSequence} - {title}` without a sequence) are tidied away.

use std::path::PathBuf;

pub const DEFAULT_SERIES_TEMPLATE: &str = "{author}/{series}/{seriesSequence} - {title}";
pub const DEFAULT_TEMPLATE: &str = "{author}/{title}";

pub const TOKENS: &[&str] = &[
    "author",
    "authors",
    "title",
    "subtitle",
    "series",
    "seriesSequence",
    "year",
    "narrator",
    "asin",
    "provider",
];

/// Longest path component, in bytes (many filesystems cap names at 255 bytes; this
/// leaves room for the file name inside the folder and for multi-byte titles).
pub const MAX_COMPONENT_BYTES: usize = 120;
const MAX_COMPONENTS: usize = 8;

/// Values for one book. Empty strings mean "not known".
#[derive(Debug, Clone, Default)]
pub struct TemplateValues {
    pub authors: Vec<String>,
    pub narrators: Vec<String>,
    pub title: String,
    pub subtitle: String,
    pub series: String,
    pub series_sequence: String,
    pub year: String,
    pub asin: String,
    pub provider: String,
}

impl TemplateValues {
    fn get(&self, token: &str) -> String {
        match token {
            "author" => self
                .authors
                .first()
                .cloned()
                .unwrap_or_else(|| "Unknown Author".to_string()),
            "authors" => {
                if self.authors.is_empty() {
                    "Unknown Author".to_string()
                } else {
                    self.authors.join(", ")
                }
            }
            "title" => fallback_title(self),
            "subtitle" => self.subtitle.clone(),
            "series" => self.series.clone(),
            "seriesSequence" => self.series_sequence.clone(),
            "year" => self.year.clone(),
            "narrator" => self.narrators.join(", "),
            "asin" => self.asin.clone(),
            "provider" => self.provider.clone(),
            _ => String::new(),
        }
    }
}

/// Split a template component into literal text and token names.
enum Piece<'a> {
    Text(&'a str),
    Token(&'a str),
}

fn pieces(component: &str) -> Result<Vec<Piece<'_>>, String> {
    let mut out = Vec::new();
    let mut rest = component;
    while !rest.is_empty() {
        match (rest.find('{'), rest.find('}')) {
            (Some(open), close) if close.is_none_or(|c| c > open) => {
                if open > 0 {
                    out.push(Piece::Text(&rest[..open]));
                }
                let after = &rest[open + 1..];
                let end = after.find('}').ok_or("unclosed '{'")?;
                let name = &after[..end];
                if name.contains('{') {
                    return Err("unclosed '{'".into());
                }
                if !TOKENS.contains(&name) {
                    return Err(format!("unknown token {{{name}}}"));
                }
                out.push(Piece::Token(name));
                rest = &after[end + 1..];
            }
            (None, None) => {
                out.push(Piece::Text(rest));
                rest = "";
            }
            _ => return Err("unmatched '}'".into()),
        }
    }
    Ok(out)
}

pub fn validate(template: &str) -> Result<(), String> {
    let t = template.trim();
    if t.is_empty() {
        return Err("must not be empty".into());
    }
    if t.starts_with('/') || t.contains('\\') {
        return Err("must be a relative path using '/'".into());
    }
    let components: Vec<&str> = t.split('/').collect();
    if components.len() > MAX_COMPONENTS {
        return Err(format!("at most {MAX_COMPONENTS} folder levels"));
    }
    let mut has_title = false;
    for c in components {
        let c = c.trim();
        if c.is_empty() {
            return Err("empty folder level ('//')".into());
        }
        if c == "." || c == ".." {
            return Err("'.' and '..' are not allowed".into());
        }
        for p in pieces(c)? {
            if let Piece::Token("title") = p {
                has_title = true;
            }
        }
    }
    if !has_title {
        return Err("must contain {title}, or every book would share one folder".into());
    }
    Ok(())
}

/// Render `template` into a relative path. Falls back to the sanitised title when
/// every component renders empty (or the template is invalid, which validation
/// prevents).
pub fn render(template: &str, v: &TemplateValues) -> PathBuf {
    let mut path = PathBuf::new();
    for component in template.trim().split('/') {
        let Ok(parts) = pieces(component) else { continue };
        let mut raw = String::new();
        for p in parts {
            match p {
                Piece::Text(t) => raw.push_str(t),
                Piece::Token(name) => raw.push_str(&sanitize_component(&v.get(name))),
            }
        }
        let clean = sanitize_component(&tidy(&raw));
        if !clean.is_empty() {
            path.push(clean);
        }
    }
    if path.as_os_str().is_empty() {
        path.push(fallback_title(v));
    }
    path
}

fn fallback_title(v: &TemplateValues) -> String {
    let t = sanitize_component(&v.title);
    if t.is_empty() {
        "Unknown Title".into()
    } else {
        t
    }
}

/// The file stem for single-file output (`{title}.m4b`).
pub fn file_stem(v: &TemplateValues) -> String {
    fallback_title(v)
}

/// Remove what an empty token leaves behind: doubled or dangling ` - ` separators and
/// empty brackets.
fn tidy(s: &str) -> String {
    let mut out = collapse_spaces(s);
    loop {
        let before = out.clone();
        for empty in ["[]", "()", "{}", "[ ]", "( )"] {
            out = out.replace(empty, "");
        }
        out = out.replace(" - - ", " - ");
        out = collapse_spaces(&out);
        if out == before {
            break;
        }
    }
    out.trim_matches(|c: char| c.is_whitespace() || c == '-' || c == ',' || c == '_')
        .to_string()
}

fn collapse_spaces(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Make one path component safe: no `/ \ : * ? " < > |` or control characters, no
/// leading dots (hidden files), no trailing dots/spaces, at most
/// [`MAX_COMPONENT_BYTES`] bytes.
pub fn sanitize_component(s: &str) -> String {
    let s = s.replace(": ", " - ");
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '/' | '\\' | ':' => out.push('-'),
            '*' | '?' | '"' | '<' | '>' | '|' => {}
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    let mut out = collapse_spaces(&out);
    out = out.trim_start_matches(['.', ' ']).to_string();
    if out.len() > MAX_COMPONENT_BYTES {
        let mut end = MAX_COMPONENT_BYTES;
        while !out.is_char_boundary(end) {
            end -= 1;
        }
        out.truncate(end);
    }
    out.trim_end_matches(['.', ' ']).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn book() -> TemplateValues {
        TemplateValues {
            authors: vec!["Brandon Sanderson".into(), "Someone Else".into()],
            narrators: vec!["Michael Kramer".into()],
            title: "The Final Empire".into(),
            subtitle: String::new(),
            series: "Mistborn".into(),
            series_sequence: "1".into(),
            year: "2006".into(),
            asin: "B002UZMLXM".into(),
            provider: "audible".into(),
        }
    }

    #[test]
    fn renders_the_default_templates() {
        assert_eq!(
            render(DEFAULT_SERIES_TEMPLATE, &book()),
            PathBuf::from("Brandon Sanderson/Mistborn/1 - The Final Empire")
        );
        assert_eq!(
            render(DEFAULT_TEMPLATE, &book()),
            PathBuf::from("Brandon Sanderson/The Final Empire")
        );
    }

    #[test]
    fn empty_tokens_leave_no_dangling_separators() {
        let mut b = book();
        b.series_sequence.clear();
        assert_eq!(
            render(DEFAULT_SERIES_TEMPLATE, &b),
            PathBuf::from("Brandon Sanderson/Mistborn/The Final Empire")
        );
        assert_eq!(
            render("{author}/{year} - {seriesSequence} - {title} [{asin}]", &TemplateValues {
                asin: String::new(),
                ..b.clone()
            }),
            PathBuf::from("Brandon Sanderson/2006 - The Final Empire")
        );
        // A component that is only an empty token disappears.
        b.series.clear();
        assert_eq!(
            render("{author}/{series}/{title}", &b),
            PathBuf::from("Brandon Sanderson/The Final Empire")
        );
    }

    #[test]
    fn values_cannot_create_directories_or_escape() {
        let mut b = book();
        b.title = "../../etc/passwd".into();
        b.authors = vec!["AC/DC".into()];
        let p = render(DEFAULT_TEMPLATE, &b);
        assert_eq!(p.components().count(), 2);
        assert!(p.starts_with("AC-DC"));
        assert!(p.components().all(|c| matches!(c, std::path::Component::Normal(_))));
        assert!(!p.to_string_lossy().contains("/.."));
    }

    #[test]
    fn sanitises_reserved_characters_and_trailing_dots() {
        assert_eq!(sanitize_component("Dune: Messiah"), "Dune - Messiah");
        assert_eq!(sanitize_component("What? <Really> \"yes\" | *no*"), "What Really yes no");
        assert_eq!(sanitize_component("Ends with dots... "), "Ends with dots");
        assert_eq!(sanitize_component(".hidden"), "hidden");
        assert_eq!(sanitize_component("tab\there"), "tabhere");
        assert_eq!(sanitize_component(".."), "");
    }

    #[test]
    fn truncates_to_the_byte_limit_on_a_char_boundary() {
        let long = "ø".repeat(100); // 200 bytes
        let s = sanitize_component(&long);
        assert!(s.len() <= MAX_COMPONENT_BYTES);
        assert!(s.chars().all(|c| c == 'ø'));
        let spaced = format!("{} end", "a".repeat(MAX_COMPONENT_BYTES - 1));
        assert!(!sanitize_component(&spaced).ends_with(' '));
    }

    #[test]
    fn missing_author_and_title_have_fallbacks() {
        let b = TemplateValues::default();
        assert_eq!(render(DEFAULT_TEMPLATE, &b), PathBuf::from("Unknown Author/Unknown Title"));
        assert_eq!(file_stem(&b), "Unknown Title");
    }

    #[test]
    fn validation() {
        assert!(validate(DEFAULT_SERIES_TEMPLATE).is_ok());
        assert!(validate(DEFAULT_TEMPLATE).is_ok());
        assert!(validate("{authors} - {title} {{narrator}}").is_err());
        assert!(validate("{author}/{bogus}/{title}").is_err());
        assert!(validate("/abs/{title}").is_err());
        assert!(validate("{author}/../{title}").is_err());
        assert!(validate("{author}//{title}").is_err());
        assert!(validate("{author}").is_err());
        assert!(validate("{title").is_err());
        assert!(validate("title}").is_err());
        assert!(validate("").is_err());
    }
}
