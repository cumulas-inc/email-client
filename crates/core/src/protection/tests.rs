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
Relay (X-Signature-Stamped): no
Counted as internal (AuthAs): not said
Direction: not said
Request read from: none
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
        "Request read from: not read",
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

#[test]
fn values_are_cleaned_capped_and_empty_ones_dropped() {
    assert_eq!(
        clean_value(" Anonymous\r\n", 200).as_deref(),
        Some("Anonymous")
    );
    assert_eq!(
        clean_value("In\u{202E}ter\u{0007}nal", 200).as_deref(),
        Some("Internal")
    );
    // Nothing left: never a value, so an empty AuthAs doesn't count.
    for empty in ["", "   ", "\u{200F}\t"] {
        assert_eq!(clean_value(empty, 200), None, "{empty:?}");
    }
    assert_eq!(clean_value("abcdef", 3).as_deref(), Some("abc"));
    let id = format!("<{}@contoso.com>", "x".repeat(300));
    assert_eq!(clean_value(&id, 998).as_deref(), Some(id.as_str()));
}

#[test]
fn setup_changes_are_kept_per_tenant() {
    assert_eq!(
        org_setup_changed_key(" 85F67998-E79A-4173-95C2-90B0E6DC231D "),
        "orgsetup.changed.85f67998-e79a-4173-95c2-90b0e6dc231d"
    );
    assert_eq!(RULES_SETTLE, chrono::Duration::minutes(60));
    assert_eq!(POLICIES_SETTLE, chrono::Duration::hours(24));
    let settle: Settle =
        serde_json::from_str(r#"{ "rulesChangedAt": "2026-10-19T14:03:00Z" }"#).unwrap();
    assert_eq!(
        settle,
        Settle {
            rules_changed_at: Some("2026-10-19T14:03:00Z".parse().unwrap()),
            policies_changed_at: None,
        }
    );
    assert_eq!(
        serde_json::to_value(settle).unwrap(),
        serde_json::json!({ "rulesChangedAt": "2026-10-19T14:03:00Z", "policiesChangedAt": null })
    );
}

#[test]
fn evidence_reads_with_defaults() {
    let evidence: CheckEvidence =
        serde_json::from_str(r#"{ "relayStamped": true, "authAs": "Anonymous" }"#).unwrap();
    assert_eq!(
        evidence,
        CheckEvidence {
            relay_stamped: true,
            auth_as: Some("Anonymous".into()),
            ..CheckEvidence::default()
        }
    );
    assert_eq!(evidence.headers_from, HeadersFrom::Unread);
    let json = serde_json::to_value(CheckEvidence {
        own_labels: vec![SeenLabel {
            id: LABEL.into(),
            enabled: true,
        }],
        headers_from: HeadersFrom::Transport,
        detected: Some(ProtectionKind::RightsManaged),
        ..CheckEvidence::default()
    })
    .unwrap();
    assert_eq!(json["headersFrom"], "transport");
    assert_eq!(json["detected"], "rightsManaged");
    assert_eq!(
        json["ownLabels"],
        serde_json::json!([{ "id": LABEL, "enabled": true }])
    );
}

// --- Check evidence ---------------------------------------------------------------

const ARRIVED_BODY: &str = "<p>Automated setup check from Cumulas Mail. You can delete it.</p>";

fn arrived(signals: MessageSignals) -> MessageContent {
    MessageContent {
        body_html: Some(ARRIVED_BODY.into()),
        internet_message_id: Some("<a1@contoso.com>".into()),
        signals,
        ..MessageContent::default()
    }
}

fn at(hour: u32, minute: u32, second: u32) -> DateTime<Utc> {
    use chrono::TimeZone;
    Utc.with_ymd_and_hms(2026, 10, 19, hour, minute, second)
        .unwrap()
}

#[test]
fn header_values_are_unfolded_in_order() {
    let raw = "Received: from a\r\n\tby b\r\nX-MS-Exchange-Organization-AuthAs: Anonymous\r\n\
               x-ms-exchange-organization-authas:  Internal \r\nThread-Topic: a: b: c\r\n\
               Subject: folded\r\n over\r\n  two lines\r\n\r\nX-Body: not a header\r\n";
    assert_eq!(
        header_values(raw, "X-MS-Exchange-Organization-AuthAs"),
        ["Anonymous", "Internal"]
    );
    assert_eq!(header_values(raw, "RECEIVED"), ["from a\tby b"]);
    assert_eq!(header_values(raw, "Subject"), ["folded over  two lines"]);
    assert_eq!(header_values(raw, "Thread-Topic"), ["a: b: c"]);
    // The body isn't read.
    assert!(header_values(raw, "X-Body").is_empty());
    // Bare line feeds, and a field that ends the block.
    assert_eq!(header_values("A: 1\nB: 2\n\tmore", "b"), ["2\tmore"]);
    assert!(header_values("", "A").is_empty());
    // A line without a colon names no field, nor do its continuations.
    assert!(header_values("AuthAs Internal\r\n Internal\r\n", "AuthAs").is_empty());
    assert_eq!(header_values("Empty:\r\nEmpty: x", "Empty"), ["", "x"]);
}

#[test]
fn evidence_comes_from_the_transport_headers_first() {
    let transport = format!(
        "Received: from AM0PR01MB0001.eurprd01.prod.outlook.com\r\n\
         X-MS-Exchange-Organization-AuthAs: Anonymous\r\n\
         X-MS-Exchange-Organization-MessageDirectionality: Incoming\r\n\
         X-Signature-Stamped: 1\r\n\
         x-cumulas-label-stamped: {LABEL}\r\n\
         X-MS-Exchange-Organization-AuthAs: Internal\r\n\
         X-MS-Exchange-Organization-MessageDirectionality: Originating\r\n\
         Content-Class: rpmsg.message\r\n\
         x-cumulas-protect: encrypt\r\n\r\n"
    );
    // Graph's parsed headers say otherwise: the transport headers win.
    let content = MessageContent {
        attachments: vec![file("att-1", "message.rpmsg", RPMSG_TYPE)],
        ..arrived(MessageSignals {
            auth_as: Some("Internal".into()),
            message_class: Some("IPM.Note.rpmsg.Microsoft Information Protection".into()),
            ..signals()
        })
    };
    let evidence = check_evidence(
        &content,
        Some(&transport),
        TENANT,
        at(14, 0, 0),
        at(14, 0, 40),
    );
    assert_eq!(
        evidence,
        CheckEvidence {
            message_id: Some("<a1@contoso.com>".into()),
            request_arrived: true,
            relay_stamped: true,
            label_stamped: Some(LABEL.into()),
            auth_as: Some("Anonymous".into()),
            directionality: Some("Incoming".into()),
            detected: Some(ProtectionKind::RightsManaged),
            own_labels: vec![],
            foreign_labels: 0,
            content_class: Some("rpmsg.message".into()),
            message_class: Some("IPM.Note.rpmsg.Microsoft Information Protection".into()),
            rpmsg_attachment: true,
            headers_from: HeadersFrom::Transport,
            found_after_secs: Some(40),
        }
    );
    assert!(relay_not_internal(&evidence));
}

#[test]
fn without_transport_headers_evidence_comes_from_graphs() {
    let msip_labels = msip(&[
        (LABEL, "Enabled", "true"),
        (LABEL, "SiteId", TENANT),
        (OTHER_LABEL, "Enabled", "false"),
        (OTHER_LABEL, "SiteId", TENANT),
        ("bbbbbbbb-bbbb-4ccc-8ddd-eeeeeeeeeeee", "Enabled", "true"),
        (
            "bbbbbbbb-bbbb-4ccc-8ddd-eeeeeeeeeeee",
            "SiteId",
            OTHER_TENANT,
        ),
    ]);
    let content = arrived(MessageSignals {
        relay_stamped: true,
        label_stamped: Some(LABEL.into()),
        auth_as: Some("Internal".into()),
        directionality: Some("Originating".into()),
        msip_labels: Some(msip_labels),
        requested_label: Some(LABEL.into()),
        ..signals()
    });
    let evidence = check_evidence(&content, None, TENANT, at(14, 0, 0), at(14, 1, 10));
    assert_eq!(evidence.headers_from, HeadersFrom::Graph);
    assert!(evidence.relay_stamped && evidence.request_arrived);
    assert_eq!(evidence.label_stamped.as_deref(), Some(LABEL));
    assert_eq!(evidence.auth_as.as_deref(), Some("Internal"));
    assert_eq!(evidence.directionality.as_deref(), Some("Originating"));
    assert_eq!(evidence.detected, None);
    assert!(!evidence.rpmsg_attachment);
    assert_eq!(evidence.found_after_secs, Some(70));
    // The organization's labels by id, others counted only.
    assert_eq!(
        evidence.own_labels,
        [
            SeenLabel {
                id: LABEL.into(),
                enabled: true
            },
            SeenLabel {
                id: OTHER_LABEL.into(),
                enabled: false
            }
        ]
    );
    assert_eq!(evidence.foreign_labels, 1);
    assert_eq!(first_own_label(&evidence), Some(LABEL));
    // Returned as mail from inside: not "not internal".
    assert!(!relay_not_internal(&evidence));
}

#[test]
fn a_request_arrived_when_either_source_kept_it() {
    let transport = "X-MS-Exchange-Organization-AuthAs: Internal\r\n\r\n";
    // The property alone.
    let content = arrived(MessageSignals {
        requested_action: Some("encrypt".into()),
        requested_action_source: Some("property".into()),
        ..signals()
    });
    for transport in [Some(transport), None] {
        let evidence = check_evidence(&content, transport, TENANT, at(14, 0, 0), at(14, 1, 0));
        assert!(evidence.request_arrived, "{transport:?}");
    }
    // The transport headers alone.
    let headers = "x-cumulas-label: 6c1b3b5e-0d4f-4b1e-9d3a-2f0e6a7b8c9d\r\n\r\n";
    let content = arrived(signals());
    let evidence = check_evidence(&content, Some(headers), TENANT, at(14, 0, 0), at(14, 1, 0));
    assert!(evidence.request_arrived);
    // Neither, or only an empty header.
    for transport in [Some(transport), Some("x-cumulas-protect:  \r\n"), None] {
        let evidence = check_evidence(&content, transport, TENANT, at(14, 0, 0), at(14, 1, 0));
        assert!(!evidence.request_arrived, "{transport:?}");
    }
}

#[test]
fn a_content_class_only_the_transport_headers_have_still_counts() {
    // An rpmsg file by type alone needs a message-level signal.
    let content = MessageContent {
        attachments: vec![file("att-1", "attachment", RPMSG_TYPE)],
        ..arrived(signals())
    };
    let evidence = check_evidence(&content, None, TENANT, at(14, 0, 0), at(14, 1, 0));
    assert_eq!(evidence.detected, None);
    assert!(evidence.rpmsg_attachment);
    let evidence = check_evidence(
        &content,
        Some("Content-Class: rpmsg.message\r\n"),
        TENANT,
        at(14, 0, 0),
        at(14, 1, 0),
    );
    assert_eq!(evidence.detected, Some(ProtectionKind::RightsManaged));
    assert_eq!(evidence.content_class.as_deref(), Some("rpmsg.message"));
}

#[test]
fn evidence_values_are_cleaned_and_capped() {
    let long = "O".repeat(300);
    let message_id = format!("<{}@contoso.com>", "x".repeat(300));
    let transport = format!(
        "X-Signature-Stamped: 1\r\n\
         X-MS-Exchange-Organization-AuthAs: In\u{202E}ter\u{0007}nal\r\n\
         X-MS-Exchange-Organization-MessageDirectionality: {long}\r\n\r\n"
    );
    let content = MessageContent {
        internet_message_id: Some(message_id.clone()),
        ..arrived(signals())
    };
    let evidence = check_evidence(
        &content,
        Some(&transport),
        TENANT,
        at(14, 0, 0),
        // The clock went back: never a negative wait.
        at(13, 59, 0),
    );
    assert_eq!(evidence.auth_as.as_deref(), Some("Internal"));
    assert_eq!(evidence.directionality.unwrap().chars().count(), 200);
    // Whole, to paste into Get-MailDetailTransportRuleReport.
    assert_eq!(evidence.message_id.as_deref(), Some(message_id.as_str()));
    assert_eq!(evidence.found_after_secs, Some(0));
}

#[test]
fn an_empty_auth_as_is_none_and_never_counts_as_outside() {
    for transport in [
        Some("X-Signature-Stamped: 1\r\nX-MS-Exchange-Organization-AuthAs:   \r\n\r\n"),
        Some("X-Signature-Stamped: 1\r\n\r\n"),
        None,
    ] {
        let content = arrived(MessageSignals {
            relay_stamped: true,
            auth_as: Some("\u{200F}".into()),
            ..signals()
        });
        let evidence = check_evidence(&content, transport, TENANT, at(14, 0, 0), at(14, 1, 0));
        assert!(evidence.relay_stamped, "{transport:?}");
        assert_eq!(evidence.auth_as, None, "{transport:?}");
        assert!(!relay_not_internal(&evidence), "{transport:?}");
    }
    // Stamped but empty: not the relay.
    let evidence = check_evidence(
        &arrived(signals()),
        Some("X-Signature-Stamped: \r\nX-MS-Exchange-Organization-AuthAs: Anonymous\r\n"),
        TENANT,
        at(14, 0, 0),
        at(14, 1, 0),
    );
    assert!(!evidence.relay_stamped);
    assert!(!relay_not_internal(&evidence));
    // Not internal, in any case.
    let returned = |auth_as: &str| CheckEvidence {
        relay_stamped: true,
        auth_as: Some(auth_as.into()),
        ..CheckEvidence::default()
    };
    assert!(!relay_not_internal(&returned("internal")));
    assert!(relay_not_internal(&returned("Anonymous")));
    assert!(relay_not_internal(&returned("Partner")));
}

#[test]
fn powershell_strings_double_every_quote_that_ends_them() {
    assert_eq!(ps_quote("O'Brien’s"), "'O''Brien’’s'");
    assert_eq!(ps_quote("‘a‚b‛"), "'‘‘a‚‚b‛‛'");
    assert_eq!(ps_quote("a'|iex#@contoso.com"), "'a''|iex#@contoso.com'");
    assert_eq!(ps_quote(""), "''");
}

// --- Copy details for IT -----------------------------------------------------------

fn details_fixture() -> (Option<LastControl>, Vec<CheckDetailsItem>) {
    let control = LastControl {
        sent_at: at(14, 0, 0),
        outcome: crate::store::ControlOutcome {
            protected: false,
            label_id: None,
            evidence: Some(CheckEvidence {
                message_id: Some("<c1@contoso.com>".into()),
                relay_stamped: true,
                auth_as: Some("Anonymous".into()),
                directionality: Some("Originating".into()),
                headers_from: HeadersFrom::Transport,
                found_after_secs: Some(40),
                ..CheckEvidence::default()
            }),
        },
    };
    let encrypt = CheckEvidence {
        message_id: Some("<e1@contoso.com>".into()),
        relay_stamped: true,
        auth_as: Some("Anonymous".into()),
        directionality: Some("Originating".into()),
        detected: Some(ProtectionKind::RightsManaged),
        content_class: Some("rpmsg.message".into()),
        message_class: Some("IPM.Note.rpmsg.Microsoft Information Protection".into()),
        rpmsg_attachment: true,
        headers_from: HeadersFrom::Transport,
        found_after_secs: Some(40),
        ..CheckEvidence::default()
    };
    let label = CheckEvidence {
        message_id: Some("<l1@contoso.com>".into()),
        request_arrived: true,
        relay_stamped: true,
        label_stamped: Some(LABEL.into()),
        auth_as: Some("Anonymous".into()),
        directionality: Some("Originating".into()),
        own_labels: vec![SeenLabel {
            id: OTHER_LABEL.into(),
            enabled: false,
        }],
        foreign_labels: 1,
        headers_from: HeadersFrom::Transport,
        found_after_secs: Some(50),
        ..CheckEvidence::default()
    };
    let items = vec![
        CheckDetailsItem {
            title: "Encrypt".into(),
            item: CheckItem::Action {
                action: ProtectAction::Encrypt,
            },
            result: CheckResult {
                state: CheckState::Applied,
                at: Some(at(14, 1, 0)),
                test_message: true,
                passed_at: Some(at(14, 1, 0)),
                ..CheckResult::default()
            },
            record: CheckRecord {
                state: CheckState::Applied,
                at: Some(at(14, 1, 0)),
                test_message_id: Some("AAMk-1".into()),
                evidence: Some(encrypt),
                sent_at: Some(at(14, 0, 1)),
                ..CheckRecord::default()
            },
        },
        CheckDetailsItem {
            title: "Do Not Forward".into(),
            item: CheckItem::Action {
                action: ProtectAction::DoNotForward,
            },
            result: CheckResult::default(),
            record: CheckRecord::default(),
        },
        CheckDetailsItem {
            title: format!("Label Confidential ({LABEL})"),
            item: CheckItem::Label {
                label_id: LABEL.into(),
            },
            result: CheckResult {
                state: CheckState::NotApplied,
                at: Some(at(14, 1, 0)),
                note: Some(CheckNote::RelayNotInternal),
                test_message: true,
                changed_at: Some(at(13, 51, 0)),
                ..CheckResult::default()
            },
            record: CheckRecord {
                state: CheckState::NotApplied,
                at: Some(at(14, 1, 0)),
                note: Some(CheckNote::RelayNotInternal),
                test_message_id: Some("AAMk-2".into()),
                evidence: Some(label),
                changed_at: Some(at(13, 51, 0)),
                sent_at: Some(at(14, 0, 2)),
                ..CheckRecord::default()
            },
        },
    ];
    (Some(control), items)
}

#[test]
fn check_details_say_what_each_message_showed_in_a_fixed_shape() {
    let (control, items) = details_fixture();
    let catalog = catalog();
    let settle = Settle {
        rules_changed_at: Some(at(13, 30, 0)),
        policies_changed_at: Some(at(13, 51, 0)),
    };
    let text = check_details(&CheckDetailsInput {
        email: "adele@contoso.com",
        tenant: TENANT,
        version: "0.5.0",
        control: control.as_ref(),
        items,
        catalog: Some(&catalog),
        settle: &settle,
    });
    let expected = format!(
        "Cumulas Mail 0.5.0: setup check details for adele@contoso.com
Organization (tenant): {TENANT}
These details include no message text. They include the test messages' Message-IDs and what Microsoft 365 said about how it handled them.

Control message (no request), sent 2026-10-19 14:00:00 UTC, found within 40 s
  Message-ID: <c1@contoso.com>
  Went through the Cumulas Signatures relay (X-Signature-Stamped): yes
  Counted as from inside your organization (X-MS-Exchange-Organization-AuthAs): Anonymous
  Direction (X-MS-Exchange-Organization-MessageDirectionality): Originating
  Arrived: nothing; label of your organization: none
  Headers read from: the message's transport headers
Path: relay present, return not internal

Encrypt: Arrived encrypted · checked 2026-10-19 14:01 UTC
  Sent: x-cumulas-protect: encrypt, 2026-10-19 14:00:01 UTC
  Message-ID: <e1@contoso.com>
  Request still on the message when it arrived: no
  Relay: yes · AuthAs: Anonymous · Direction: Originating
  Arrived: Microsoft Purview encryption · Content-Class: rpmsg.message · Message class: IPM.Note.rpmsg.Microsoft Information Protection · message.rpmsg: yes
  Labels on it: none · other organizations' labels: 0
Do Not Forward: Not checked yet
Label Confidential ({LABEL}): Arrived without it · checked 2026-10-19 14:01 UTC
  Note: Your mail goes through the Cumulas Signatures relay, and Microsoft 365 treats it as outside mail when it comes back, so labels can't work yet. They need Cumulas Signatures to count mail coming back from its server as mail from inside your organization: your IT team turns that on in Cumulas Signatures where it's offered (Finish setting up); otherwise ask Cumulas. Your organization's setup changed 10 minutes before this check; if that's under an hour (a day for labels), check again later.
  Sent: x-cumulas-label: {LABEL}, 2026-10-19 14:00:02 UTC
  Message-ID: <l1@contoso.com>
  Request still on the message when it arrived: yes
  Arrived with x-cumulas-label-stamped: {LABEL}
  Relay: yes · AuthAs: Anonymous · Direction: Originating
  Arrived: nothing · Content-Class: not said · Message class: not said · message.rpmsg: no
  Labels on it: {OTHER_LABEL}, not enabled · other organizations' labels: 1

Setup changed on this computer: rules 2026-10-19 13:30:00 UTC, auto-labeling policies 2026-10-19 13:51:00 UTC

For your IT team, in Exchange Online PowerShell:
  Get-MessageTraceV2 -SenderAddress 'adele@contoso.com' -RecipientAddress 'adele@contoso.com' -Subject 'Cumulas Mail setup check' -SubjectFilterType StartsWith -StartDate (Get-Date).AddDays(-2) -EndDate (Get-Date) | ForEach-Object {{ Get-MessageTraceDetailV2 -MessageTraceId $_.MessageTraceId -RecipientAddress 'adele@contoso.com' }} | Format-List Date,Event,Detail
  Get-MailDetailTransportRuleReport -MessageId '<c1@contoso.com>' -EventType TransportRuleHits,TransportRuleActionHits -StartDate (Get-Date).AddDays(-2) -EndDate (Get-Date) | Format-List
  Get-MailDetailTransportRuleReport -MessageId '<e1@contoso.com>' -EventType TransportRuleHits,TransportRuleActionHits -StartDate (Get-Date).AddDays(-2) -EndDate (Get-Date) | Format-List
  Get-MailDetailTransportRuleReport -MessageId '<l1@contoso.com>' -EventType TransportRuleHits,TransportRuleActionHits -StartDate (Get-Date).AddDays(-2) -EndDate (Get-Date) | Format-List
Guide: https://github.com/cumulas-inc/email-client/blob/main/docs/microsoft-365.md#troubleshooting-the-check
"
    );
    assert_eq!(text, expected);
}

#[test]
fn check_details_keep_every_value_inside_one_powershell_string() {
    let (control, mut items) = details_fixture();
    let crafted = "<x'; Remove-TransportRule *>";
    items[0].record.evidence.as_mut().unwrap().message_id = Some(crafted.into());
    let text = check_details(&CheckDetailsInput {
        email: "a'|iex#@contoso.com",
        tenant: TENANT,
        version: "0.5.0",
        control: control.as_ref(),
        items,
        catalog: None,
        settle: &Settle::default(),
    });
    assert!(text.contains(
        "Get-MessageTraceV2 -SenderAddress 'a''|iex#@contoso.com' -RecipientAddress \
         'a''|iex#@contoso.com' -Subject"
    ));
    assert!(text.contains(
        "Get-MailDetailTransportRuleReport -MessageId '<x''; Remove-TransportRule *>' -EventType"
    ));
    // Every command line's quotes pair up: no value ends its string.
    for line in text.lines().filter(|l| l.starts_with("  Get-")) {
        assert_eq!(line.matches('\'').count() % 2, 0, "{line}");
    }
    assert!(
        text.contains("Setup changed on this computer: rules never, auto-labeling policies never")
    );
    // Without the label list, a label is its id.
    assert!(text.contains(&format!("Label Confidential ({LABEL}): Arrived without it")));
}

#[test]
fn check_details_show_the_path_when_nothing_else_arrived() {
    let (control, _) = details_fixture();
    let not_arrived = |title: &str, item: CheckItem| CheckDetailsItem {
        title: title.into(),
        item,
        result: CheckResult {
            state: CheckState::Unknown,
            at: Some(at(14, 16, 0)),
            note: Some(CheckNote::NotArrived),
            ..CheckResult::default()
        },
        record: CheckRecord {
            state: CheckState::Unknown,
            at: Some(at(14, 16, 0)),
            note: Some(CheckNote::NotArrived),
            sent_at: Some(at(14, 0, 1)),
            ..CheckRecord::default()
        },
    };
    let items = vec![
        not_arrived(
            "Encrypt",
            CheckItem::Action {
                action: ProtectAction::Encrypt,
            },
        ),
        not_arrived(
            "Do Not Forward",
            CheckItem::Action {
                action: ProtectAction::DoNotForward,
            },
        ),
    ];
    let input = |control: Option<&LastControl>, items: Vec<CheckDetailsItem>| {
        check_details(&CheckDetailsInput {
            email: "adele@contoso.com",
            tenant: TENANT,
            version: "0.5.0",
            control,
            items,
            catalog: None,
            settle: &Settle::default(),
        })
    };
    let text = input(control.as_ref(), items.clone());
    assert!(text.contains("\nPath: relay present, return not internal\n"));
    assert!(text.contains(
        "Encrypt: Couldn't check · 2026-10-19 14:16 UTC
  Note: The test message didn't arrive in your Inbox within 15 minutes.
  Sent: x-cumulas-protect: encrypt, 2026-10-19 14:00:01 UTC
  Arrived: not kept
Do Not Forward: Couldn't check"
    ));
    // Only the control arrived: one rule report.
    assert_eq!(text.matches("Get-MailDetailTransportRuleReport").count(), 1);

    // No control kept, or the relay not in the path.
    let text = input(None, items.clone());
    assert!(text.contains("Control message (no request): not found\nPath: not known\n"));
    assert!(!text.contains("Get-MailDetailTransportRuleReport"));
    let mut direct = control.unwrap();
    let evidence = direct.outcome.evidence.as_mut().unwrap();
    evidence.relay_stamped = false;
    evidence.auth_as = Some("Internal".into());
    assert!(input(Some(&direct), items.clone()).contains("\nPath: no relay\n"));
    let evidence = direct.outcome.evidence.as_mut().unwrap();
    evidence.relay_stamped = true;
    assert!(
        input(Some(&direct), items.clone()).contains("\nPath: relay present, return internal\n")
    );
    direct.outcome.evidence.as_mut().unwrap().auth_as = None;
    assert!(
        input(Some(&direct), items.clone()).contains("\nPath: relay present, return not known\n")
    );
}

#[test]
fn diagnostics_say_how_the_message_came_and_where_its_request_was_read() {
    let content = MessageContent {
        internet_message_id: Some("<a1@contoso.com>".into()),
        ..arrived(MessageSignals {
            relay_stamped: true,
            auth_as: Some("Anonymous".into()),
            directionality: Some("Originating".into()),
            requested_action: Some("encrypt".into()),
            requested_action_source: Some("property".into()),
            requested_label: Some(LABEL.into()),
            requested_label_source: Some("header".into()),
            ..signals()
        })
    };
    let input = |content| DiagnosticsInput {
        account: DiagnosticsAccount::MicrosoftWork,
        folder_role: FolderRole::Sent,
        content,
        protection: None,
        tenant_id: Some(TENANT),
        catalog: None,
    };
    let text = diagnostics(&input(&content));
    let tail = format!(
        "Requested by Cumulas Mail: encrypt · label: {LABEL}
Relay (X-Signature-Stamped): yes
Counted as internal (AuthAs): Anonymous
Direction: Originating
Request read from: the message property and the header
"
    );
    assert!(text.ends_with(&tail), "{text}");
    // A text for bug reports: no identifiers.
    assert!(!text.contains("a1@contoso.com"));

    let one_source = MessageContent {
        signals: MessageSignals {
            requested_label_source: Some("property".into()),
            ..content.signals.clone()
        },
        ..content.clone()
    };
    assert!(
        diagnostics(&input(&one_source)).contains("\nRequest read from: the message property\n")
    );
}
