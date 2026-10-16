//! Decoding of IMAP "modified UTF-7" mailbox names (RFC 3501 section 5.1.3).

use base64::Engine as _;
use base64::alphabet::IMAP_MUTF7;
use base64::engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig};

/// Base64 as used inside modified UTF-7: `,` instead of `/` and no padding.
const MUTF7: GeneralPurpose = GeneralPurpose::new(
    &IMAP_MUTF7,
    GeneralPurposeConfig::new()
        .with_encode_padding(false)
        .with_decode_padding_mode(DecodePaddingMode::RequireNone)
        .with_decode_allow_trailing_bits(true),
);

/// Decodes a mailbox name for display. Names that are not valid modified
/// UTF-7 (e.g. raw UTF-8 from servers that allow it) are returned unchanged.
pub(super) fn decode(name: &str) -> String {
    if !name.contains('&') {
        return name.to_owned();
    }
    decode_strict(name).unwrap_or_else(|| name.to_owned())
}

fn decode_strict(name: &str) -> Option<String> {
    let mut out = String::with_capacity(name.len());
    let mut rest = name;
    while let Some(start) = rest.find('&') {
        out.push_str(&rest[..start]);
        let shifted = &rest[start + 1..];
        let end = shifted.find('-')?;
        let encoded = &shifted[..end];
        if encoded.is_empty() {
            // "&-" is a literal ampersand.
            out.push('&');
        } else {
            let bytes = MUTF7.decode(encoded).ok()?;
            let (pairs, remainder) = bytes.as_chunks::<2>();
            if !remainder.is_empty() {
                return None;
            }
            let units = pairs.iter().map(|pair| u16::from_be_bytes(*pair));
            for c in char::decode_utf16(units) {
                out.push(c.ok()?);
            }
        }
        rest = &shifted[end + 1..];
    }
    out.push_str(rest);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::decode;

    #[test]
    fn decodes_modified_utf7() {
        assert_eq!(decode("Entw&APw-rfe"), "Entwürfe");
        assert_eq!(decode("Gel&APY-schte Elemente"), "Gelöschte Elemente");
        assert_eq!(
            decode("~peter/mail/&U,BTFw-/&ZeVnLIqe-"),
            "~peter/mail/台北/日本語"
        );
        assert_eq!(decode("Tom &- Jerry"), "Tom & Jerry");
        assert_eq!(decode("INBOX"), "INBOX");
        // Surrogate pairs (emoji).
        assert_eq!(decode("&2D3egA-"), "🚀");
    }

    #[test]
    fn leaves_invalid_names_unchanged() {
        assert_eq!(decode("A&B"), "A&B");
        assert_eq!(decode("&Jjo!-"), "&Jjo!-");
        assert_eq!(decode("Entwürfe"), "Entwürfe");
    }
}
