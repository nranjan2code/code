#![allow(clippy::expect_used)]

use std::time::Duration;
use vak_delivery::client::WorkerClient;
use vak_delivery::{
    AnswerDraft, DeliveryContent, DeliveryJob, DeliveryKind, DeliveryPosture, DeliveryProfile,
    Markup,
};

fn job(id: &str, markdown: &str) -> DeliveryJob {
    DeliveryJob {
        job_id: id.into(),
        target: "telegram:test".into(),
        kind: DeliveryKind::Assistant,
        content: DeliveryContent::Answer(AnswerDraft::from_markdown(markdown)),
        profile: DeliveryProfile {
            surface: "telegram".into(),
            markup: Markup::TelegramHtml,
            max_chars: Some(4000),
            supports_tables: false,
            supports_code_blocks: true,
            supports_links: true,
            supports_actions: false,
            template: None,
            posture: DeliveryPosture::default(),
        },
        skill_registry: None,
    }
}

#[tokio::test]
async fn persistent_worker_renders_multiple_jobs() {
    let worker = WorkerClient::new(
        env!("CARGO_BIN_EXE_vak-delivery-worker"),
        Duration::from_secs(3),
    );
    let first = worker
        .render(&job("worker-1", "# One\n\n**done**"))
        .await
        .expect("first render");
    let second = worker
        .render(&job("worker-2", "# Two\n\n`ok`"))
        .await
        .expect("second render");
    assert_eq!(first.job_id, "worker-1");
    assert_eq!(second.job_id, "worker-2");
    assert!(first.chunks[0].contains("<b>done</b>"));
    assert!(second.chunks[0].contains("<code>ok</code>"));
}
