//! Contacts (default contacts folder) on Microsoft Graph.

use async_trait::async_trait;
use chrono::DateTime;
use reqwest::Method;
use serde::Deserialize;
use serde_json::{Value, json};

use super::GraphProvider;
use super::client::{PREFER_IMMUTABLE_ID, segment};
use super::types::{GraphEmailAddress, email_address_json, nearest_date, non_empty, null_default};
use crate::error::{Error, Result};
use crate::model::{Contact, ContactDraft, EntityKind, Phone, local_id};
use crate::providers::ContactsProvider;

const CONTACT_FIELDS: &str = "id,displayName,givenName,surname,emailAddresses,businessPhones,\
     homePhones,mobilePhone,companyName,jobTitle,department,businessAddress,homeAddress,\
     otherAddress,birthday,personalNotes";
const PREFER: &[&str] = &[PREFER_IMMUTABLE_ID];
/// Outlook contacts hold at most three email addresses.
const MAX_EMAILS: usize = 3;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GraphContact {
    id: String,
    display_name: Option<String>,
    given_name: Option<String>,
    surname: Option<String>,
    #[serde(default, deserialize_with = "null_default")]
    email_addresses: Vec<GraphEmailAddress>,
    #[serde(default, deserialize_with = "null_default")]
    business_phones: Vec<String>,
    #[serde(default, deserialize_with = "null_default")]
    home_phones: Vec<String>,
    mobile_phone: Option<String>,
    company_name: Option<String>,
    job_title: Option<String>,
    department: Option<String>,
    business_address: Option<PhysicalAddress>,
    home_address: Option<PhysicalAddress>,
    other_address: Option<PhysicalAddress>,
    /// Read leniently: clients write all sorts of values here, and one bad
    /// birthday must not fail the whole contact list.
    birthday: Option<String>,
    personal_notes: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PhysicalAddress {
    street: Option<String>,
    city: Option<String>,
    state: Option<String>,
    postal_code: Option<String>,
    country_or_region: Option<String>,
}

impl PhysicalAddress {
    fn one_line(&self) -> Option<String> {
        let parts: Vec<String> = [
            &self.street,
            &self.city,
            &self.state,
            &self.postal_code,
            &self.country_or_region,
        ]
        .into_iter()
        .filter_map(|part| {
            let lines: Vec<&str> = part
                .as_deref()?
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .collect();
            (!lines.is_empty()).then(|| lines.join(", "))
        })
        .collect();
        (!parts.is_empty()).then(|| parts.join(", "))
    }
}

/// Graph property holding the address shown as the contact's one-line
/// address (the first non-empty of business, home, other).
fn address_property(contact: &GraphContact) -> Option<(&'static str, String)> {
    [
        ("businessAddress", &contact.business_address),
        ("homeAddress", &contact.home_address),
        ("otherAddress", &contact.other_address),
    ]
    .into_iter()
    .find_map(|(property, address)| Some((property, address.as_ref()?.one_line()?)))
}

/// A one-line address written back as the street; the other parts are
/// cleared so the address reads as entered.
fn address_json(line: Option<&str>) -> Value {
    json!({
        "street": line.unwrap_or_default(),
        "city": "",
        "state": "",
        "postalCode": "",
        "countryOrRegion": "",
    })
}

impl GraphContact {
    fn into_contact(self, account_id: &str) -> Contact {
        let address = address_property(&self).map(|(_, line)| line);
        let emails: Vec<_> = self
            .email_addresses
            .iter()
            .filter_map(GraphEmailAddress::to_model)
            .collect();
        let mut phones = Vec::new();
        if let Some(number) = non_empty(self.mobile_phone) {
            phones.push(Phone {
                kind: "mobile".into(),
                number,
            });
        }
        for (kind, numbers) in [
            ("business", self.business_phones),
            ("home", self.home_phones),
        ] {
            phones.extend(numbers.into_iter().filter_map(|n| {
                non_empty(Some(n)).map(|number| Phone {
                    kind: kind.into(),
                    number,
                })
            }));
        }
        let given_name = non_empty(self.given_name);
        let surname = non_empty(self.surname);
        let display_name = non_empty(self.display_name)
            .or_else(|| {
                let full = [given_name.as_deref(), surname.as_deref()]
                    .into_iter()
                    .flatten()
                    .collect::<Vec<_>>()
                    .join(" ");
                non_empty(Some(full))
            })
            .or_else(|| emails.first().map(|e| e.address.clone()))
            .unwrap_or_default();
        Contact {
            id: local_id(account_id, EntityKind::Contact, &self.id),
            account_id: account_id.to_owned(),
            remote_id: self.id,
            display_name,
            given_name,
            surname,
            emails,
            phones,
            company: non_empty(self.company_name),
            job_title: non_empty(self.job_title),
            department: non_empty(self.department),
            address,
            // Clients store birthdays at various times of day (often local
            // midnight); the nearest date is the birthday.
            birthday: self
                .birthday
                .and_then(|b| DateTime::parse_from_rfc3339(b.trim()).ok())
                .map(|b| nearest_date(b.naive_utc())),
            notes: non_empty(self.personal_notes),
        }
    }
}

/// Request body for creating or updating a contact. Fields the draft leaves
/// empty are cleared. Addresses are handled by the caller.
fn contact_json(draft: &ContactDraft) -> Result<Value> {
    if draft.emails.len() > MAX_EMAILS {
        return Err(Error::invalid_input(
            "Microsoft accounts store at most three email addresses per contact.",
        ));
    }
    let mut mobile = None;
    let mut business = Vec::new();
    let mut home = Vec::new();
    for phone in &draft.phones {
        let number = phone.number.trim();
        if number.is_empty() {
            continue;
        }
        match phone.kind.as_str() {
            "mobile" if mobile.is_none() => mobile = Some(number),
            "home" => home.push(number),
            // Graph has a single mobile number and no "other" numbers; extra
            // numbers are kept as business numbers rather than dropped.
            _ => business.push(number),
        }
    }
    let mut json = json!({
        "givenName": draft.given_name,
        "surname": draft.surname,
        "emailAddresses": draft.emails.iter().map(email_address_json).collect::<Vec<_>>(),
        "businessPhones": business,
        "homePhones": home,
        "mobilePhone": mobile,
        "companyName": draft.company,
        "jobTitle": draft.job_title,
        "department": draft.department,
        // Late morning UTC keeps the same date in every time zone from
        // UTC-11 to UTC+12.
        "birthday": draft.birthday.map(|d| format!("{d}T11:59:59Z")),
        "personalNotes": draft.notes,
    });
    // Without a display name Outlook composes one from the name parts.
    if let Some(name) = non_empty(Some(draft.display_name.clone())) {
        json["displayName"] = name.into();
    }
    Ok(json)
}

impl GraphProvider {
    fn contact_url(&self, remote_id: &str) -> String {
        self.client
            .url(&format!("/me/contacts/{}", segment(remote_id)))
    }
}

#[async_trait]
impl ContactsProvider for GraphProvider {
    async fn list_contacts(&self) -> Result<Vec<Contact>> {
        let url = self.client.url_with_query(
            "/me/contacts",
            &[("$select", CONTACT_FIELDS), ("$top", "100")],
        );
        let contacts: Vec<GraphContact> = self.client.get_all(&url, PREFER).await?;
        Ok(contacts
            .into_iter()
            .map(|c| c.into_contact(&self.account_id))
            .collect())
    }

    async fn create_contact(&self, draft: &ContactDraft) -> Result<Contact> {
        let mut body = contact_json(draft)?;
        if let Some(line) = draft
            .address
            .as_deref()
            .map(str::trim)
            .filter(|a| !a.is_empty())
        {
            body["businessAddress"] = address_json(Some(line));
        }
        let created: GraphContact = self
            .client
            .send_json(
                Method::POST,
                &self.client.url("/me/contacts"),
                &body,
                PREFER,
            )
            .await?;
        Ok(created.into_contact(&self.account_id))
    }

    async fn update_contact(&self, remote_id: &str, draft: &ContactDraft) -> Result<Contact> {
        let mut body = contact_json(draft)?;
        // The model has a one-line address while Outlook stores structured
        // ones: only touch the address when the line was actually edited, so
        // street, city and postcode survive other edits.
        let url = self.client.url_with_query(
            &format!("/me/contacts/{}", segment(remote_id)),
            &[("$select", "id,businessAddress,homeAddress,otherAddress")],
        );
        let current: GraphContact = self.client.get_json(&url, PREFER).await?;
        let wanted = draft
            .address
            .as_deref()
            .map(str::trim)
            .filter(|a| !a.is_empty());
        match (address_property(&current), wanted) {
            (Some((_, line)), Some(wanted)) if line == wanted => {}
            (Some((property, _)), wanted) => body[property] = address_json(wanted),
            (None, Some(wanted)) => body["businessAddress"] = address_json(Some(wanted)),
            (None, None) => {}
        }
        let updated: GraphContact = self
            .client
            .send_json(Method::PATCH, &self.contact_url(remote_id), &body, PREFER)
            .await?;
        Ok(updated.into_contact(&self.account_id))
    }

    async fn delete_contact(&self, remote_id: &str) -> Result<()> {
        self.client
            .send_no_content(Method::DELETE, &self.contact_url(remote_id), None, PREFER)
            .await?;
        Ok(())
    }
}
