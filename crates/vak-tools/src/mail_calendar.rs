//! Worker-only parsing for untrusted calendar payloads.

use chrono::{DateTime, NaiveDate, NaiveDateTime, TimeZone, Utc};
use ical::IcalParser;
use quick_xml::{Reader, events::Event as XmlEvent};
use serde_json::{Value, json};
use std::io::Cursor;

const MAX_CALENDAR_BYTES: usize = 256 * 1024;
const MAX_EVENTS: usize = 100;
const MAX_FIELD_CHARS: usize = 1024;
const MAX_XML_DEPTH: usize = 64;
const MAX_DISCOVERY_RESULTS: usize = 32;
const MAX_MIME_PARTS: usize = 64;
const MAX_MAIL_BODY_CHARS: usize = 16_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiscoveryMode {
    CurrentUserPrincipal,
    CalendarHomeSet,
    CalendarCollections,
}

/// Parse a bounded VCALENDAR document inside `__tool_worker`.
pub(crate) fn parse_calendar_data(data: &str) -> Result<String, String> {
    if data.is_empty() || data.len() > MAX_CALENDAR_BYTES || data.contains('\0') {
        return Err("calendar payload is empty or exceeds its size limit".into());
    }
    let mut events = Vec::new();
    let calendars = extract_calendar_data(data)?;
    if calendars.is_empty() {
        return Err("calendar payload contains no VCALENDAR".into());
    }
    for calendar_data in calendars {
        let mut calendars = IcalParser::new(Cursor::new(calendar_data.as_bytes()));
        for calendar in &mut calendars {
            let calendar = calendar.map_err(|_| "calendar payload is invalid".to_owned())?;
            for event in calendar.events {
                if events.len() >= MAX_EVENTS {
                    break;
                }
                let prop = |name: &str| {
                    event
                        .properties
                        .iter()
                        .find(|property| property.name.eq_ignore_ascii_case(name))
                };
                let value = |name: &str| prop(name).and_then(|property| property.value.as_deref());
                let uid = value("UID")
                    .map(str::trim)
                    .filter(|uid| !uid.is_empty() && !uid.chars().any(char::is_control))
                    .ok_or_else(|| "calendar event has no valid UID".to_owned())?;
                let start_property =
                    prop("DTSTART").ok_or_else(|| "calendar event has no DTSTART".to_owned())?;
                let start_value = start_property
                    .value
                    .as_deref()
                    .ok_or_else(|| "calendar event DTSTART is empty".to_owned())?;
                let start = parse_ical_time(start_value, start_property)
                    .ok_or_else(|| "calendar event DTSTART is unsupported".to_owned())?;
                let end = prop("DTEND")
                    .map(|property| {
                        property
                            .value
                            .as_deref()
                            .and_then(|value| parse_ical_time(value, property))
                            .ok_or_else(|| "calendar event DTEND is unsupported".to_owned())
                    })
                    .transpose()?;
                let (starts_at, starts_on, ends_at, ends_on, all_day) = match (start, end) {
                    (IcalTime::Date(start), Some(IcalTime::Date(end))) => (
                        None,
                        Some(start.to_string()),
                        None,
                        Some(end.to_string()),
                        true,
                    ),
                    (IcalTime::Date(start), None) => {
                        (None, Some(start.to_string()), None, None, true)
                    }
                    (IcalTime::DateTime(start), Some(IcalTime::DateTime(end))) => {
                        if end <= start {
                            return Err("calendar event ends before it starts".into());
                        }
                        (Some(start), None, Some(end), None, false)
                    }
                    (IcalTime::DateTime(start), None) => (Some(start), None, None, None, false),
                    _ => {
                        return Err("calendar event has mismatched start and end types".into());
                    }
                };
                let private = value("CLASS").is_some_and(|class| {
                    class.eq_ignore_ascii_case("PRIVATE")
                        || class.eq_ignore_ascii_case("CONFIDENTIAL")
                });
                let attendees = event
                    .properties
                    .iter()
                    .filter(|property| property.name.eq_ignore_ascii_case("ATTENDEE"))
                    .count();
                let recurring = event.properties.iter().any(|property| {
                    property.name.eq_ignore_ascii_case("RRULE")
                        || property.name.eq_ignore_ascii_case("RDATE")
                        || property.name.eq_ignore_ascii_case("RECURRENCE-ID")
                });
                events.push(json!({
            "provider_id": bounded(uid),
            "version": null,
            "title": if private { "Private event".to_owned() } else { value("SUMMARY").map(unescape_text).map(|s| bounded(&s)).filter(|s| !s.is_empty()).unwrap_or_else(|| "(no title)".into()) },
            "starts_at": starts_at,
                    "ends_at": ends_at,
            "starts_on": starts_on,
                    "ends_on": ends_on,
            "all_day": all_day,
            "location": if private { None::<String> } else { value("LOCATION").map(unescape_text).map(|s| bounded(&s)).filter(|s| !s.is_empty()) },
            "description": if private { None::<String> } else { value("DESCRIPTION").map(unescape_text).map(|s| bounded(&s)).filter(|s| !s.is_empty()) },
            "attendee_count": if private { 0 } else { attendees },
            "recurring": recurring,
            "private": private,
        }));
            }
            if events.len() >= MAX_EVENTS {
                break;
            }
        }
        if events.len() >= MAX_EVENTS {
            break;
        }
    }
    serde_json::to_string(&events).map_err(|_| "calendar results could not be encoded".into())
}

/// Decode only an explicit, non-attachment `text/plain` MIME part. This is
/// called only inside the isolated tool worker; HTML and attachment payloads
/// are never promoted to model-visible message text.
pub(crate) fn parse_mail_mime(data: &[u8]) -> Result<String, String> {
    if data.is_empty() || data.len() > MAX_CALENDAR_BYTES {
        return Err("email message is empty or exceeds its size limit".into());
    }
    let parsed =
        mailparse::parse_mail(data).map_err(|_| "email MIME structure is invalid".to_owned())?;
    let parts: Vec<_> = parsed.parts().take(MAX_MIME_PARTS + 1).collect();
    if parts.len() > MAX_MIME_PARTS {
        return Err("email has too many MIME parts".into());
    }
    for part in parts {
        let disposition = part.get_content_disposition();
        if disposition.disposition == mailparse::DispositionType::Attachment
            || disposition.params.contains_key("filename")
            || part.ctype.params.contains_key("name")
        {
            continue;
        }
        if !part.ctype.mimetype.eq_ignore_ascii_case("text/plain") || !part.subparts.is_empty() {
            continue;
        }
        let body = part
            .get_body()
            .map_err(|_| "email text part could not be decoded".to_owned())?;
        let body = body.trim();
        if body.is_empty() {
            continue;
        }
        let bounded = body.chars().take(MAX_MAIL_BODY_CHARS).collect::<String>();
        return serde_json::to_string(&json!({
            "body_text": bounded,
            "body_status": "available",
        }))
        .map_err(|_| "email text could not be encoded".into());
    }
    serde_json::to_string(&json!({
        "body_text": Value::Null,
        "body_status": "no_plain_text",
    }))
    .map_err(|_| "email text could not be encoded".into())
}

fn extract_calendar_data(document: &str) -> Result<Vec<String>, String> {
    if document.trim_start().starts_with("BEGIN:VCALENDAR") {
        return Ok(vec![document.to_owned()]);
    }
    let mut reader = Reader::from_str(document);
    reader.config_mut().check_end_names = true;
    let mut depth = 0usize;
    let mut calendar_depth = None;
    let mut current = String::new();
    let mut calendars = Vec::new();
    loop {
        match reader.read_event() {
            Ok(XmlEvent::Start(start)) => {
                depth += 1;
                if depth > MAX_XML_DEPTH {
                    return Err("CalDAV XML nesting exceeds its limit".into());
                }
                if local_name(start.name().as_ref()) == b"calendar-data" {
                    calendar_depth = Some(depth);
                    current.clear();
                }
            }
            Ok(XmlEvent::Empty(start)) => {
                if local_name(start.name().as_ref()) == b"calendar-data" {
                    if calendars.len() >= MAX_EVENTS {
                        return Err("CalDAV response contains too many calendars".into());
                    }
                    calendars.push(String::new());
                }
            }
            Ok(XmlEvent::Text(text)) if calendar_depth.is_some() => {
                let decoded = text
                    .decode()
                    .map_err(|_| "CalDAV XML text is invalid".to_owned())?;
                let unescaped = quick_xml::escape::unescape(&decoded)
                    .map_err(|_| "CalDAV XML entity is invalid".to_owned())?;
                current.push_str(&unescaped);
                if current.len() > MAX_CALENDAR_BYTES {
                    return Err("CalDAV calendar data exceeds its size limit".into());
                }
            }
            Ok(XmlEvent::CData(text)) if calendar_depth.is_some() => {
                current.push_str(
                    &text
                        .decode()
                        .map_err(|_| "CalDAV XML text is invalid".to_owned())?,
                );
                if current.len() > MAX_CALENDAR_BYTES {
                    return Err("CalDAV calendar data exceeds its size limit".into());
                }
            }
            Ok(XmlEvent::GeneralRef(reference)) if calendar_depth.is_some() => {
                let value = reference
                    .resolve_char_ref()
                    .map_err(|_| "CalDAV XML reference is invalid".to_owned())?
                    .or_else(|| {
                        reference
                            .decode()
                            .ok()
                            .and_then(|name| match name.as_ref() {
                                "amp" => Some('&'),
                                "apos" => Some('\''),
                                "gt" => Some('>'),
                                "lt" => Some('<'),
                                "quot" => Some('"'),
                                _ => None,
                            })
                    })
                    .ok_or_else(|| "CalDAV XML reference is unsupported".to_owned())?;
                current.push(value);
            }
            Ok(XmlEvent::End(end)) => {
                if calendar_depth == Some(depth)
                    && local_name(end.name().as_ref()) == b"calendar-data"
                {
                    if calendars.len() >= MAX_EVENTS {
                        return Err("CalDAV response contains too many calendars".into());
                    }
                    calendars.push(std::mem::take(&mut current));
                    calendar_depth = None;
                }
                depth = depth.saturating_sub(1);
            }
            Ok(XmlEvent::DocType(_)) => return Err("CalDAV XML document type is forbidden".into()),
            Ok(XmlEvent::Eof) => break,
            Ok(_) => {}
            Err(_) => return Err("CalDAV XML response is invalid".into()),
        }
    }
    Ok(calendars)
}

fn local_name(name: &[u8]) -> &[u8] {
    name.rsplit(|byte| *byte == b':').next().unwrap_or(name)
}

/// Read only the hrefs and calendar collection labels needed for CalDAV
/// discovery; this XML parser is called from the isolated worker only.
pub(crate) fn parse_caldav_discovery(
    document: &str,
    mode: DiscoveryMode,
) -> Result<String, String> {
    if document.is_empty() || document.len() > MAX_CALENDAR_BYTES || document.contains('\0') {
        return Err("CalDAV discovery response exceeds its size limit".into());
    }
    let mut reader = Reader::from_str(document);
    reader.config_mut().check_end_names = true;
    let mut stack = Vec::<Vec<u8>>::new();
    let mut depth = 0usize;
    let mut target_property: Option<Vec<u8>> = None;
    let mut collecting: Option<Vec<u8>> = None;
    let mut text = String::new();
    let mut hrefs = Vec::<String>::new();
    let mut results = Vec::<Value>::new();
    let mut response_href: Option<String> = None;
    let mut response_displayname: Option<String> = None;
    let mut response_calendar = false;
    loop {
        match reader.read_event() {
            Ok(XmlEvent::Start(start)) => {
                depth += 1;
                if depth > MAX_XML_DEPTH {
                    return Err("CalDAV XML nesting exceeds its limit".into());
                }
                let name = local_name(start.name().as_ref()).to_vec();
                if matches!(
                    name.as_slice(),
                    b"current-user-principal" | b"calendar-home-set"
                ) {
                    target_property = Some(name.clone());
                }
                if name.as_slice() == b"response" {
                    response_href = None;
                    response_displayname = None;
                    response_calendar = false;
                }
                if mode == DiscoveryMode::CalendarCollections
                    && name.as_slice() == b"calendar"
                    && stack
                        .iter()
                        .any(|parent| parent.as_slice() == b"resourcetype")
                {
                    response_calendar = true;
                }
                let target_name = match mode {
                    DiscoveryMode::CurrentUserPrincipal => b"current-user-principal".as_slice(),
                    DiscoveryMode::CalendarHomeSet => b"calendar-home-set".as_slice(),
                    DiscoveryMode::CalendarCollections => b"response".as_slice(),
                };
                if name.as_slice() == b"href"
                    && (target_property.as_deref() == Some(target_name)
                        || mode == DiscoveryMode::CalendarCollections
                            && stack.iter().any(|parent| parent.as_slice() == b"response"))
                {
                    collecting = Some(b"href".to_vec());
                    text.clear();
                } else if name.as_slice() == b"displayname"
                    && mode == DiscoveryMode::CalendarCollections
                {
                    collecting = Some(b"displayname".to_vec());
                    text.clear();
                }
                stack.push(name);
            }
            Ok(XmlEvent::Empty(start)) => {
                let name = local_name(start.name().as_ref()).to_vec();
                if mode == DiscoveryMode::CalendarCollections
                    && name.as_slice() == b"calendar"
                    && stack
                        .iter()
                        .any(|parent| parent.as_slice() == b"resourcetype")
                {
                    response_calendar = true;
                }
            }
            Ok(XmlEvent::Text(value)) if collecting.is_some() => {
                let decoded = value
                    .decode()
                    .map_err(|_| "CalDAV discovery XML text is invalid".to_owned())?;
                let unescaped = quick_xml::escape::unescape(&decoded)
                    .map_err(|_| "CalDAV discovery XML entity is invalid".to_owned())?;
                text.push_str(&unescaped);
                if text.len() > 2048 {
                    return Err("CalDAV discovery value exceeds its size limit".into());
                }
            }
            Ok(XmlEvent::CData(value)) if collecting.is_some() => {
                text.push_str(
                    &value
                        .decode()
                        .map_err(|_| "CalDAV discovery XML text is invalid".to_owned())?,
                );
                if text.len() > 2048 {
                    return Err("CalDAV discovery value exceeds its size limit".into());
                }
            }
            Ok(XmlEvent::GeneralRef(reference)) if collecting.is_some() => {
                let character = reference
                    .resolve_char_ref()
                    .map_err(|_| "CalDAV discovery XML reference is invalid".to_owned())?
                    .or_else(|| {
                        reference
                            .decode()
                            .ok()
                            .and_then(|name| match name.as_ref() {
                                "amp" => Some('&'),
                                "apos" => Some('\''),
                                "gt" => Some('>'),
                                "lt" => Some('<'),
                                "quot" => Some('"'),
                                _ => None,
                            })
                    })
                    .ok_or_else(|| "CalDAV discovery XML reference is unsupported".to_owned())?;
                text.push(character);
            }
            Ok(XmlEvent::End(end)) => {
                let name = local_name(end.name().as_ref()).to_vec();
                if name.as_slice() == b"href" && collecting.as_deref() == Some(b"href") {
                    if !text.is_empty() && !text.chars().any(char::is_control) && text.len() <= 2048
                    {
                        if mode == DiscoveryMode::CalendarCollections {
                            response_href = Some(std::mem::take(&mut text));
                        } else {
                            hrefs.push(std::mem::take(&mut text));
                        }
                    }
                    collecting = None;
                } else if name.as_slice() == b"displayname"
                    && collecting.as_deref() == Some(b"displayname")
                {
                    response_displayname = Some(bounded(&text));
                    text.clear();
                    collecting = None;
                }
                if name.as_slice() == b"current-user-principal"
                    || name.as_slice() == b"calendar-home-set"
                {
                    target_property = None;
                }
                if name.as_slice() == b"response" && mode == DiscoveryMode::CalendarCollections {
                    if response_calendar && let Some(href) = response_href.take() {
                        results.push(json!({
                            "href": href,
                            "display_name": response_displayname.take().filter(|s| !s.is_empty()),
                        }));
                        if results.len() > MAX_DISCOVERY_RESULTS {
                            return Err("CalDAV response contains too many calendars".into());
                        }
                    }
                }
                stack.pop();
                depth = depth.saturating_sub(1);
            }
            Ok(XmlEvent::DocType(_)) => {
                return Err("CalDAV XML document type is forbidden".into());
            }
            Ok(XmlEvent::Eof) => break,
            Ok(_) => {}
            Err(_) => return Err("CalDAV discovery XML is invalid".into()),
        }
    }
    let output = match mode {
        DiscoveryMode::CalendarCollections => json!(results),
        _ => {
            hrefs.sort();
            hrefs.dedup();
            hrefs.truncate(MAX_DISCOVERY_RESULTS);
            json!(hrefs)
        }
    };
    serde_json::to_string(&output).map_err(|_| "CalDAV discovery could not be encoded".into())
}

enum IcalTime {
    Date(NaiveDate),
    DateTime(DateTime<Utc>),
}

fn parse_ical_time(value: &str, property: &ical::property::Property) -> Option<IcalTime> {
    if let Ok(date) = NaiveDate::parse_from_str(value, "%Y%m%d") {
        return Some(IcalTime::Date(date));
    }
    let local =
        NaiveDateTime::parse_from_str(value.strip_suffix('Z').unwrap_or(value), "%Y%m%dT%H%M%S")
            .ok()?;
    if value.ends_with('Z') {
        return Some(IcalTime::DateTime(DateTime::from_naive_utc_and_offset(
            local, Utc,
        )));
    }
    let tzid = property
        .params
        .as_ref()?
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("TZID"))?
        .1
        .first()?;
    let zone = tzid.parse::<chrono_tz::Tz>().ok()?;
    let datetime = zone
        .from_local_datetime(&local)
        .single()?
        .with_timezone(&Utc);
    Some(IcalTime::DateTime(datetime))
}

fn unescape_text(value: &str) -> String {
    let mut output = String::with_capacity(value.len().min(MAX_FIELD_CHARS));
    let mut chars = value.chars();
    let mut count = 0;
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            match chars.next() {
                Some('n' | 'N') => output.push('\n'),
                Some(escaped @ (',' | ';' | '\\')) => output.push(escaped),
                Some(other) => output.push(other),
                None => output.push('\\'),
            }
        } else {
            output.push(ch);
        }
        count += 1;
        if count > MAX_FIELD_CHARS {
            break;
        }
    }
    output.retain(|ch| ch == '\n' || ch == '\t' || !ch.is_control());
    output
}

fn bounded(value: &str) -> String {
    value.chars().take(MAX_FIELD_CHARS).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn parser_bounds_and_redacts_untrusted_calendar_content() {
        let data = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:evt-1\r\nDTSTART;TZID=America/New_York:20260930T100000\r\nDTEND;TZID=America/New_York:20260930T110000\r\nSUMMARY:Planning\\, review\r\nATTENDEE:mailto:a@example.test\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nUID:evt-2\r\nDTSTART:20261001\r\nDTEND:20261002\r\nSUMMARY:Secret\r\nDESCRIPTION:private details\r\nCLASS:PRIVATE\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
        let parsed: Value = serde_json::from_str(&parse_calendar_data(data).unwrap()).unwrap();
        assert_eq!(parsed[0]["title"], "Planning, review");
        assert_eq!(parsed[0]["starts_at"], "2026-09-30T14:00:00Z");
        assert_eq!(parsed[0]["attendee_count"], 1);
        assert_eq!(parsed[1]["title"], "Private event");
        assert_eq!(parsed[1]["description"], Value::Null);
        assert_eq!(parsed[1]["starts_on"], "2026-10-01");
        assert_eq!(parsed[1]["ends_on"], "2026-10-02");
    }

    #[test]
    fn parser_extracts_escaped_icalendar_from_caldav_multistatus() {
        let xml = r#"<?xml version="1.0"?><d:multistatus xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav"><d:response><d:propstat><d:prop><c:calendar-data>BEGIN:VCALENDAR&#13;&#10;BEGIN:VEVENT&#13;&#10;UID:ev1&#13;&#10;DTSTART:20260930T100000Z&#13;&#10;SUMMARY:Escaped &amp; safe&#13;&#10;END:VEVENT&#13;&#10;END:VCALENDAR&#13;&#10;</c:calendar-data></d:prop></d:propstat></d:response></d:multistatus>"#;
        let parsed: Value = serde_json::from_str(&parse_calendar_data(xml).unwrap()).unwrap();
        assert_eq!(parsed[0]["title"], "Escaped & safe");
    }

    #[test]
    fn mime_parser_selects_plain_text_and_skips_html_and_attachments() {
        let email = b"MIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=outer\r\n\r\n--outer\r\nContent-Type: multipart/alternative; boundary=inner\r\n\r\n--inner\r\nContent-Type: text/html\r\n\r\n<p>html only</p>\r\n--inner\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: quoted-printable\r\n\r\nHello=2C mail\r\n--inner--\r\n--outer\r\nContent-Type: text/plain\r\nContent-Disposition: attachment; filename=secret.txt\r\n\r\nDO NOT EXPOSE\r\n--outer--\r\n";
        let parsed: Value = serde_json::from_str(&parse_mail_mime(email).expect("valid MIME"))
            .expect("worker JSON");
        assert_eq!(parsed["body_text"], "Hello, mail");
        assert_eq!(parsed["body_status"], "available");
        assert!(!parsed.to_string().contains("DO NOT EXPOSE"));
    }

    #[test]
    fn mime_parser_labels_html_only_and_rejects_oversized_messages() {
        let html = b"Content-Type: text/html\r\n\r\n<p>not plain text</p>";
        let parsed: Value =
            serde_json::from_str(&parse_mail_mime(html).expect("valid MIME")).expect("worker JSON");
        assert_eq!(parsed["body_text"], Value::Null);
        assert_eq!(parsed["body_status"], "no_plain_text");
        assert!(parse_mail_mime(&vec![b'x'; MAX_CALENDAR_BYTES + 1]).is_err());
    }

    #[test]
    fn parser_rejects_floating_times_and_oversized_documents() {
        let floating = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:x\r\nDTSTART:20260930T100000\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
        assert!(parse_calendar_data(floating).is_err());
        assert!(parse_calendar_data(&"x".repeat(MAX_CALENDAR_BYTES + 1)).is_err());
    }

    #[test]
    fn discovery_parser_extracts_scoped_principal_home_and_calendar_collections() {
        let principal = r#"<d:multistatus xmlns:d="DAV:"><d:response><d:propstat><d:prop><d:current-user-principal><d:href>/principal/owner</d:href></d:current-user-principal></d:prop></d:propstat></d:response></d:multistatus>"#;
        let homeset = r#"<d:multistatus xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav"><d:response><d:propstat><d:prop><c:calendar-home-set><d:href>/calendars/owner</d:href></c:calendar-home-set></d:prop></d:propstat></d:response></d:multistatus>"#;
        let collections = r#"<d:multistatus xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav"><d:response><d:href>/calendars/owner/work</d:href><d:propstat><d:prop><d:displayname>Work</d:displayname><d:resourcetype><d:collection/><c:calendar/></d:resourcetype></d:prop></d:propstat></d:response><d:response><d:href>/calendars/owner/files</d:href><d:propstat><d:prop><d:resourcetype><d:collection/></d:resourcetype></d:prop></d:propstat></d:response></d:multistatus>"#;
        let principal: Value = serde_json::from_str(
            &parse_caldav_discovery(principal, DiscoveryMode::CurrentUserPrincipal).unwrap(),
        )
        .unwrap();
        let homeset: Value = serde_json::from_str(
            &parse_caldav_discovery(homeset, DiscoveryMode::CalendarHomeSet).unwrap(),
        )
        .unwrap();
        let collections: Value = serde_json::from_str(
            &parse_caldav_discovery(collections, DiscoveryMode::CalendarCollections).unwrap(),
        )
        .unwrap();
        assert_eq!(principal, json!(["/principal/owner"]));
        assert_eq!(homeset, json!(["/calendars/owner"]));
        assert_eq!(
            collections,
            json!([{"href":"/calendars/owner/work","display_name":"Work"}])
        );
    }

    #[test]
    fn discovery_parser_rejects_dtd_and_excessive_depth() {
        assert!(
            parse_caldav_discovery(
                "<!DOCTYPE a [<!ENTITY x 'y'>]><a>&x;</a>",
                DiscoveryMode::CalendarCollections,
            )
            .is_err()
        );
        let deep = format!(
            "{}<d:href>/a</d:href>{}",
            "<d:a>".repeat(65),
            "</d:a>".repeat(65)
        );
        assert!(parse_caldav_discovery(&deep, DiscoveryMode::CalendarHomeSet).is_err());
    }
}
