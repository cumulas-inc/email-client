//! Reads server-sent events (the WHATWG `text/event-stream` format) as the
//! push service sends them, a chunk at a time: a byte order mark at the
//! start, lines ending in CRLF, LF or CR (also split across chunks), data
//! over several lines, comments (the service's `: ka` heartbeats), `event:`
//! and `retry:`. `id:` is ignored, as nothing is replayed.

use std::time::Duration;

const BOM: &[u8] = b"\xEF\xBB\xBF";
/// The longest line or event accepted; the service's are well under 1 KiB.
const MAX_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SseItem {
    /// An event; `name` is `message` when the stream gave none.
    Event { name: String, data: String },
    /// A comment line. It carries nothing but shows the stream is alive.
    Comment,
    /// How long to wait before reconnecting, should the stream drop.
    Retry(Duration),
}

/// A line or event over [`MAX_BYTES`].
#[derive(Debug)]
pub(crate) struct TooLong;

#[derive(Default)]
pub(crate) struct SseParser {
    line: Vec<u8>,
    /// The last line ended with a CR, so an LF right after it ends nothing.
    after_cr: bool,
    /// Bytes of a byte order mark seen at the start, until it's decided.
    bom: usize,
    started: bool,
    event: String,
    data: String,
}

impl SseParser {
    /// Reads `chunk`, adding what it completes to `out`.
    pub(crate) fn feed(&mut self, chunk: &[u8], out: &mut Vec<SseItem>) -> Result<(), TooLong> {
        for &byte in chunk {
            if !self.started {
                if byte == BOM[self.bom] {
                    self.bom += 1;
                    self.started = self.bom == BOM.len();
                    continue;
                }
                // Not a byte order mark after all: what matched is text.
                self.started = true;
                self.line.extend_from_slice(&BOM[..self.bom]);
            }
            match byte {
                b'\n' if self.after_cr => self.after_cr = false,
                b'\r' | b'\n' => {
                    self.after_cr = byte == b'\r';
                    self.end_line(out)?;
                }
                _ => {
                    self.after_cr = false;
                    if self.line.len() >= MAX_BYTES {
                        return Err(TooLong);
                    }
                    self.line.push(byte);
                }
            }
        }
        Ok(())
    }

    fn end_line(&mut self, out: &mut Vec<SseItem>) -> Result<(), TooLong> {
        let line = String::from_utf8_lossy(&self.line).into_owned();
        self.line.clear();
        if line.is_empty() {
            // A blank line ends an event; one without data is dropped.
            let name = std::mem::take(&mut self.event);
            if !self.data.is_empty() {
                let mut data = std::mem::take(&mut self.data);
                data.pop();
                let name = if name.is_empty() {
                    "message".into()
                } else {
                    name
                };
                out.push(SseItem::Event { name, data });
            }
            return Ok(());
        }
        if line.starts_with(':') {
            out.push(SseItem::Comment);
            return Ok(());
        }
        let (field, value) = match line.split_once(':') {
            Some((field, value)) => (field, value.strip_prefix(' ').unwrap_or(value)),
            None => (line.as_str(), ""),
        };
        match field {
            "event" => self.event = value.to_owned(),
            "data" => {
                if self.data.len() + value.len() >= MAX_BYTES {
                    return Err(TooLong);
                }
                self.data.push_str(value);
                self.data.push('\n');
            }
            "retry" if !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit()) => {
                if let Ok(ms) = value.parse() {
                    out.push(SseItem::Retry(Duration::from_millis(ms)));
                }
            }
            _ => {}
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Everything the format allows that the parser must handle.
    const SAMPLE: &[u8] = b"\xEF\xBB\xBFretry: 10000\r\n\r\n\
        event: ready\r\ndata: {\"v\":1,\"channel\":\"k1abc\"}\r\n\r\n\
        : ka\n\n\
        event: change\ndata: {\"s\":\"5ab1\",\"c\":\"created\"}\n\n\
        :\r\
        data: first\rdata:second\r\rdata\n\n\
        event: lifecycle\ndata: {\"s\":\"5ab1\",\"e\":\"missed\"}\nid: 7\n\n\
        event: nothing\n\n\
        event: renew\ndata: {\"endsAt\":\"2026-10-15T12:00:00.000Z\"}\nunknown: x\n\n\
        retry: soon\n\
        data: caf\xC3\xA9\n\n\
        event: unfinished\ndata: never sent";

    fn expected() -> Vec<SseItem> {
        let event = |name: &str, data: &str| SseItem::Event {
            name: name.into(),
            data: data.into(),
        };
        vec![
            SseItem::Retry(Duration::from_secs(10)),
            event("ready", r#"{"v":1,"channel":"k1abc"}"#),
            SseItem::Comment,
            event("change", r#"{"s":"5ab1","c":"created"}"#),
            SseItem::Comment,
            event("message", "first\nsecond"),
            event("message", ""),
            event("lifecycle", r#"{"s":"5ab1","e":"missed"}"#),
            event("renew", r#"{"endsAt":"2026-10-15T12:00:00.000Z"}"#),
            event("message", "café"),
        ]
    }

    fn parse(chunks: &[&[u8]]) -> Vec<SseItem> {
        let mut parser = SseParser::default();
        let mut out = Vec::new();
        for chunk in chunks {
            parser.feed(chunk, &mut out).unwrap();
        }
        out
    }

    #[test]
    fn the_sample_parses_whole() {
        assert_eq!(parse(&[SAMPLE]), expected());
    }

    #[test]
    fn the_sample_parses_split_at_every_byte() {
        for at in 0..=SAMPLE.len() {
            let (a, b) = SAMPLE.split_at(at);
            assert_eq!(parse(&[a, b]), expected(), "split at {at}");
        }
        let bytes: Vec<&[u8]> = SAMPLE.chunks(1).collect();
        assert_eq!(parse(&bytes), expected());
    }

    #[test]
    fn the_sample_parses_split_in_three() {
        for i in 0..=SAMPLE.len() {
            for j in i..=SAMPLE.len() {
                let chunks = [&SAMPLE[..i], &SAMPLE[i..j], &SAMPLE[j..]];
                assert_eq!(parse(&chunks), expected(), "split at {i} and {j}");
            }
        }
    }

    #[test]
    fn a_byte_order_mark_counts_only_at_the_start() {
        assert_eq!(
            parse(&[b"data: x\n\n\xEF\xBB\xBFdata: y\n\n"]),
            [
                SseItem::Event {
                    name: "message".into(),
                    data: "x".into()
                },
                // Later, it's part of the line: a field nobody knows.
            ]
        );
        // A partial mark followed by text is text, which here spoils the
        // field's name.
        assert!(parse(&[b"\xEFdata: z\n\n"]).is_empty());
        assert_eq!(
            parse(&[b"\xEF\xBB", b"\xBFdata: z\n\n"]),
            [SseItem::Event {
                name: "message".into(),
                data: "z".into()
            }]
        );
    }

    #[test]
    fn overlong_lines_are_refused() {
        let mut parser = SseParser::default();
        let mut out = Vec::new();
        let long = vec![b'a'; MAX_BYTES + 1];
        assert!(parser.feed(&long, &mut out).is_err());
        let mut parser = SseParser::default();
        let line = format!("data: {}\n", "b".repeat(MAX_BYTES / 2));
        parser.feed(line.as_bytes(), &mut out).unwrap();
        assert!(parser.feed(line.as_bytes(), &mut out).is_err());
    }
}
