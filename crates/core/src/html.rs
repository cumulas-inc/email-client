//! Handling of untrusted HTML from email bodies.
//!
//! Defence in depth: bodies are sanitised here (no scripts, event handlers,
//! forms, frames or `javascript:` URLs), then the UI renders them inside a
//! sandboxed `<iframe>` without `allow-scripts`, with a Content Security
//! Policy that blocks remote loads unless the user allowed them.

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

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

/// Sanitises an HTML email body for display.
pub fn sanitize_email_html(html: &str, allow_remote: bool) -> SanitizedHtml {
    let blocked = Arc::new(AtomicBool::new(false));
    let blocked_in_filter = Arc::clone(&blocked);

    let generic_attributes: HashSet<&str> = [
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
    ]
    .into_iter()
    .collect();

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
}
