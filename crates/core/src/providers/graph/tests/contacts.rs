//! Contacts.

use chrono::NaiveDate;
use serde_json::{Value, json};
use wiremock::matchers::{body_json, method, path, query_param, query_param_is_missing};
use wiremock::{Mock, Request, ResponseTemplate};

use super::{ACCOUNT, fixture, ok_json, prefers, setup};
use crate::error::ErrorCode;
use crate::model::{ContactDraft, EmailAddress, EntityKind, Phone, local_id};
use crate::providers::ContactsProvider;
use crate::providers::graph::client::PREFER_IMMUTABLE_ID;

fn phone(kind: &str, number: &str) -> Phone {
    Phone {
        kind: kind.into(),
        number: number.into(),
    }
}

#[tokio::test]
async fn contacts_are_mapped_across_pages() {
    let (server, provider) = setup().await;
    Mock::given(method("GET"))
        .and(path("/me/contacts"))
        .and(query_param_is_missing("$skip"))
        .and(prefers(PREFER_IMMUTABLE_ID))
        .respond_with(ok_json(fixture("contacts.json", &server)))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/me/contacts"))
        .and(query_param("$skip", "2"))
        .respond_with(ok_json(json!({
            "value": [{ "id": "AAMkContact-last", "displayName": "Last One" }]
        })))
        .expect(1)
        .mount(&server)
        .await;

    let contacts = provider.list_contacts().await.unwrap();
    assert_eq!(contacts.len(), 3);

    let pat = &contacts[0];
    assert_eq!(
        pat.id,
        local_id(ACCOUNT, EntityKind::Contact, "AAMkContact-pat")
    );
    assert_eq!(pat.account_id, ACCOUNT);
    assert_eq!(pat.display_name, "Pat Kim");
    assert_eq!(pat.given_name.as_deref(), Some("Pat"));
    assert_eq!(pat.surname.as_deref(), Some("Kim"));
    assert_eq!(
        pat.emails,
        vec![
            EmailAddress::new(Some("Pat Kim"), "pat.kim@fabrikam.com"),
            EmailAddress::new(Some("pat@example.org"), "pat@example.org"),
        ]
    );
    assert_eq!(
        pat.phones,
        vec![
            phone("mobile", "+1 425 555 0142"),
            phone("business", "+1 425 555 0100"),
            phone("home", "+1 425 555 0199"),
        ]
    );
    assert_eq!(pat.company.as_deref(), Some("Fabrikam"));
    assert_eq!(pat.job_title.as_deref(), Some("Buyer"));
    assert_eq!(pat.department.as_deref(), Some("Purchasing"));
    assert_eq!(
        pat.address.as_deref(),
        Some("1 Microsoft Way, Building 4, Redmond, WA, 98052, United States")
    );
    // Stored as local midnight in UTC+1.
    assert_eq!(pat.birthday, NaiveDate::from_ymd_opt(1984, 3, 9));
    assert_eq!(pat.notes.as_deref(), Some("Met at the trade fair."));

    let sam = &contacts[1];
    assert_eq!(sam.display_name, "Sam");
    assert_eq!(sam.surname, None);
    assert!(sam.emails.is_empty() && sam.phones.is_empty());
    assert_eq!(sam.address.as_deref(), Some("Leeds, United Kingdom"));
    assert_eq!(sam.birthday, NaiveDate::from_ymd_opt(1990, 11, 30));
    assert_eq!(sam.notes, None);

    assert_eq!(contacts[2].display_name, "Last One");
}

fn draft() -> ContactDraft {
    ContactDraft {
        account_id: ACCOUNT.into(),
        display_name: "Pat Kim".into(),
        given_name: Some("Pat".into()),
        surname: Some("Kim".into()),
        emails: vec![EmailAddress::new(None, "pat.kim@fabrikam.com")],
        phones: vec![
            phone("mobile", "+1 425 555 0142"),
            phone("mobile", "+1 425 555 0143"),
            phone("home", "+1 425 555 0199"),
            phone("other", "+1 425 555 0111"),
            phone("business", " "),
        ],
        company: Some("Fabrikam".into()),
        job_title: None,
        department: None,
        address: Some("1 Microsoft Way, Redmond".into()),
        birthday: NaiveDate::from_ymd_opt(1984, 3, 9),
        notes: Some("Met at the trade fair.".into()),
    }
}

#[tokio::test]
async fn contacts_are_created_from_drafts() {
    let (server, provider) = setup().await;
    Mock::given(method("POST"))
        .and(path("/me/contacts"))
        .and(body_json(json!({
            "displayName": "Pat Kim",
            "givenName": "Pat",
            "surname": "Kim",
            "emailAddresses": [{ "address": "pat.kim@fabrikam.com" }],
            "businessPhones": ["+1 425 555 0143", "+1 425 555 0111"],
            "homePhones": ["+1 425 555 0199"],
            "mobilePhone": "+1 425 555 0142",
            "companyName": "Fabrikam",
            "jobTitle": null,
            "department": null,
            "birthday": "1984-03-09T11:59:59Z",
            "personalNotes": "Met at the trade fair.",
            "businessAddress": {
                "street": "1 Microsoft Way, Redmond",
                "city": "",
                "state": "",
                "postalCode": "",
                "countryOrRegion": ""
            }
        })))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
            "id": "AAMkContact-new",
            "displayName": "Pat Kim",
            "birthday": "1984-03-09T11:59:59Z",
            "businessAddress": { "street": "1 Microsoft Way, Redmond" }
        })))
        .expect(1)
        .mount(&server)
        .await;
    let contact = provider.create_contact(&draft()).await.unwrap();
    assert_eq!(contact.remote_id, "AAMkContact-new");
    assert_eq!(contact.birthday, NaiveDate::from_ymd_opt(1984, 3, 9));
    assert_eq!(contact.address.as_deref(), Some("1 Microsoft Way, Redmond"));
}

fn current_with_structured_address() -> ResponseTemplate {
    ok_json(json!({
        "id": "AAMkContact-pat",
        "businessAddress": {},
        "homeAddress": {
            "street": "1 Microsoft Way",
            "city": "Redmond"
        },
        "otherAddress": {}
    }))
}

#[tokio::test]
async fn unchanged_addresses_keep_their_structure() {
    let (server, provider) = setup().await;
    Mock::given(method("GET"))
        .and(path("/me/contacts/AAMkContact-pat"))
        .respond_with(current_with_structured_address())
        .mount(&server)
        .await;
    Mock::given(method("PATCH"))
        .and(path("/me/contacts/AAMkContact-pat"))
        .and(|r: &Request| {
            let body: Value = r.body_json().unwrap();
            body.get("businessAddress").is_none() && body.get("homeAddress").is_none()
        })
        .respond_with(ok_json(
            json!({ "id": "AAMkContact-pat", "displayName": "Pat Kim" }),
        ))
        .expect(1)
        .mount(&server)
        .await;
    provider
        .update_contact("AAMkContact-pat", &draft())
        .await
        .unwrap();
}

#[tokio::test]
async fn edited_addresses_replace_the_slot_they_came_from() {
    let (server, provider) = setup().await;
    Mock::given(method("GET"))
        .and(path("/me/contacts/AAMkContact-pat"))
        .respond_with(current_with_structured_address())
        .mount(&server)
        .await;
    Mock::given(method("PATCH"))
        .and(path("/me/contacts/AAMkContact-pat"))
        .and(|r: &Request| {
            let body: Value = r.body_json().unwrap();
            body["homeAddress"]["street"] == "2 Main Street, Leeds"
                && body["homeAddress"]["city"] == ""
                && body.get("businessAddress").is_none()
        })
        .respond_with(ok_json(
            json!({ "id": "AAMkContact-pat", "displayName": "Pat Kim" }),
        ))
        .expect(1)
        .mount(&server)
        .await;
    let edited = ContactDraft {
        address: Some("2 Main Street, Leeds".into()),
        ..draft()
    };
    provider
        .update_contact("AAMkContact-pat", &edited)
        .await
        .unwrap();
}

#[tokio::test]
async fn contacts_are_deleted_and_validated() {
    let (server, provider) = setup().await;
    Mock::given(method("DELETE"))
        .and(path("/me/contacts/AAMkContact-pat"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    provider.delete_contact("AAMkContact-pat").await.unwrap();

    let too_many = ContactDraft {
        emails: (0..4)
            .map(|i| EmailAddress::new(None, &format!("p{i}@example.org")))
            .collect(),
        ..draft()
    };
    let error = provider.create_contact(&too_many).await.unwrap_err();
    assert_eq!(error.code, ErrorCode::InvalidInput);
}
