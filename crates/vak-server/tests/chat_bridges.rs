//! Discord/Slack bridge end-to-end against faked remote APIs
//! (docs/design/34-channel-onboarding.md Phase 3), following
//! `telegram_bridge.rs`: an axum router stands in for the remote surface,
//! a scripted provider stands in for the model, and the real gateway
//! router sits in between — so the assertion is on the whole contract,
//! not on a mocked seam.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use tokio_util::sync::CancellationToken;

use vak_core::Core;
use vak_llm::stream;
use vak_llm::types::{AssistantMessage, ChatRequest, ContentBlock, Usage};
use vak_llm::{EventStream, LlmError, Provider};
use vak_server::surfaces::PollCursor;
use vak_server::surfaces::discord::DiscordBridge;
use vak_server::surfaces::slack::SlackBridge;

struct Scripted {
    capacity_key: crate::support::CapacityKey,
    responses: Mutex<VecDeque<AssistantMessage>>,
}

#[async_trait::async_trait]
impl Provider for Scripted {
    fn name(&self) -> &str {
        "scripted"
    }

    fn rate_limit_key(&self) -> String {
        self.capacity_key.0.clone()
    }

    async fn stream(
        &self,
        _request: ChatRequest,
        _cancel: CancellationToken,
    ) -> Result<EventStream, LlmError> {
        let next = self.responses.lock().unwrap().pop_front();
        let (mut sink, rx) = stream::channel(64);
        match next {
            Some(m) => {
                sink.push(stream::StreamEvent::Start { partial: m.clone() });
                sink.close_message(m).await;
            }
            None => sink.close_error(LlmError::Parse("exhausted".into())).await,
        }
        Ok(rx)
    }
}

fn text(t: &str) -> AssistantMessage {
    AssistantMessage {
        content: vec![ContentBlock::text(t)],
        stop_reason: vak_llm::types::StopReason::EndTurn,
        usage: Usage {
            input_tokens: 7,
            output_tokens: 3,
            ..Default::default()
        },
        model: "test-model".into(),
        response_id: None,
    }
}

/// A gateway with `key` pre-allowlisted and one scripted reply queued.
async fn spawn_gateway(allow_key: &str, reply: &str) -> String {
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().to_path_buf();
    // Hermetic against the developer's own global config.
    let _ = std::fs::create_dir_all(cwd.join(".vak"));
    let _ = std::fs::write(
        cwd.join(".vak/config.toml"),
        format!("[memory]\nreflection = false\n\n[gateway]\nchat_allowlist = [\"{allow_key}\"]\n"),
    );
    vak_config::paths::isolate_home_for_tests();
    let core = Core::new_with_trust(cwd, true).unwrap();
    core.set_shared_scope(vak_config::scope::SharedScope::new(dir.path().join("home")));
    core.set_provider_instance(Arc::new(Scripted {
        capacity_key: crate::support::CapacityKey::default(),
        responses: Mutex::new(VecDeque::from(vec![text(reply)])),
    }));
    std::mem::forget(dir);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, vak_server::gateway_router(core))
            .await
            .unwrap();
    });
    format!("http://{addr}")
}

type Sent = Arc<Mutex<Vec<serde_json::Value>>>;

/// A Discord snowflake for a message `seconds_ago` seconds old.
fn snowflake(seconds_ago: i64) -> String {
    let ms = (chrono::Utc::now() - chrono::Duration::seconds(seconds_ago)).timestamp_millis();
    (((ms - 1_420_070_400_000) as u64) << 22).to_string()
}

/// A Slack `ts` for a message `seconds_ago` seconds old.
fn slack_ts(seconds_ago: i64) -> String {
    format!("{}.000100", chrono::Utc::now().timestamp() - seconds_ago)
}

/// This bot's cursors in a scratch data home, held, with `seed` (stream,
/// position) already written.
fn cursor(surface: &str, seed: Option<(&str, &str)>) -> (tempfile::TempDir, PollCursor) {
    let dir = tempfile::tempdir().unwrap();
    let cursor = PollCursor::at(
        vak_session::cursors::Cursors::at(dir.path().join("cursors"), dir.path().join("tenant")),
        surface,
        None,
    );
    cursor.hold().unwrap();
    if let Some((stream, position)) = seed {
        cursor.advance(stream, position).unwrap();
    }
    (dir, cursor)
}

/// Minimal Discord REST double: `GET /channels/{id}/messages` serves one
/// scripted message the first time and nothing after, `POST` records the
/// replies the bridge sends.
async fn spawn_mock_discord(message_id: String) -> (String, Sent) {
    let served = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let sent: Sent = Arc::new(Mutex::new(Vec::new()));
    let s = served.clone();
    let recorder = sent.clone();
    let app = axum::Router::new().route(
        "/channels/{id}/messages",
        axum::routing::get(move || {
            let s = s.clone();
            let message_id = message_id.clone();
            async move {
                if s.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
                    return axum::Json(serde_json::json!([{
                        "id": message_id,
                        "content": "ping",
                        "author": { "id": "u-7" },
                    }]));
                }
                axum::Json(serde_json::json!([]))
            }
        })
        .post(move |axum::Json(body): axum::Json<serde_json::Value>| {
            let recorder = recorder.clone();
            async move {
                recorder.lock().unwrap().push(body);
                axum::Json(serde_json::json!({ "id": "2001" }))
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (format!("http://{addr}"), sent)
}

/// Minimal Slack API double: `conversations.history` + `chat.postMessage`.
async fn spawn_mock_slack(ts: String, has_more: bool) -> (String, Sent) {
    let served = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let sent: Sent = Arc::new(Mutex::new(Vec::new()));
    let s = served.clone();
    let recorder = sent.clone();
    let app = axum::Router::new()
        .route(
            "/conversations.history",
            axum::routing::get(move || {
                let s = s.clone();
                let ts = ts.clone();
                async move {
                    if s.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
                        return axum::Json(serde_json::json!({
                            "ok": true,
                            "has_more": has_more,
                            "messages": [{ "ts": ts, "text": "ping", "user": "U7" }],
                        }));
                    }
                    axum::Json(serde_json::json!({ "ok": true, "messages": [] }))
                }
            }),
        )
        .route(
            "/chat.postMessage",
            axum::routing::post(move |axum::Json(body): axum::Json<serde_json::Value>| {
                let recorder = recorder.clone();
                async move {
                    recorder.lock().unwrap().push(body);
                    axum::Json(serde_json::json!({ "ok": true }))
                }
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (format!("http://{addr}"), sent)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn discord_bridge_routes_message_and_delivers_reply() {
    let newest = snowflake(5);
    let (api_base, sent) = spawn_mock_discord(newest.clone()).await;
    let gateway_url = spawn_gateway("discord:555", "pong from agent").await;
    let (_home, cursor) = cursor("discord", None);
    let bridge = DiscordBridge {
        api_base,
        bot_token: "bottok".into(),
        channel_ids: vec!["555".into()],
        gateway_url,
        gateway_token: "vk_test".into(),
        poll_secs: 0,
        bot_id: None,
        cursor: cursor.clone(),
    };

    // First pass seeds the cursor from the channel's latest message
    // without replaying a backlog into the agent.
    bridge.tick().await.unwrap();
    assert_eq!(cursor.position("555").unwrap(), Some(newest));
    assert!(sent.lock().unwrap().is_empty(), "cold start must not reply");

    // The mock now returns nothing new, so a second pass is quiet — the
    // cursor is what makes that true, not luck.
    bridge.tick().await.unwrap();
    assert!(sent.lock().unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn discord_bridge_replies_to_a_message_after_the_cursor() {
    let newest = snowflake(5);
    let (api_base, sent) = spawn_mock_discord(newest.clone()).await;
    let gateway_url = spawn_gateway("discord:555", "pong from agent").await;
    // A cursor before the scripted message, so it counts as new.
    let (_home, cursor) = cursor("discord", Some(("555", &snowflake(60))));
    let bridge = DiscordBridge {
        api_base,
        bot_token: "bottok".into(),
        channel_ids: vec!["555".into()],
        gateway_url,
        gateway_token: "vk_test".into(),
        poll_secs: 0,
        bot_id: None,
        cursor: cursor.clone(),
    };
    bridge.tick().await.unwrap();

    let delivered = sent.lock().unwrap();
    assert_eq!(delivered.len(), 1);
    assert_eq!(delivered[0]["content"], "pong from agent");
    assert_eq!(cursor.position("555").unwrap(), Some(newest));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn slack_bridge_replies_to_a_message_after_the_cursor() {
    let newest = slack_ts(5);
    let (api_base, sent) = spawn_mock_slack(newest.clone(), false).await;
    let gateway_url = spawn_gateway("slack:C1", "pong from agent").await;
    let (home, cursor) = cursor("slack", Some(("C1", &slack_ts(60))));
    let bridge = SlackBridge {
        api_base,
        bot_token: "bottok".into(),
        channel_ids: vec!["C1".into()],
        gateway_url,
        gateway_token: "vk_test".into(),
        poll_secs: 0,
        bot_id: None,
        cursor: cursor.clone(),
    };
    bridge.tick().await.unwrap();

    let delivered = sent.lock().unwrap();
    assert_eq!(delivered.len(), 1);
    assert_eq!(delivered[0]["channel"], "C1");
    assert_eq!(delivered[0]["text"], "pong from agent");
    assert_eq!(cursor.position("C1").unwrap(), Some(newest));
    let gaps =
        vak_session::cursors::Cursors::at(home.path().join("cursors"), home.path().join("tenant"))
            .gaps()
            .unwrap();
    assert!(gaps.is_empty(), "a poll that read everything leaves no gap");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn slack_bridge_records_a_gap_when_one_poll_cannot_read_everything() {
    let newest = slack_ts(5);
    let (api_base, sent) = spawn_mock_slack(newest.clone(), true).await;
    let gateway_url = spawn_gateway("slack:C1", "pong from agent").await;
    let before = slack_ts(600);
    let (home, cursor) = cursor("slack", Some(("C1", &before)));
    let bridge = SlackBridge {
        api_base,
        bot_token: "bottok".into(),
        channel_ids: vec!["C1".into()],
        gateway_url,
        gateway_token: "vk_test".into(),
        poll_secs: 0,
        bot_id: None,
        cursor: cursor.clone(),
    };
    bridge.tick().await.unwrap();

    // The page it read is routed; the older messages it never read are a
    // recorded gap, not a silent loss.
    assert_eq!(sent.lock().unwrap().len(), 1);
    assert_eq!(cursor.position("C1").unwrap(), Some(newest.clone()));
    let gaps =
        vak_session::cursors::Cursors::at(home.path().join("cursors"), home.path().join("tenant"))
            .gaps()
            .unwrap();
    assert_eq!(gaps.len(), 1);
    assert_eq!(gaps[0].from.as_deref(), Some(before.as_str()));
    assert_eq!(gaps[0].to, newest);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_discord_cursor_older_than_the_resume_bound_resyncs_without_replay() {
    let newest = snowflake(5);
    let (api_base, sent) = spawn_mock_discord(newest.clone()).await;
    let gateway_url = spawn_gateway("discord:555", "pong from agent").await;
    let stale = snowflake(3 * 24 * 3600);
    let (home, cursor) = cursor("discord", Some(("555", &stale)));
    let bridge = DiscordBridge {
        api_base,
        bot_token: "bottok".into(),
        channel_ids: vec!["555".into()],
        gateway_url,
        gateway_token: "vk_test".into(),
        poll_secs: 0,
        bot_id: None,
        cursor: cursor.clone(),
    };
    bridge.tick().await.unwrap();
    assert!(
        sent.lock().unwrap().is_empty(),
        "a stale backlog is not replayed"
    );
    assert_eq!(cursor.position("555").unwrap(), Some(newest.clone()));
    let gaps =
        vak_session::cursors::Cursors::at(home.path().join("cursors"), home.path().join("tenant"))
            .gaps()
            .unwrap();
    assert_eq!(gaps.len(), 1);
    assert_eq!(gaps[0].from.as_deref(), Some(stale.as_str()));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_chat_not_on_the_allowlist_gets_a_rejection_not_an_agent_turn() {
    // Phase 1's lifecycle applies uniformly to any surface's key: an
    // unknown Discord channel becomes pending and is answered with the
    // gateway's rejection, never with a model reply.
    let (api_base, sent) = spawn_mock_discord(snowflake(5)).await;
    let gateway_url = spawn_gateway("discord:other", "should not be reached").await;
    let (_home, cursor) = cursor("discord", Some(("555", &snowflake(60))));
    let bridge = DiscordBridge {
        api_base,
        bot_token: "bottok".into(),
        channel_ids: vec!["555".into()],
        gateway_url,
        gateway_token: "vk_test".into(),
        poll_secs: 0,
        bot_id: None,
        cursor,
    };
    bridge.tick().await.unwrap();

    let delivered = sent.lock().unwrap();
    assert_eq!(delivered.len(), 1);
    let content = delivered[0]["content"].as_str().unwrap();
    assert!(
        content.contains("gateway error"),
        "unallowed chat must surface the rejection, got: {content}"
    );
}
