//! Owner verification of uncertain provider charges. This never dismisses a
//! reservation or sends anything to a provider.
use crate::{AppState, AuthenticatedPrincipal};
use axum::{
    Extension, Json,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};

pub(crate) async fn pending(
    State(state): State<AppState>,
    Extension(principal): Extension<AuthenticatedPrincipal>,
) -> Response {
    if !matches!(principal, AuthenticatedPrincipal::Operator) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let home = state.active_core().shared_scope().into_root();
    match tokio::task::spawn_blocking(move || {
        let ledger = vak_core::finops::FinOpsLedger::new(&home);
        let reservations = ledger.pending_reservations()?;
        let unaddressable_unknown_calls = ledger.accounted_rows()?.iter()
            .filter(|row| row.usd.is_none() && row.reservation_id.is_none()).count();
        Ok::<_, String>((reservations, unaddressable_unknown_calls))
    })
    .await
    {
        Ok(Ok((reservations, unaddressable_unknown_calls))) => {
            Json(serde_json::json!({"reservations": reservations, "unaddressable_unknown_calls": unaddressable_unknown_calls})).into_response()
        }
        _ => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct VerifiedCost {
    actual_usd: f64,
    verified_with_provider: bool,
}

pub(crate) async fn reconcile(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Extension(principal): Extension<AuthenticatedPrincipal>,
    Json(body): Json<VerifiedCost>,
) -> Response {
    if !matches!(principal, AuthenticatedPrincipal::Operator) {
        return StatusCode::FORBIDDEN.into_response();
    }
    if !body.verified_with_provider || !body.actual_usd.is_finite() || body.actual_usd < 0.0 {
        return StatusCode::BAD_REQUEST.into_response();
    }
    if state.any_running() {
        return (
            StatusCode::CONFLICT,
            "Wait for active work to finish before recording a charge.",
        )
            .into_response();
    }
    let actor = crate::request_actor(&state);
    let core = state.active_core();
    let mut cached_cores = vec![core.clone(), state.core.clone()];
    cached_cores.extend(
        state
            .sessions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .values()
            .map(|handle| handle.core.clone()),
    );
    let reconciled = tokio::task::spawn_blocking(move || {
        let ledger = vak_core::finops::FinOpsLedger::new(core.shared_scope().root());
        // A different process may still own the provider dispatch. Check its
        // durable run and liveness, not just this server's active handles.
        let pending = ledger
            .pending_reservations()?
            .into_iter()
            .find(|pending| pending.id == id);
        let owner = pending
            .as_ref()
            .map(|pending| pending.budget_session_id.clone());
        if let Some(trace) = pending.as_ref().and_then(|pending| pending.trace.as_ref()) {
            let runs = core.runs();
            let run = runs
                .get(trace.run)
                .map_err(|_| "run history unavailable".to_string())?
                .ok_or_else(|| "the reservation run could not be verified".to_string())?;
            if run.status == vak_session::runs::RunStatus::Running {
                let holder = run
                    .holder
                    .ok_or_else(|| "run owner unavailable".to_string())?;
                if runs
                    .is_alive(&holder, chrono::Utc::now())
                    .map_err(|_| "run liveness unavailable".to_string())?
                {
                    return Err("the reservation run is still active".to_string());
                }
            }
        }
        ledger.reconcile_reservation(&id, body.actual_usd, actor)?;
        if let Some(owner) = owner {
            for cached in &cached_cores {
                cached.forget_spend_gate(&owner);
            }
        }
        Ok::<_, String>(())
    })
    .await;
    match reconciled {
        Ok(Ok(_)) => StatusCode::NO_CONTENT.into_response(),
        Ok(Err(_)) => (
            StatusCode::CONFLICT,
            "The charge could not be verified or changed. Refresh and review its current state.",
        )
            .into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use tower::ServiceExt;
    use vak_agent::SpendGate;

    #[tokio::test]
    async fn verified_cost_routes_enforce_owner_auth_and_record_exactly_once() {
        vak_config::paths::isolate_home_for_tests();
        let dir = tempfile::tempdir().unwrap();
        let core = vak_core::Core::new(dir.path().to_path_buf()).unwrap();
        core.set_shared_scope(vak_config::scope::SharedScope::new(dir.path().join("home")));
        let state = AppState::new(core.clone());
        let home = core.shared_scope().into_root();
        let gate = vak_core::finops::CoreSpendGate::new(&home, &Default::default());
        let check = vak_agent::SpendCheck {
            model: "unpriced-fixture",
            provider: "fixture",
            session_id: "budget-fixture",
            est_input_tokens: 1,
            planned_output_tokens: 1,
            trace: None,
        };
        let reservation = gate.reserve_dispatch(&check).await.unwrap().unwrap();
        let participant_token = "budget-participant-fixture";
        crate::coworking::invite(
            &vak_core::grants::Grants::at(&core.shared_scope()),
            crate::coworking::AudienceGrant {
                grant_id: crate::coworking::test_grant_id("budget-guest"),
                principal_id: "budget-guest".into(),
                display_name: "Guest".into(),
                conversation_id: "budget-fixture".into(),
                audience_id: "local".into(),
                capabilities: vec!["read".into()],
                token_hash: crate::coworking::token_hash(participant_token),
                created_at: chrono::Utc::now(),
                expires_at: chrono::Utc::now() + chrono::Duration::hours(1),
                trace: None,
                actor: None,
            },
        )
        .unwrap();
        let auth = crate::AuthPolicy {
            token: "budget-owner-fixture".into(),
            home: core.scope().into_root(),
            shared: home.clone(),
            trusted_hosts: Vec::new(),
            public_url: None,
            browser_sessions: crate::web::BrowserSessions::default(),
        };
        let app = axum::Router::new()
            .route("/finops/reservations", axum::routing::get(pending))
            .route(
                "/finops/reservations/{id}/reconcile",
                axum::routing::post(reconcile),
            )
            .with_state(state.clone())
            .layer(axum::middleware::from_fn_with_state(
                state,
                crate::enforce_participant_audience,
            ))
            .layer(axum::middleware::from_fn_with_state(
                auth,
                crate::require_bearer,
            ));
        let request = |method: &str, url: &str, token: Option<&str>, body: &str| {
            let mut request = axum::http::Request::builder()
                .method(method)
                .uri(url)
                .header("content-type", "application/json");
            if let Some(token) = token {
                request = request.header("authorization", format!("Bearer {token}"));
            }
            request
                .body(axum::body::Body::from(body.to_owned()))
                .unwrap()
        };
        let url = format!("/finops/reservations/{}/reconcile", reservation.as_str());
        assert_eq!(
            app.clone()
                .oneshot(request("GET", "/finops/reservations", None, ""))
                .await
                .unwrap()
                .status(),
            StatusCode::UNAUTHORIZED
        );
        for method in ["GET", "POST"] {
            let path = if method == "GET" {
                "/finops/reservations"
            } else {
                &url
            };
            assert_eq!(
                app.clone()
                    .oneshot(request(method, path, Some(participant_token), "{}"))
                    .await
                    .unwrap()
                    .status(),
                StatusCode::FORBIDDEN
            );
        }
        assert_eq!(
            app.clone()
                .oneshot(request(
                    "GET",
                    "/finops/reservations",
                    Some("budget-owner-fixture"),
                    ""
                ))
                .await
                .unwrap()
                .status(),
            StatusCode::OK
        );
        assert_eq!(
            app.clone()
                .oneshot(request(
                    "POST",
                    &url,
                    Some("budget-owner-fixture"),
                    r#"{"actual_usd":0.75,"verified_with_provider":false}"#
                ))
                .await
                .unwrap()
                .status(),
            StatusCode::BAD_REQUEST
        );
        for _ in 0..2 {
            assert_eq!(
                app.clone()
                    .oneshot(request(
                        "POST",
                        &url,
                        Some("budget-owner-fixture"),
                        r#"{"actual_usd":0.75,"verified_with_provider":true}"#
                    ))
                    .await
                    .unwrap()
                    .status(),
                StatusCode::NO_CONTENT
            );
        }
        let ledger = vak_core::finops::FinOpsLedger::new(&home);
        assert!(ledger.pending_reservations().unwrap().is_empty());
        let rows = ledger.all_rows();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].usd, Some(0.75));
        assert_eq!(rows[0].source, "owner_verified");
        assert!(rows[0].actor.is_some());
        assert_eq!(
            app.clone()
                .oneshot(request(
                    "POST",
                    &url,
                    Some("budget-owner-fixture"),
                    r#"{"actual_usd":0.50,"verified_with_provider":true}"#
                ))
                .await
                .unwrap()
                .status(),
            StatusCode::CONFLICT
        );
        let mut legacy = rows[0].clone();
        legacy.reservation_id = None;
        legacy.usd = None;
        legacy.source = "estimated".into();
        ledger.append(&legacy).unwrap();
        let response = app
            .clone()
            .oneshot(request(
                "GET",
                "/finops/reservations",
                Some("budget-owner-fixture"),
                "",
            ))
            .await
            .unwrap();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let view: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            view["unaddressable_unknown_calls"], 1,
            "old calls without a reservation cannot disappear from the uncertainty report"
        );
        let trace = vak_session::trace::TraceKey::root(
            vak_session::ids::TenantId::new(),
            vak_session::ids::SpaceId::new(),
            vak_session::ids::AgentId::new(),
            vak_session::trace::Cause::Heartbeat,
        );
        core.runs().open(&trace, None, None, 0).unwrap();
        let live = gate
            .reserve_dispatch(&vak_agent::SpendCheck {
                trace: Some(&trace),
                ..check
            })
            .await
            .unwrap()
            .unwrap();
        let live_url = format!("/finops/reservations/{}/reconcile", live.as_str());
        assert_eq!(
            app.oneshot(request(
                "POST",
                &live_url,
                Some("budget-owner-fixture"),
                r#"{"actual_usd":0.50,"verified_with_provider":true}"#
            ))
            .await
            .unwrap()
            .status(),
            StatusCode::CONFLICT
        );
        assert_eq!(
            ledger.pending_reservations().unwrap().len(),
            1,
            "a live run's reservation stays reserved"
        );
        core.runs()
            .settle(trace.run, vak_session::runs::RunOutcome::Cancelled, None)
            .unwrap();
    }
}
