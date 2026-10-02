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

#[tokio::test]
async fn email_mime_is_parsed_in_the_isolated_worker() {
    let raw = b"MIME-Version: 1.0\r\nContent-Type: multipart/alternative; boundary=b\r\n\r\n--b\r\nContent-Type: text/html\r\n\r\n<b>hidden html</b>\r\n--b\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: quoted-printable\r\n\r\nHello=2C selected mail\r\n--b--\r\n";
    let parsed = vak_tools::broker::parse_mail_mime(worker(), raw)
        .await
        .unwrap();
    assert_eq!(parsed["body_text"], "Hello, selected mail");
    assert_eq!(parsed["body_status"], "available");
    assert!(!parsed.to_string().contains("hidden html"));

    let html_only = b"Content-Type: text/html\r\n\r\n<p>Safe <strong>formatted</strong> text</p><img src=\"https://tracking.example.test/pixel\"><script>active content</script>";
    let html_parsed = vak_tools::broker::parse_mail_mime(worker(), html_only)
        .await
        .unwrap();
    assert_eq!(html_parsed["body_text"], "Safe formatted text");
    assert_eq!(html_parsed["body_status"], "sanitized_html");
    assert!(!html_parsed.to_string().contains("tracking.example.test"));
    assert!(!html_parsed.to_string().contains("active content"));

    let missing =
        vak_tools::broker::parse_mail_mime(Path::new("/nonexistent/vak-tool-worker"), raw).await;
    assert!(missing.is_err());
}

#[tokio::test]
async fn selected_attachment_is_read_only_in_the_network_denied_document_worker() {
    let preview = vak_tools::broker::preview_mail_attachment(
        worker(),
        "invoice.txt",
        b"Invoice total: $24.00\n",
    )
    .await
    .unwrap();
    assert!(preview.contains("Invoice total: $24.00"));
    assert!(!preview.contains("/tmp/"));

    assert!(
        vak_tools::broker::preview_mail_attachment(worker(), "active.html", b"<script>x</script>")
            .await
            .is_err()
    );
    assert!(
        vak_tools::broker::preview_mail_attachment(
            worker(),
            "oversized.txt",
            &vec![b'x'; vak_tools::broker::MAX_MAIL_ATTACHMENT_PREVIEW_BYTES + 1],
        )
        .await
        .is_err()
    );
}
