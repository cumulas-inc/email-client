//! Handling of untrusted HTML from email bodies.
//!
//! Defense in depth: bodies are sanitized here (no scripts, event handlers,
//! forms, frames or `javascript:` URLs), then the UI renders them inside a
//! sandboxed `<iframe>` without `allow-scripts`, with a Content Security
//! Policy that blocks remote loads unless the user allowed them.

use std::borrow::Cow;
use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use url::Url;

use crate::error::{Error, Result};

/// The largest signature taken, the person's own or an organization's.
pub const MAX_SIGNATURE_BYTES: usize = 512 * 1024;

/// Result of [`sanitize_email_html`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SanitizedHtml {
    pub html: String,
    /// The original referenced remote images or stylesheets that were removed
    /// because `allow_remote` was false.
    pub has_blocked_content: bool,
}

fn is_remote_url(value: &str) -> bool {
    let v = value.trim_start().to_ascii_lowercase();
    v.starts_with("http:") || v.starts_with("https:") || v.starts_with("//")
}

fn css_has_remote_url(css: &str) -> bool {
    let lower = css.to_ascii_lowercase();
    lower.match_indices("url(").any(|(i, _)| {
        let rest = lower[i + 4..].trim_start_matches([' ', '"', '\'']);
        is_remote_url(rest)
    }) || lower.contains("@import")
}

/// Sanitizes an HTML email body for display.
pub fn sanitize_email_html(html: &str, allow_remote: bool) -> SanitizedHtml {
    let blocked = Arc::new(AtomicBool::new(false));
    let blocked_in_filter = Arc::clone(&blocked);

    let generic_attributes: HashSet<&str> = LAYOUT_ATTRIBUTES.iter().copied().collect();

    let cleaned = ammonia::Builder::default()
        .add_tags(["style", "center", "font"])
        .rm_clean_content_tags(["style"])
        .add_generic_attributes(generic_attributes)
        .add_tag_attributes("img", ["src", "alt", "width", "height"])
        .add_tag_attributes("a", ["href", "name", "target"])
        .url_schemes(["http", "https", "mailto", "tel", "data", "cid"].into())
        .link_rel(Some("noopener noreferrer"))
        .attribute_filter(move |element, attribute, value| {
            match (element, attribute) {
                // data: URLs are only acceptable as inline images.
                (_, "href") if value.trim_start().to_ascii_lowercase().starts_with("data:") => None,
                ("img", "src") | (_, "background") if is_remote_url(value) => {
                    if allow_remote {
                        Some(value.into())
                    } else {
                        blocked_in_filter.store(true, Ordering::Relaxed);
                        None
                    }
                }
                ("img", "src")
                    if value.trim_start().to_ascii_lowercase().starts_with("data:")
                        && !value
                            .trim_start()
                            .to_ascii_lowercase()
                            .starts_with("data:image/") =>
                {
                    None
                }
                (_, "style") if css_has_remote_url(value) => {
                    if !allow_remote {
                        blocked_in_filter.store(true, Ordering::Relaxed);
                    }
                    // The rendering CSP blocks the actual load when not allowed.
                    Some(value.into())
                }
                _ => Some(value.into()),
            }
        })
        .clean(html)
        .to_string();

    // Remote loads from <style> blocks are blocked by the CSP; only flag them.
    let style_remote = !allow_remote && css_has_remote_url(&style_blocks(html));

    SanitizedHtml {
        html: cleaned,
        has_blocked_content: blocked.load(Ordering::Relaxed) || style_remote,
    }
}

/// Attributes kept on any element, in bodies and signatures alike.
const LAYOUT_ATTRIBUTES: &[&str] = &[
    "style",
    "class",
    "id",
    "align",
    "valign",
    "width",
    "height",
    "bgcolor",
    "border",
    "cellpadding",
    "cellspacing",
    "color",
    "face",
    "size",
    "dir",
    "lang",
    "title",
    "role",
    "colspan",
    "rowspan",
    "nowrap",
    "background",
];

/// Sanitizes a signature for the compose editor and the messages it goes
/// out in: the person's own (`base` is `None`) or their organization's
/// (`base` is the signature service, which relative links point to).
///
/// It keeps what the email allowlist keeps (tables, inline styles, `font`
/// and `center`), with stricter rules for what loads or opens: images and
/// CSS backgrounds only from `https:` or `data:image/*`, links only to
/// `http`, `https`, `mailto` and `tel`. Relative URLs are resolved against
/// `base`, or dropped without one. A declaration of a `style` attribute that
/// loads something else, or could run code, goes; so does a `<style>` block
/// with `@import`, `expression(`, `javascript:`, `behavior:` or a `url()`
/// other than `https:` or `data:`. Comments go too, and with them the
/// conditional comments Outlook writes for its own font fallbacks (Word's
/// `<!--[if mso]>` blocks), which nothing else reads.
///
/// More than [`MAX_SIGNATURE_BYTES`] is refused with `InvalidInput`.
pub fn sanitize_signature_html(html: &str, base: Option<&Url>) -> Result<String> {
    if html.len() > MAX_SIGNATURE_BYTES {
        return Err(Error::invalid_input(
            "That signature is too large. Signatures can be 512 KB at most.",
        ));
    }
    let base = base.cloned();
    let cleaned = ammonia::Builder::default()
        .add_tags(["style", "center", "font"])
        .rm_clean_content_tags(["style"])
        .add_generic_attributes(LAYOUT_ATTRIBUTES.iter().copied())
        .add_tag_attributes("img", ["src", "alt", "width", "height"])
        .add_tag_attributes("a", ["href", "name", "target"])
        // Narrowed per attribute by the filter below.
        .url_schemes(["http", "https", "mailto", "tel", "data"].into())
        // The filter resolves or drops relative URLs itself.
        .url_relative(ammonia::UrlRelative::PassThrough)
        .link_rel(Some("noopener noreferrer"))
        .strip_comments(true)
        .attribute_filter(move |element, attribute, value| {
            signature_attribute(element, attribute, value, base.as_ref())
        })
        .clean(html)
        .to_string();
    Ok(drop_unsafe_style_blocks(&cleaned))
}

fn signature_attribute<'a>(
    element: &str,
    attribute: &str,
    value: &'a str,
    base: Option<&Url>,
) -> Option<Cow<'a, str>> {
    match (element, attribute) {
        ("a", "href") => {
            let (url, relative) = resolve_url(value, base)?;
            matches!(url.scheme(), "http" | "https" | "mailto" | "tel")
                .then(|| keep_or_resolved(value, url, relative))
        }
        ("img", "src") | (_, "background") => {
            let (url, relative) = resolve_url(value, base)?;
            is_signature_image(&url).then(|| keep_or_resolved(value, url, relative))
        }
        (_, "style") => clean_inline_style(value),
        _ => Some(Cow::Borrowed(value)),
    }
}

/// The URL in an attribute, resolved against `base` when it's relative,
/// and whether it was.
fn resolve_url(value: &str, base: Option<&Url>) -> Option<(Url, bool)> {
    let value = value.trim();
    match Url::parse(value) {
        Ok(url) => Some((url, false)),
        Err(url::ParseError::RelativeUrlWithoutBase) => {
            base?.join(value).ok().map(|url| (url, true))
        }
        Err(_) => None,
    }
}

fn keep_or_resolved(value: &str, url: Url, relative: bool) -> Cow<'_, str> {
    if relative {
        Cow::Owned(url.into())
    } else {
        Cow::Borrowed(value)
    }
}

fn is_signature_image(url: &Url) -> bool {
    match url.scheme() {
        "https" => true,
        "data" => url.path().to_ascii_lowercase().starts_with("image/"),
        _ => false,
    }
}

/// What in CSS can run code or load another stylesheet, once comments,
/// escapes and whitespace are gone (see [`normalize_css`]).
const UNSAFE_CSS: &[&str] = &[
    "@import",
    "expression(",
    "javascript:",
    "vbscript:",
    "behavior:",
    "-moz-binding",
];

/// A `style` attribute without the declarations that load from anywhere
/// but `https:` or `data:image/*`, or could run code; `None` when nothing
/// is left.
fn clean_inline_style(style: &str) -> Option<Cow<'_, str>> {
    let declarations = split_declarations(style);
    let kept: Vec<&str> = declarations
        .iter()
        .copied()
        .filter(|declaration| {
            css_is_safe(declaration, |url| {
                url.starts_with("https://") || url.starts_with("data:image/")
            })
        })
        .collect();
    if kept.len() == declarations.len() {
        return Some(Cow::Borrowed(style));
    }
    (!kept.is_empty()).then(|| Cow::Owned(kept.join("; ")))
}

/// Splits a declaration list at the semicolons outside strings and
/// parentheses (a `data:` URL has its own), dropping empty ones.
fn split_declarations(style: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let (mut start, mut depth, mut quote, mut escaped) = (0, 0usize, None, false);
    for (i, c) in style.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match (c, quote) {
            ('\\', _) => escaped = true,
            (q, Some(open)) if q == open => quote = None,
            (_, Some(_)) => {}
            ('"' | '\'', None) => quote = Some(c),
            ('(', None) => depth += 1,
            (')', None) => depth = depth.saturating_sub(1),
            (';', None) if depth == 0 => {
                out.push(&style[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(&style[start..]);
    out.into_iter()
        .map(str::trim)
        .filter(|d| !d.is_empty())
        .collect()
}

/// Whether CSS is free of [`UNSAFE_CSS`] and every `url()` in it passes
/// `url_ok` (given the URL lowercased, unquoted and without whitespace).
fn css_is_safe(css: &str, url_ok: impl Fn(&str) -> bool) -> bool {
    let css = normalize_css(css);
    if UNSAFE_CSS.iter().any(|token| css.contains(token)) {
        return false;
    }
    css.match_indices("url(").all(|(at, prefix)| {
        let rest = &css[at + prefix.len()..];
        let url = rest[..rest.find(')').unwrap_or(rest.len())].trim_matches(['"', '\'']);
        url_ok(url)
    })
}

/// CSS as a browser reads it, for finding what it would do: lowercased,
/// without comments or whitespace, with escapes (`\69 mport`) decoded.
fn normalize_css(css: &str) -> String {
    let mut out = String::with_capacity(css.len());
    let mut chars = css.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '/' if chars.peek() == Some(&'*') => {
                chars.next();
                let mut previous = '\0';
                for c in chars.by_ref() {
                    if previous == '*' && c == '/' {
                        break;
                    }
                    previous = c;
                }
            }
            '\\' => {
                let mut hex = String::new();
                while hex.len() < 6 && chars.peek().is_some_and(char::is_ascii_hexdigit) {
                    hex.extend(chars.next());
                }
                if hex.is_empty() {
                    // An escaped character stands for itself; an escaped
                    // newline for nothing.
                    if let Some(next) = chars.next().filter(|c| *c != '\n') {
                        out.extend(next.to_lowercase());
                    }
                } else {
                    if chars.peek().is_some_and(|c| c.is_whitespace()) {
                        chars.next();
                    }
                    let decoded = u32::from_str_radix(&hex, 16)
                        .ok()
                        .and_then(char::from_u32)
                        .unwrap_or('\u{fffd}');
                    out.extend(decoded.to_lowercase());
                }
            }
            c if c.is_whitespace() => {}
            c => out.extend(c.to_lowercase()),
        }
    }
    out
}

/// Removes the `<style>` blocks of sanitized HTML whose CSS isn't safe
/// (see [`sanitize_signature_html`]). The serializer escapes `<` in text
/// and attribute values, so a `<style` is always the element's start tag,
/// and the end tag is always `</style>`, which its text can't contain.
fn drop_unsafe_style_blocks(html: &str) -> String {
    const OPEN: &str = "<style";
    const CLOSE: &str = "</style>";
    let lower = html.to_ascii_lowercase();
    let mut out = String::with_capacity(html.len());
    let mut pos = 0;
    while let Some(found) = lower[pos..].find(OPEN) {
        let start = pos + found;
        let after = lower.as_bytes().get(start + OPEN.len());
        if !matches!(after, Some(b'>' | b' ')) {
            // Another element (none is allowed, but stay exact).
            out.push_str(&html[pos..start + OPEN.len()]);
            pos = start + OPEN.len();
            continue;
        }
        let Some(open_end) = lower[start..].find('>').map(|e| start + e + 1) else {
            break;
        };
        let (css_end, end) = match lower[open_end..].find(CLOSE) {
            Some(e) => (open_end + e, open_end + e + CLOSE.len()),
            None => (html.len(), html.len()),
        };
        out.push_str(&html[pos..start]);
        let safe = css_is_safe(&html[open_end..css_end], |url| {
            url.starts_with("https://") || url.starts_with("data:")
        });
        if safe {
            out.push_str(&html[start..end]);
        }
        pos = end;
    }
    out.push_str(&html[pos..]);
    out
}

fn style_blocks(html: &str) -> String {
    let lower = html.to_ascii_lowercase();
    let mut out = String::new();
    let mut pos = 0;
    while let Some(start) = lower[pos..].find("<style") {
        let start = pos + start;
        let Some(end) = lower[start..].find("</style") else {
            out.push_str(&lower[start..]);
            break;
        };
        out.push_str(&lower[start..start + end]);
        pos = start + end + 7;
    }
    out
}

/// Converts HTML to readable plain text (for previews, search and replies).
pub fn html_to_text(html: &str) -> String {
    let mut out = String::with_capacity(html.len() / 2);
    let mut chars = html.char_indices().peekable();
    let lower = html.to_ascii_lowercase();
    let mut skip_until: Option<&str> = None;

    while let Some((i, c)) = chars.next() {
        if let Some(end_tag) = skip_until {
            if c == '<' && lower[i..].starts_with(end_tag) {
                skip_until = None;
            } else {
                continue;
            }
        }
        match c {
            '<' => {
                let rest = &lower[i..];
                let tag_end = rest.find('>').map(|e| i + e).unwrap_or(html.len());
                let tag = &lower[i + 1..tag_end];
                let name: String = tag
                    .trim_start_matches('/')
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric())
                    .collect();
                if !tag.starts_with('/') {
                    match name.as_str() {
                        "script" => skip_until = Some("</script"),
                        "style" => skip_until = Some("</style"),
                        "head" => skip_until = Some("</head"),
                        "title" => skip_until = Some("</title"),
                        _ => {}
                    }
                }
                if matches!(
                    name.as_str(),
                    "br" | "p"
                        | "div"
                        | "tr"
                        | "li"
                        | "h1"
                        | "h2"
                        | "h3"
                        | "h4"
                        | "h5"
                        | "h6"
                        | "blockquote"
                        | "table"
                        | "hr"
                ) {
                    out.push('\n');
                } else if matches!(name.as_str(), "td" | "th") {
                    out.push(' ');
                }
                while let Some((j, _)) = chars.peek() {
                    if *j > tag_end {
                        break;
                    }
                    chars.next();
                }
            }
            '&' => {
                let rest = &html[i..];
                // Entities are short; look for ';' within the next few chars.
                let semi = rest
                    .char_indices()
                    .take(12)
                    .find(|(_, c)| *c == ';')
                    .map(|(at, _)| at);
                if let Some(decoded) = semi.and_then(|semi| decode_entity(&rest[1..semi])) {
                    out.push(decoded);
                    let consumed = rest[..semi.unwrap_or(0)].chars().count();
                    for _ in 0..consumed {
                        chars.next();
                    }
                    continue;
                }
                out.push('&');
            }
            _ => out.push(c),
        }
    }
    normalize_whitespace(&out)
}

fn decode_entity(entity: &str) -> Option<char> {
    match entity {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" | "#39" => Some('\''),
        "nbsp" => Some(' '),
        "zwnj" | "zwj" | "#8203" | "#847" => Some('\u{200b}'),
        e if e.starts_with("#x") || e.starts_with("#X") => u32::from_str_radix(&e[2..], 16)
            .ok()
            .and_then(char::from_u32),
        e if e.starts_with('#') => e[1..].parse().ok().and_then(char::from_u32),
        _ => None,
    }
}

/// Collapses runs of spaces within lines and limits blank lines to one.
fn normalize_whitespace(text: &str) -> String {
    let mut lines = Vec::new();
    let mut blank = false;
    for line in text.lines() {
        let collapsed = line
            .split(|c: char| c.is_whitespace() || c == '\u{200b}')
            .filter(|w| !w.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        if collapsed.is_empty() {
            if !blank && !lines.is_empty() {
                lines.push(String::new());
            }
            blank = true;
        } else {
            lines.push(collapsed);
            blank = false;
        }
    }
    while lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }
    lines.join("\n")
}

/// A one-line preview of at most `max_chars` characters.
pub fn make_preview(text: &str, max_chars: usize) -> String {
    let one_line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.chars().count() <= max_chars {
        return one_line;
    }
    let mut cut: String = one_line.chars().take(max_chars).collect();
    if let Some(space) = cut.rfind(' ') {
        cut.truncate(space);
    }
    cut.push('…');
    cut
}

/// Escapes text for inclusion in HTML.
pub fn escape_html(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

/// Renders a plain-text body as simple HTML, preserving line breaks.
pub fn text_to_html(text: &str) -> String {
    format!(
        "<div style=\"white-space: pre-wrap\">{}</div>",
        escape_html(text)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_scripts_handlers_and_dangerous_urls() {
        let out = sanitize_email_html(
            r#"<p onclick="evil()">Hi<script>alert(1)</script></p>
               <a href="javascript:alert(1)">x</a><a href="data:text/html,hi">y</a>
               <iframe src="https://evil"></iframe><form><input></form>"#,
            false,
        );
        assert!(!out.html.contains("script"));
        assert!(!out.html.contains("onclick"));
        assert!(!out.html.contains("javascript"));
        assert!(!out.html.contains("data:text"));
        assert!(!out.html.contains("iframe"));
        assert!(!out.html.contains("<form"));
        assert!(out.html.contains("Hi"));
    }

    #[test]
    fn blocks_remote_images_unless_allowed() {
        let html = r#"<img src="https://tracker.example/pixel.gif"><img src="data:image/png;base64,AAAA">"#;
        let blocked = sanitize_email_html(html, false);
        assert!(blocked.has_blocked_content);
        assert!(!blocked.html.contains("tracker"));
        assert!(blocked.html.contains("data:image/png"));

        let allowed = sanitize_email_html(html, true);
        assert!(!allowed.has_blocked_content);
        assert!(allowed.html.contains("tracker"));
    }

    #[test]
    fn keeps_email_layout_and_flags_remote_css() {
        let html = r##"<style>.x { background: url('https://cdn.example/bg.png') }</style>
            <table width="600" bgcolor="#fff"><tr><td style="color: red">Cell</td></tr></table>"##;
        let out = sanitize_email_html(html, false);
        assert!(out.html.contains("<style>"));
        assert!(out.html.contains(r#"width="600""#));
        assert!(out.html.contains("color: red"));
        assert!(out.has_blocked_content);
    }

    #[test]
    fn html_to_text_drops_markup_and_decodes_entities() {
        let text = html_to_text(
            "<html><head><title>T</title><style>p{}</style></head><body>\
             <p>Hello&nbsp;<b>world</b> &amp; friends</p><p>Second&#33;</p>\
             <script>x()</script></body></html>",
        );
        assert_eq!(text, "Hello world & friends\n\nSecond!");
    }

    #[test]
    fn html_to_text_handles_non_ascii_and_broken_markup() {
        assert_eq!(html_to_text("caf&eacute; &é; Zoë"), "caf&eacute; &é; Zoë");
        assert_eq!(html_to_text("&ééééééééééééé"), "&ééééééééééééé");
        assert_eq!(html_to_text("a <é"), "a");
        assert_eq!(html_to_text("x &#233; y"), "x é y");
    }

    #[test]
    fn preview_truncates_on_word_boundary() {
        assert_eq!(make_preview("one two three", 100), "one two three");
        assert_eq!(make_preview("one two three", 9), "one two…");
    }

    fn signature(html: &str) -> String {
        sanitize_signature_html(html, None).unwrap()
    }

    fn organization_signature(html: &str) -> String {
        let base = Url::parse("https://signature.example.org/").unwrap();
        sanitize_signature_html(html, Some(&base)).unwrap()
    }

    #[test]
    fn signatures_keep_their_layout_and_lose_scripts() {
        let out = signature(
            r##"<table cellpadding="0" style="border-collapse: collapse"><tr>
                <td style="font-family: Georgia, serif; color: #1a2b3c" width="120">
                <font face="Arial" color="red">Ada Lovelace</font><center>CEO</center></td>
                </tr></table>
                <p onclick="evil()">Hi<script>alert(1)</script></p>
                <iframe src="https://evil.example"></iframe><form><input></form>"##,
        );
        assert!(out.contains(r#"cellpadding="0""#), "{out}");
        assert!(out.contains("border-collapse: collapse"), "{out}");
        assert!(
            out.contains("font-family: Georgia, serif; color: #1a2b3c"),
            "{out}"
        );
        assert!(out.contains(r#"<font face="Arial" color="red">"#), "{out}");
        assert!(out.contains("<center>CEO</center>"), "{out}");
        for gone in ["script", "onclick", "iframe", "<form", "<input"] {
            assert!(!out.contains(gone), "{gone} in {out}");
        }
    }

    #[test]
    fn signature_images_come_from_https_or_data_images_only() {
        let out = signature(
            r#"<img src="https://cdn.example/logo.png" alt="Logo">
               <img src="http://cdn.example/plain.png">
               <img src="data:image/png;base64,iVBORw0KGgo=">
               <img src="data:text/html;base64,PHNjcmlwdD4=">
               <img src="javascript:alert(1)">
               <img src="cid:logo@example">
               <img src="/relative.png">
               <table background="https://cdn.example/bg.png"><tr><td background="http://cdn.example/bg.png">x</td></tr></table>"#,
        );
        assert!(
            out.contains(r#"src="https://cdn.example/logo.png""#),
            "{out}"
        );
        assert!(
            out.contains(r#"src="data:image/png;base64,iVBORw0KGgo=""#),
            "{out}"
        );
        assert!(
            out.contains(r#"background="https://cdn.example/bg.png""#),
            "{out}"
        );
        for gone in [
            "http://cdn.example",
            "data:text",
            "javascript",
            "cid:",
            "relative.png",
        ] {
            assert!(!out.contains(gone), "{gone} in {out}");
        }
        // The images stay; the five refused sources go.
        assert_eq!(out.matches("<img").count(), 7, "{out}");
        assert_eq!(out.matches("src=").count(), 2, "{out}");
    }

    #[test]
    fn signature_links_open_the_web_mail_or_phone_only() {
        let out = signature(
            r#"<a href="https://example.com/">web</a><a href="http://example.com/">plain</a>
               <a href="mailto:ada@example.com">mail</a><a href="tel:+15550100">call</a>
               <a href="javascript:alert(1)">js</a><a href="JaVaScRiPt:alert(1)">js2</a>
               <a href="data:text/html,hi">data</a><a href="/about">relative</a>
               <a href="ftp://example.com/">ftp</a>"#,
        );
        for kept in [
            r#"href="https://example.com/""#,
            r#"href="http://example.com/""#,
            r#"href="mailto:ada@example.com""#,
            r#"href="tel:+15550100""#,
        ] {
            assert!(out.contains(kept), "{kept} not in {out}");
        }
        for gone in ["javascript", "JaVaScRiPt", "data:", "/about", "ftp:"] {
            assert!(!out.contains(gone), "{gone} in {out}");
        }
        assert!(out.contains(r#"rel="noopener noreferrer""#), "{out}");
    }

    #[test]
    fn relative_urls_resolve_against_the_signature_service() {
        let out = organization_signature(
            r#"<a href="/people/ada">Ada</a><img src="logo.png">
               <table><tr><td background="/bg.png">x</td></tr></table>"#,
        );
        assert!(
            out.contains(r#"href="https://signature.example.org/people/ada""#),
            "{out}"
        );
        assert!(
            out.contains(r#"src="https://signature.example.org/logo.png""#),
            "{out}"
        );
        assert!(
            out.contains(r#"background="https://signature.example.org/bg.png""#),
            "{out}"
        );
        // Without a base (the person's own signature), they go.
        let own = signature(r#"<a href="/people/ada">Ada</a><img src="logo.png">"#);
        assert!(!own.contains("href"), "{own}");
        assert!(!own.contains("src"), "{own}");
    }

    #[test]
    fn inline_styles_lose_only_what_loads_or_runs() {
        let out = signature(
            r#"<table><tr><td style="color: red; background: url('http://cdn.example/bg.png'); font-size: 12px">a</td>
               <td style="background-image: url(https://cdn.example/bg.png); color: blue">b</td>
               <td style="background: url(data:image/png;base64,iVBORw0KGgo=); color: green">c</td>
               <td style="background: url(data:text/css,x)">d</td>
               <td style="width: expression(alert(1)); color: navy">e</td>
               <td style="background: url(java\73 cript:alert(1)); margin: 0">f</td>
               <td style="behavior: url(x.htc); padding: 0">g</td>
               <td style="background: url(/relative.png)">h</td></tr></table>"#,
        );
        assert!(
            out.contains(r#"style="color: red; font-size: 12px""#),
            "{out}"
        );
        assert!(
            out.contains("background-image: url(https://cdn.example/bg.png); color: blue"),
            "{out}"
        );
        assert!(
            out.contains("url(data:image/png;base64,iVBORw0KGgo=); color: green"),
            "{out}"
        );
        assert!(out.contains(r#"style="color: navy""#), "{out}");
        assert!(out.contains(r#"style="margin: 0""#), "{out}");
        assert!(out.contains(r#"style="padding: 0""#), "{out}");
        for gone in [
            "http://cdn",
            "data:text",
            "expression",
            "cript",
            "behavior",
            "relative",
        ] {
            assert!(!out.contains(gone), "{gone} in {out}");
        }
        assert!(out.contains("<td>h</td>"), "{out}");
    }

    #[test]
    fn style_blocks_stay_unless_they_load_or_run_something() {
        let kept = signature(
            "<style>.sig td { font-family: Calibri, sans-serif; color: #333 }\n\
             .logo { background: url(https://cdn.example/logo.png) }\n\
             @font-face { font-family: X; src: url(data:font/woff2;base64,AAAA) }</style>\
             <p class=\"sig\">Ada</p>",
        );
        assert!(kept.contains("<style>.sig td { font-family"), "{kept}");
        assert!(kept.contains("url(https://cdn.example/logo.png)"), "{kept}");
        assert!(kept.contains("data:font/woff2"), "{kept}");

        for css in [
            "@import url(https://evil.example/x.css);",
            "@\\69mport 'https://evil.example/x.css';",
            "p { background: url(http://cdn.example/bg.png) }",
            "p { background: url('/relative.png') }",
            "p { width: expression(alert(1)) }",
            "p { background: url(javascript:alert(1)) }",
            "p { behavior: url(x.htc) }",
            "p { color: red } /* */ @im/**/port 'x.css';",
        ] {
            let out = signature(&format!("<style>{css}</style><p>Ada</p>"));
            assert_eq!(out, "<p>Ada</p>", "{css}");
        }

        // The block ends where a browser ends it: what follows a false end
        // tag is still its CSS.
        let out =
            signature("<style>p{}</stylex>@import 'https://evil.example/x.css';</style><p>Ada</p>");
        assert!(!out.contains("import"), "{out}");
        // A start tag's text in an attribute is not a block.
        let out = signature(r#"<p title="<style>">Ada</p><style>p { color: red }</style>"#);
        assert!(out.contains("<style>p { color: red }</style>"), "{out}");
        assert!(out.contains("Ada"), "{out}");
    }

    #[test]
    fn signature_comments_go_with_outlook_font_fallbacks() {
        let out = signature(
            "<!--[if mso]><style>p { font-family: Arial }</style><![endif]--><p>Ada<!-- note --></p>",
        );
        assert_eq!(out, "<p>Ada</p>");
    }

    #[test]
    fn oversized_signatures_are_refused() {
        let big = format!("<p>{}</p>", "a".repeat(MAX_SIGNATURE_BYTES));
        let error = sanitize_signature_html(&big, None).unwrap_err();
        assert_eq!(error.code, crate::error::ErrorCode::InvalidInput);
        assert!(sanitize_signature_html(&"a".repeat(MAX_SIGNATURE_BYTES), None).is_ok());
    }
}
