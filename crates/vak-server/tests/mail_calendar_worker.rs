//! Untrusted CalDAV/iCalendar content is parsed only in a bounded worker.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::Path;

fn worker() -> &'static Path {
    Path::new(env!("CARGO_BIN_EXE_vak-tool-worker"))
}

#[tokio::test]
async fn caldav_multistatus_is_parsed_in_the_isolated_worker() {
    let response = r#"<?xml version="1.0"?><d:multistatus xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav"><d:response><d:propstat><d:prop><c:calendar-data>BEGIN:VCALENDAR&#13;&#10;BEGIN:VEVENT&#13;&#10;UID:worker-event&#13;&#10;DTSTART:20260930T100000Z&#13;&#10;SUMMARY:Worker preview&#13;&#10;END:VEVENT&#13;&#10;END:VCALENDAR&#13;&#10;</c:calendar-data></d:prop></d:propstat></d:response></d:multistatus>"#;
    let events = vak_tools::broker::parse_icalendar(worker(), response)
        .await
        .unwrap();
    assert_eq!(events[0]["provider_id"], "worker-event");
    assert_eq!(events[0]["title"], "Worker preview");
    assert_eq!(events[0]["starts_at"], "2026-09-30T10:00:00Z");
}

#[tokio::test]
async fn caldav_discovery_xml_is_parsed_in_the_isolated_worker() {
    let response = r#"<d:multistatus xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav"><d:response><d:href>/calendars/owner/work</d:href><d:propstat><d:prop><d:displayname>Work</d:displayname><d:resourcetype><d:collection/><c:calendar/></d:resourcetype></d:prop></d:propstat></d:response></d:multistatus>"#;
    let collections = vak_tools::broker::parse_caldav_discovery(
        worker(),
        response,
        vak_tools::mail_calendar::DiscoveryMode::CalendarCollections,
    )
    .await
    .unwrap();
    assert_eq!(collections[0]["href"], "/calendars/owner/work");
    assert_eq!(collections[0]["display_name"], "Work");
}

#[tokio::test]
async fn caldav_parser_fails_closed_when_worker_is_missing_or_input_is_oversized() {
    let missing = vak_tools::broker::parse_icalendar(
        Path::new("/nonexistent/vak-tool-worker"),
        "BEGIN:VCALENDAR\r\nEND:VCALENDAR\r\n",
    )
    .await;
    assert!(missing.is_err());

    let oversized = vak_tools::broker::parse_icalendar(worker(), &"x".repeat(256 * 1024 + 1)).await;
    assert!(oversized.is_err());
}
