//! Detection, labels and diagnostics, against the forms Microsoft 365,
//! S/MIME clients and senders who plant signals produce.

use super::*;
use crate::model::{CatalogLabel, LabelCatalog};

const TENANT: &str = "72f988bf-86f1-41af-91ab-2d7cd011db47";
const OTHER_TENANT: &str = "11111111-2222-3333-4444-555555555555";
const LABEL: &str = "6c1b3b5e-0d4f-4b1e-9d3a-2f0e6a7b8c9d";
const OTHER_LABEL: &str = "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee";
const PORTAL: &str = "https://outlook.office365.com/Encryption/retrieve.ashx?recipientemailaddress=alex.morgan%40contoso.com&messageId=abc";

fn file(id: &str, name: &str, content_type: &str) -> AttachmentMeta {
    AttachmentMeta {
        id: id.into(),
        name: name.into(),
        content_type: content_type.into(),
        size: 48_213,
        is_inline: false,
        content_id: None,
    }
}

fn signals() -> MessageSignals {
    MessageSignals::default()
}

fn class(class: &str) -> MessageSignals {
    MessageSignals {
        message_class: Some(class.into()),
        ..signals()
    }
}

fn mime(mime_type: &str) -> MessageSignals {
    MessageSignals {
        mime_type: Some(mime_type.into()),
        ..signals()
    }
}

fn kind(protection: Option<Protection>) -> Option<ProtectionKind> {
    protection.map(|p| p.kind)
}

// --- Microsoft Purview encryption ---------------------------------------------

#[test]
fn rpmsg_files_count_by_exact_name_whatever_their_type() {
    for name in [
        "message.rpmsg",
        "MESSAGE_V2.RPMSG",
        "Message_v3.rpmsg",
        " message_v4.rpmsg ",
    ] {
        let files = [file("att-1", name, "application/octet-stream")];
        let found = detect(&signals(), &files, Some("<p>notice</p>"), false, false).unwrap();
        assert_eq!(found.kind, ProtectionKind::RightsManaged, "{name}");
        assert_eq!(found.container_ids, ["att-1"], "{name}");
        assert!(!found.content_hidden);
    }
}

#[test]
fn the_rpmsg_type_counts_only_with_a_message_level_signal() {
    let files = [file("att-1", "report.rpmsg", RPMSG_TYPE)];
    // A forwarded protected file in ordinary mail stays a file.
    assert_eq!(
        detect(&signals(), &files, Some("<p>FYI</p>"), false, false),
        None
    );

    let content_class = MessageSignals {
        content_class: Some("rpmsg.message".into()),
        ..signals()
    };
    let found = detect(&content_class, &files, None, true, false).unwrap();
    assert_eq!(found.kind, ProtectionKind::RightsManaged);
    assert_eq!(found.container_ids, ["att-1"]);
    for class_name in [
        "IPM.Note.rpmsg.Microsoft Information Protection",
        "ipm.note.RPMSG",
    ] {
        let found = detect(&class(class_name), &files, None, true, false).unwrap();
        assert_eq!(found.kind, ProtectionKind::RightsManaged, "{class_name}");
        assert_eq!(found.container_ids, ["att-1"]);
    }
}

#[test]
fn content_class_alone_marks_rights_managed_mail() {
    // Graph may list no attachment at all.
    let found = detect(
        &MessageSignals {
            content_class: Some(" RPMSG.Message ".into()),
            ..signals()
        },
        &[],
        None,
        true,
        false,
    )
    .unwrap();
    assert_eq!(found.kind, ProtectionKind::RightsManaged);
    assert!(found.container_ids.is_empty());
    assert_eq!(found.portal_url, None);
}

#[test]
fn rights_managed_mail_with_a_portal_link_offers_the_portal() {
    // The rpmsg-plus-link form non-Microsoft recipients get.
    let html = format!(
        r#"<p><a href="{}">Read the message</a></p>"#,
        PORTAL.replace('&', "&amp;")
    );
    let files = [file("att-1", "message.rpmsg", RPMSG_TYPE)];
    let found = detect(&signals(), &files, Some(&html), false, false).unwrap();
    assert_eq!(found.kind, ProtectionKind::RightsManaged);
    assert_eq!(found.portal_url.as_deref(), Some(PORTAL));
}

// --- S/MIME --------------------------------------------------------------------

#[test]
fn smime_message_classes_and_mime_types() {
    let p7m = [file("att-1", "smime.p7m", "application/pkcs7-mime")];
    // On Graph, opaque-signed and encrypted share a class: unknown content
    // is treated as encrypted.
    assert_eq!(
        kind(detect(&class("IPM.Note.SMIME"), &p7m, None, true, false)),
        Some(ProtectionKind::SmimeEncrypted)
    );
    assert_eq!(
        kind(detect(
            &class("IPM.Note.SMIME.MultipartSigned"),
            &p7m,
            Some("<p>Hi</p>"),
            false,
            false
        )),
        Some(ProtectionKind::SmimeSigned)
    );

    // IMAP: the top-level MIME type.
    let enveloped = mime("application/pkcs7-mime; smime-type=enveloped-data");
    assert_eq!(
        kind(detect(&enveloped, &p7m, None, true, false)),
        Some(ProtectionKind::SmimeEncrypted)
    );
    let signed_data = mime("application/x-pkcs7-mime; smime-type=signed-data");
    let found = detect(&signed_data, &p7m, None, true, false).unwrap();
    assert_eq!(found.kind, ProtectionKind::SmimeSigned);
    let p7s = [file("att-2", "smime.p7s", "application/pkcs7-signature")];
    let clear_signed = mime("multipart/signed; protocol=application/pkcs7-signature");
    let found = detect(&clear_signed, &p7s, Some("<p>Hi</p>"), false, false).unwrap();
    assert_eq!(found.kind, ProtectionKind::SmimeSigned);
    assert_eq!(found.container_ids, ["att-2"]);
    assert!(!found.content_hidden);

    // OpenPGP isn't S/MIME, and its signature file is an ordinary one.
    let pgp = mime("multipart/signed; protocol=application/pgp-signature");
    let asc = [file("att-2", "signature.asc", "application/pgp-signature")];
    assert_eq!(detect(&pgp, &asc, Some("<p>Hi</p>"), false, false), None);
}

#[test]
fn smime_types_count_as_containers_only_with_a_signal() {
    // An Italian e-invoice: a signed file attached to ordinary mail.
    let invoice = [
        file("att-1", "fattura.xml.p7m", "application/pkcs7-mime"),
        file("att-2", "certs.p7c", "application/pkcs7-mime"),
        file("att-3", "detached.p7s", "application/pkcs7-signature"),
    ];
    let mixed = mime("multipart/mixed");
    assert_eq!(
        detect(
            &mixed,
            &invoice,
            Some("<p>In allegato la fattura.</p>"),
            false,
            false
        ),
        None
    );
    // With an S/MIME class, the type agrees with it.
    let found = detect(&class("IPM.Note.SMIME"), &invoice[..1], None, true, false).unwrap();
    assert_eq!(found.container_ids, ["att-1"]);
}

#[test]
fn signed_documents_attached_to_signed_mail_stay_ordinary() {
    // multipart/signed around the sender's files: a CAdES-signed contract
    // and a detached signature of their own are theirs to open, only the
    // message's signature part is a container.
    let signed = mime("multipart/signed; protocol=application/pkcs7-signature");
    for prefix in ["", MIME_ATTACHMENT_PREFIX] {
        let files = [
            file(
                &format!("{prefix}1.2"),
                "contract.pdf.p7m",
                "application/pkcs7-mime",
            ),
            file(
                &format!("{prefix}1.3"),
                "document.pdf.p7s",
                "application/pkcs7-signature",
            ),
            file(
                &format!("{prefix}2"),
                "signature",
                "application/x-pkcs7-signature",
            ),
        ];
        let found = detect(&signed, &files, Some("<p>Signed</p>"), false, false).unwrap();
        assert_eq!(found.kind, ProtectionKind::SmimeSigned);
        assert_eq!(found.container_ids, [format!("{prefix}2")]);
        assert!(!found.content_hidden);
    }
    // An opaque root is its own content; a .p7m inside it isn't another.
    let opaque = mime("application/pkcs7-mime; smime-type=enveloped-data");
    let files = [
        file("1", "encrypted", "application/pkcs7-mime"),
        file("1.2", "fattura.xml.p7m", "application/pkcs7-mime"),
    ];
    let found = detect(&opaque, &files, None, true, false).unwrap();
    assert_eq!(found.kind, ProtectionKind::SmimeEncrypted);
    assert_eq!(found.container_ids, ["1"]);
    // The exact names count wherever they are.
    let named = [file("1.2", "smime.p7s", "application/pkcs7-signature")];
    let found = detect(&signed, &named, Some("<p>Signed</p>"), false, false).unwrap();
    assert_eq!(found.container_ids, ["1.2"]);
}

#[test]
fn an_smime_p7m_without_other_signals_depends_on_the_body() {
    let p7m = [file("att-1", "smime.p7m", "multipart/signed")];
    // Next to a body: a clear-signed message's container on Graph.
    let found = detect(&signals(), &p7m, Some("<p>Hi</p>"), false, false).unwrap();
    assert_eq!(found.kind, ProtectionKind::SmimeSigned);
    assert!(found.content_hidden);
    // Alone: the encrypted content, unless its bytes said signed.
    assert_eq!(
        kind(detect(&signals(), &p7m, None, true, false)),
        Some(ProtectionKind::SmimeEncrypted)
    );
    let signed = MessageSignals {
        smime_content: Some(SmimeContent::Signed),
        ..signals()
    };
    let found = detect(&signed, &p7m, None, true, false).unwrap();
    assert_eq!(found.kind, ProtectionKind::SmimeSigned);
    assert!(found.content_hidden);
}

#[test]
fn the_p7m_content_decides_between_signed_and_encrypted() {
    let p7m = [file("att-1", "smime.p7m", "application/pkcs7-mime")];
    for (content, expected) in [
        (SmimeContent::Signed, ProtectionKind::SmimeSigned),
        (SmimeContent::Enveloped, ProtectionKind::SmimeEncrypted),
    ] {
        // Whatever the class hints.
        let signals = MessageSignals {
            smime_content: Some(content),
            ..class("IPM.Note.SMIME.MultipartSigned")
        };
        let found = detect(&signals, &p7m, None, true, false).unwrap();
        assert_eq!(found.kind, expected);
        // Opaque-signed: the content is inside.
        assert_eq!(found.content_hidden, content == SmimeContent::Signed);
    }
}

#[test]
fn signed_content_shows_once_unpacked_from_the_mime() {
    let p7m = [file("att-1", "smime.p7m", "multipart/signed")];
    let clear_signed = class("IPM.Note.SMIME.MultipartSigned");
    let found = detect(&clear_signed, &p7m, Some("<p>Hi</p>"), false, false).unwrap();
    assert!(found.content_hidden);
    // Unpacked: its attachments are the MIME's, and the container is gone.
    let unpacked = MessageSignals {
        body_from_mime: true,
        mime_type: Some("multipart/signed; protocol=application/pkcs7-signature".into()),
        ..clear_signed
    };
    let files = [
        file("mime:2", "smime.p7s", "application/pkcs7-signature"),
        file("mime:1.2", "certificate.pdf", "application/pdf"),
    ];
    let found = detect(&unpacked, &files, Some("<p>Hi</p>"), false, false).unwrap();
    assert_eq!(found.kind, ProtectionKind::SmimeSigned);
    assert!(!found.content_hidden);
    assert_eq!(found.container_ids, ["mime:2"]);
}

#[test]
fn smime_signals_ask_for_the_attachments() {
    assert!(wants_attachments(&class("IPM.Note.SMIME")));
    assert!(wants_attachments(&MessageSignals {
        content_class: Some("rpmsg.message".into()),
        ..signals()
    }));
    assert!(!wants_attachments(&class("IPM.Note")));
    assert!(!wants_attachments(&signals()));
}

// --- The encrypted message portal ----------------------------------------------

#[test]
fn portal_links_are_found_as_written() {
    let direct = format!(
        r#"<a href="{}">Read the message</a>"#,
        PORTAL.replace('&', "&amp;")
    );
    assert_eq!(portal_link_in(&direct).as_deref(), Some(PORTAL));
    let other_host = r#"<A HREF='https://outlook.office.com/encryption/retrieve.ashx?x=1'>x</A>"#;
    assert_eq!(
        portal_link_in(other_host).as_deref(),
        Some("https://outlook.office.com/encryption/retrieve.ashx?x=1")
    );

    // Safe Links: judged by what it wraps, kept as written.
    let wrapped = format!(
        "https://nam02.safelinks.protection.outlook.com/?url={}&data=05%7C02&reserved=0",
        url::form_urlencoded::byte_serialize(PORTAL.as_bytes()).collect::<String>()
    );
    let html = format!(
        r#"<a href="{}">Read the message</a>"#,
        wrapped.replace('&', "&amp;")
    );
    assert_eq!(portal_link_in(&html).as_deref(), Some(wrapped.as_str()));
    let wrapping_another = "<a href=\"https://nam02.safelinks.protection.outlook.com/?url=https%3A%2F%2Fevil.example%2FEncryption%2F\">x</a>";
    assert_eq!(portal_link_in(wrapping_another), None);
}

#[test]
fn a_safe_links_wrapper_must_itself_be_plain_https() {
    // The wrapper is what the button opens: plain HTTP, another port or a
    // user name would let it go elsewhere, whatever it wraps.
    let inner = url::form_urlencoded::byte_serialize(PORTAL.as_bytes()).collect::<String>();
    for wrapper in [
        "http://nam01.safelinks.protection.outlook.com/",
        "https://nam01.safelinks.protection.outlook.com:8080/",
        "https://user@nam01.safelinks.protection.outlook.com/",
        "https://user:pass@nam01.safelinks.protection.outlook.com/",
    ] {
        let html = format!(r#"<a href="{wrapper}?url={inner}&amp;data=x">Read the message</a>"#);
        assert_eq!(portal_link_in(&html), None, "{wrapper}");
    }
    // Nor the portal itself with a user name.
    let html = r#"<a href="https://user@outlook.office365.com/Encryption/retrieve.ashx">x</a>"#;
    assert_eq!(portal_link_in(html), None);
}

#[test]
fn only_microsofts_portal_over_https_counts() {
    for href in [
        "https://outlook.office365.com.evil.example/Encryption/retrieve.ashx",
        "https://evil.example/outlook.office365.com/Encryption/",
        "http://outlook.office365.com/Encryption/retrieve.ashx",
        "https://outlook.office365.com/mail/inbox",
        "https://outlook.office365.com:8443/Encryption/retrieve.ashx",
        "javascript:alert(1)",
    ] {
        let html = format!(r#"<a href="{href}">Read the message</a>"#);
        assert_eq!(portal_link_in(&html), None, "{href}");
    }
    // Not an href attribute.
    let data = format!(r#"<a data-href="{PORTAL}">x</a>"#);
    assert_eq!(portal_link_in(&data), None);
}

#[test]
fn a_fake_button_before_the_real_link_is_passed_over() {
    let html = format!(
        r#"<table><tr><td><a href="https://evil.example/login" style="font-size:20px">Read the message</a></td></tr></table>
           <p>Or <a href="{}">select here</a>.</p>"#,
        PORTAL.replace('&', "&amp;")
    );
    let found = detect(&signals(), &[], Some(&html), false, false).unwrap();
    assert_eq!(found.kind, ProtectionKind::PortalLink);
    assert_eq!(found.portal_url.as_deref(), Some(PORTAL));
    assert!(found.container_ids.is_empty());
}

#[test]
fn own_mail_never_has_a_portal_link() {
    let html = format!(r#"<blockquote><a href="{PORTAL}">Read the message</a></blockquote>"#);
    assert_eq!(detect(&signals(), &[], Some(&html), false, true), None);
    let files = [file("att-1", "message.rpmsg", RPMSG_TYPE)];
    let found = detect(&signals(), &files, Some(&html), false, true).unwrap();
    assert_eq!(found.kind, ProtectionKind::RightsManaged);
    assert_eq!(found.portal_url, None);
}

#[test]
fn ordinary_mail_is_none() {
    let files = [file("att-1", "report.pdf", "application/pdf")];
    assert_eq!(
        detect(&class("IPM.Note"), &files, Some("<p>Hi</p>"), false, false),
        None
    );
    assert_eq!(detect(&signals(), &[], None, true, false), None);
}

// --- What a .p7m holds ------------------------------------------------------------

/// A ContentInfo's start: SEQUENCE with `length`, then the OID.
fn content_info(length: &[u8], oid: &[u8]) -> Vec<u8> {
    let mut bytes = vec![0x30];
    bytes.extend_from_slice(length);
    bytes.push(0x06);
    bytes.push(oid.len() as u8);
    bytes.extend_from_slice(oid);
    // [0] EXPLICIT content follows.
    bytes.extend_from_slice(&[0xA0, 0x80, 0x30, 0x80]);
    bytes
}

#[test]
fn the_content_info_oid_says_what_a_p7m_holds() {
    assert_eq!(
        smime_content_of(&content_info(&[0x82, 0x05, 0x10], OID_SIGNED_DATA)),
        Some(SmimeContent::Signed)
    );
    assert_eq!(
        smime_content_of(&content_info(&[0x80], OID_ENVELOPED_DATA)),
        Some(SmimeContent::Enveloped)
    );
    assert_eq!(
        smime_content_of(&content_info(&[0x84, 0, 1, 0, 0], OID_AUTH_ENVELOPED_DATA)),
        Some(SmimeContent::Enveloped)
    );
    assert_eq!(
        smime_content_of(&content_info(&[0x7F], OID_SIGNED_DATA)),
        Some(SmimeContent::Signed)
    );
    // data (1.2.840.113549.1.7.1), and not a ContentInfo at all.
    let data = [0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x01, 0x07, 0x01];
    assert_eq!(smime_content_of(&content_info(&[0x80], &data)), None);
    assert_eq!(smime_content_of(b"MIME-Version: 1.0\r\n"), None);
    // Five length bytes are more than a .p7m can need.
    assert_eq!(
        smime_content_of(&content_info(&[0x85, 0, 0, 0, 1, 0], OID_SIGNED_DATA)),
        None
    );
}

#[test]
fn reading_a_p7m_prefix_never_panics() {
    let whole = content_info(&[0x83, 0x01, 0x00, 0x00], OID_ENVELOPED_DATA);
    for end in 0..whole.len() {
        let expected = (end >= 16).then_some(SmimeContent::Enveloped);
        assert_eq!(smime_content_of(&whole[..end]), expected, "{end} bytes");
    }
    // Arbitrary bytes, many lengths.
    let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
    for round in 0..5000 {
        let len = round % 40;
        let bytes: Vec<u8> = (0..len)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                // Mostly the bytes the reader branches on.
                match state % 4 {
                    0 => 0x30,
                    1 => 0x06,
                    2 => 0x80 | ((state >> 8) as u8 & 0x07),
                    _ => (state >> 16) as u8,
                }
            })
            .collect();
        let _ = smime_content_of(&bytes);
    }
}

// --- Labels ---------------------------------------------------------------------

fn msip(entries: &[(&str, &str, &str)]) -> String {
    entries
        .iter()
        .map(|(id, field, value)| format!("MSIP_Label_{id}_{field}={value}"))
        .collect::<Vec<_>>()
        .join("; ")
}

#[test]
fn msip_labels_of_two_organizations() {
    let raw = msip(&[
        (OTHER_LABEL, "Enabled", "true"),
        (OTHER_LABEL, "SiteId", OTHER_TENANT),
        (OTHER_LABEL, "Name", "Their General"),
        (LABEL, "Enabled", "True"),
        (LABEL, "SetDate", "2026-10-16T09:00:00Z"),
        (LABEL, "Method", "Privileged"),
        (LABEL, "Name", "Confidential"),
        (LABEL, "SiteId", &TENANT.to_uppercase()),
        (LABEL, "ActionId", "8d1a1a2c-0000-0000-0000-000000000000"),
        (LABEL, "ContentBits", "0"),
    ]);
    let labels = parse_msip_labels(&raw);
    assert_eq!(labels.len(), 2);
    assert_eq!(labels[0].id, OTHER_LABEL);
    assert_eq!(
        labels[1],
        MsipLabel {
            id: LABEL.into(),
            enabled: true,
            site_id: Some(TENANT.to_uppercase()),
            name: Some("Confidential".into()),
            method: Some("Privileged".into()),
        }
    );
    // The account's own organization's, whatever the case.
    assert_eq!(own_label(&raw, TENANT).unwrap().id, LABEL);
    assert_eq!(
        own_label(&raw, &format!(" {} ", TENANT.to_uppercase()))
            .unwrap()
            .id,
        LABEL
    );
    assert_eq!(own_label(&raw, OTHER_TENANT).unwrap().id, OTHER_LABEL);
    assert_eq!(
        own_label(&raw, "33333333-2222-3333-4444-555555555555"),
        None
    );
}

#[test]
fn msip_labels_skip_what_doesnt_read() {
    let upper = LABEL.to_uppercase();
    let raw = format!(
        "garbage; =x; msip_label_{upper}_enabled=TRUE;msip_label_{upper}_siteid={TENANT}; \
         MSIP_Label_not-a-guid_Enabled=true; MSIP_Label_{LABEL}Enabled=true; \
         MSIP_Label_{OTHER_LABEL}_Enabled=false; MSIP_Label_{OTHER_LABEL}_SiteId={TENANT}; \
         MSIP_Label_ü{LABEL}_Enabled=true; ;;"
    );
    let labels = parse_msip_labels(&raw);
    assert_eq!(
        labels.iter().map(|l| l.id.as_str()).collect::<Vec<_>>(),
        [LABEL, OTHER_LABEL]
    );
    assert!(labels[0].enabled);
    // A label that isn't enabled is never the message's.
    assert!(!labels[1].enabled);
    assert_eq!(own_label(&raw, TENANT).unwrap().id, LABEL);
    assert!(parse_msip_labels("").is_empty());
    assert!(parse_msip_labels("MSIP_Label_").is_empty());
}

#[test]
fn only_known_requests_are_read() {
    let ask = |action: Option<&str>, label: Option<&str>| {
        requested(&MessageSignals {
            requested_action: action.map(str::to_owned),
            requested_label: label.map(str::to_owned),
            ..signals()
        })
    };
    assert_eq!(
        ask(
            Some("encrypt"),
            Some(&format!(" {} ", LABEL.to_uppercase()))
        ),
        (Some(ProtectAction::Encrypt), Some(LABEL.to_owned()))
    );
    assert_eq!(
        ask(Some("Do-Not-Forward"), None),
        (Some(ProtectAction::DoNotForward), None)
    );
    assert_eq!(
        ask(Some("encrypt-everything"), Some("Confidential")),
        (None, None)
    );
    assert_eq!(
        ask(None, Some("{6c1b3b5e-0d4f-4b1e-9d3a-2f0e6a7b8c9d}")),
        (None, None)
    );
}

#[test]
fn blank_bodies() {
    assert!(is_blank(None, None));
    assert!(is_blank(Some("<div><br>&nbsp;</div>"), Some(" \r\n")));
    assert!(!is_blank(Some("<p>Hi</p>"), None));
    assert!(!is_blank(None, Some("Hi")));
}

// --- Errors ---------------------------------------------------------------------

#[test]
fn errors_name_the_kind_of_message() {
    let error = headers_not_kept(Some(ReplyKind::ReplyAll));
    assert_eq!(error.code, ErrorCode::Provider);
    assert_eq!(
        error.message,
        "Cumulas Mail couldn't add the label or encryption to this reply, so it wasn't sent. \
         Send it from Outlook on the web instead."
    );
    assert!(has_reason(&error, "headersNotKept"));
    assert!(headers_not_kept(None).message.contains("this message,"));
    assert!(
        headers_not_kept(Some(ReplyKind::Forward))
            .message
            .contains("this forward,")
    );
    let copied = label_copied(LABEL);
    assert!(has_reason(&copied, "labelCopied"));
    assert_eq!(copied.details.unwrap()["labelId"], LABEL);
    assert!(!has_reason(&Error::provider("x"), "headersNotKept"));
}

// --- Diagnostics ------------------------------------------------------------------

fn catalog() -> LabelCatalog {
    LabelCatalog {
        labels: vec![CatalogLabel {
            id: LABEL.into(),
            name: "Confidential".into(),
            parent_name: None,
            tooltip: None,
            color: None,
            has_protection: true,
            sensitivity: Some(2),
        }],
        ..LabelCatalog::default()
    }
}

#[test]
fn diagnostics_describe_the_signals_in_a_fixed_shape() {
    let content = MessageContent {
        body_html: Some("<p>Your organization sent you a protected message.</p>".into()),
        attachments: vec![
            file("att-1", "message.rpmsg", RPMSG_TYPE),
            AttachmentMeta {
                size: 96_256,
                ..file(
                    "att-2",
                    "Q4 forecast.pdf",
                    "application/pdf; name=\"Q4 forecast.pdf\"",
                )
            },
        ],
        signals: MessageSignals {
            content_class: Some("rpmsg.message".into()),
            message_class: Some("IPM.Note.rpmsg.Microsoft Information Protection".into()),
            has_attachments_flag: Some(true),
            msip_labels: Some(msip(&[
                (LABEL, "Enabled", "true"),
                (LABEL, "SiteId", TENANT),
                (LABEL, "Method", "Standard"),
            ])),
            msip_labels_source: Some("header".into()),
            ..signals()
        },
        ..MessageContent::default()
    };
    let protection = detect(
        &content.signals,
        &content.attachments,
        content.body_html.as_deref(),
        false,
        false,
    );
    let text = diagnostics(&DiagnosticsInput {
        account: DiagnosticsAccount::MicrosoftWork,
        folder_role: FolderRole::Inbox,
        content: &content,
        protection: protection.as_ref(),
        tenant_id: Some(TENANT),
        catalog: Some(&catalog()),
    });
    let expected = format!(
        "Cumulas Mail {}: protection details of one message
These details don't include the message's subject, addresses, text, links, or the names of its files other than the protection's own.

Account: Microsoft 365 (work or school)
Folder: inbox
Detected: Microsoft Purview encryption
Content hidden in a container: no
Portal link: no
Content-Class: rpmsg.message
Message class: IPM.Note.rpmsg.Microsoft Information Protection
S/MIME content: not applicable
MIME type: none
Server says it has attachments: yes
Body: HTML, 54 characters
Body read from the message's MIME: no
Properties read: yes
Attachments: 2
  - message.rpmsg · application/x-microsoft-rpmsg-message · 47 KB · protection file
  - (name left out) · application/pdf · 94 KB
Label header: from the header
  - {LABEL} · this organization: yes · enabled: yes · method: Standard · in your labels: yes
Requested by Cumulas Mail: none · label: none
",
        env!("CARGO_PKG_VERSION")
    );
    assert_eq!(text, expected);
}

#[test]
fn diagnostics_say_what_couldnt_be_read() {
    let content = MessageContent {
        attachments: vec![AttachmentMeta {
            size: 3 * 1024 * 1024 + 300 * 1024,
            ..file("att-1", "smime.p7m", "application/pkcs7-mime")
        }],
        signals: MessageSignals {
            message_class: Some("IPM.Note.SMIME".into()),
            properties_unavailable: true,
            properties_attempts: 2,
            ..signals()
        },
        ..MessageContent::default()
    };
    let protection = detect(&content.signals, &content.attachments, None, true, false);
    let text = diagnostics(&DiagnosticsInput {
        account: DiagnosticsAccount::MicrosoftPersonal,
        folder_role: FolderRole::Other,
        content: &content,
        protection: protection.as_ref(),
        tenant_id: None,
        catalog: None,
    });
    for line in [
        "Account: Microsoft 365 (personal)",
        "Folder: other",
        "Detected: S/MIME encryption",
        "Content-Class: not read",
        "S/MIME content: not read",
        "Server says it has attachments: not said",
        "Body: empty",
        "Properties read: no (2 tries)",
        "  - smime.p7m · application/pkcs7-mime · 3.3 MB · protection file",
        "Label header: not read",
        "Requested by Cumulas Mail: not read · label: not read",
    ] {
        assert!(text.lines().any(|l| l == line), "{line}\n{text}");
    }
}

#[test]
fn diagnostics_never_show_the_message() {
    let planted = [
        "Quarterly layoffs",
        "dana@contoso.com",
        "secret plans for Atlas",
        "Atlas pricing.xlsx",
        "recipientemailaddress",
        "outlook.office365.com/Encryption",
    ];
    let html = format!(
        r#"<p>secret plans for Atlas, from dana@contoso.com: Quarterly layoffs</p>
           <a href="{}">Read the message</a>"#,
        PORTAL.replace('&', "&amp;")
    );
    let content = MessageContent {
        body_html: Some(html.clone()),
        body_text: Some("secret plans for Atlas".into()),
        attachments: vec![
            file("att-1", "Atlas pricing.xlsx", "application/vnd.ms-excel"),
            // A container by type and signal, named by the sender.
            file("att-2", "Quarterly layoffs.rpmsg", RPMSG_TYPE),
        ],
        signals: MessageSignals {
            content_class: Some("rpmsg.message".into()),
            msip_labels: Some(msip(&[
                (LABEL, "Enabled", "true"),
                (LABEL, "Name", "dana@contoso.com"),
                (LABEL, "SiteId", TENANT),
            ])),
            msip_labels_source: Some("property".into()),
            requested_label: Some("secret plans for Atlas".into()),
            ..signals()
        },
        ..MessageContent::default()
    };
    let protection = detect(
        &content.signals,
        &content.attachments,
        Some(&html),
        false,
        false,
    );
    assert!(protection.as_ref().unwrap().portal_url.is_some());
    let text = diagnostics(&DiagnosticsInput {
        account: DiagnosticsAccount::Imap,
        folder_role: FolderRole::Inbox,
        content: &content,
        protection: protection.as_ref(),
        tenant_id: None,
        catalog: None,
    });
    for secret in planted {
        assert!(!text.contains(secret), "{secret} in\n{text}");
    }
    assert!(text.contains("Portal link: yes (outlook.office365.com)"));
    assert!(text.contains("Label header: from the message property"));
    assert!(text.contains("this organization: unknown"));
    assert!(text.contains("in your labels: no list"));
    assert!(text.contains("label: unrecognized"));
    assert!(text.contains(
        "  - (name left out) · application/x-microsoft-rpmsg-message · 47 KB · protection file"
    ));
}
