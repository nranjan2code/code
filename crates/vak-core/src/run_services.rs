//! Host-owned run services. Parent and child runs use the same presentation
//! and history constructors; session/workspace callbacks bind only after the
//! admitted run's own context exists.

use crate::{
    CheckpointDelta, Core, artifacts, checkpoints, elsewhere, file_state, presentation_tools,
};
use std::sync::Arc;

type HistoryRefresh = Arc<dyn Fn() + Send + Sync>;

impl Core {
    pub(crate) fn presentation_rebuilder(&self) -> vak_agent::PresentationRebuild {
        let skills = vak_delivery::built_in_skill_registry();
        let packs = self.pack_kinds();
        Arc::new(move |name, input| {
            presentation_tools::presentation_info_with(name, input, &skills, &packs).map(|info| {
                vak_tools::PresentationCard {
                    presentation_id: None,
                    semantic_type: info.semantic_type,
                    skill_id: info.skill_id,
                    skill_version: info.skill_version,
                    schema_version: info.schema_version,
                    payload: info.payload,
                    title: info.title,
                    identity_digest: info.identity_digest,
                    block: info.block,
                    data_call_id: info.data_call_id,
                }
            })
        })
    }

    pub(crate) fn history_services(
        &self,
        session_id: String,
        reach: Option<Arc<elsewhere::Elsewhere>>,
    ) -> (vak_agent::HistoryRecall, HistoryRefresh) {
        let core = self.clone();
        let sid = session_id.clone();
        let recall: vak_agent::HistoryRecall = Arc::new(move |request, leaf, cancel| {
            let core = core.clone();
            let session = sid.clone();
            let reach = reach.clone();
            Box::pin(async move {
                match tokio::task::spawn_blocking(move || {
                    core.resolve_indexed_recall(&session, &leaf, request, reach.as_deref(), cancel)
                })
                .await
                {
                    Ok(output) => output,
                    Err(_) => Err(serde_json::json!({"type":"history_unavailable",
                        "message":"history lookup could not finish"})
                    .to_string()),
                }
            })
        });
        let core = self.clone();
        let refresh: HistoryRefresh = Arc::new(move || core.queue_history_index(&session_id));
        (recall, refresh)
    }

    pub(crate) fn child_services_factory(
        &self,
        reach: Option<Arc<elsewhere::Elsewhere>>,
    ) -> vak_agent::ChildServicesFactory {
        let child_core = self.clone();
        Arc::new(move |context| {
            let core = child_core
                .clone()
                .with_agent_identity(context.agent_identity.clone());
            let file_core = core.clone();
            let artifacts = core.artifacts();
            let file_artifacts = artifacts.clone();
            let file_space = vak_session::trace::local::space(&context.cwd).to_string();
            let cwd = context.cwd.clone();
            let sid = context.session_id.clone();
            let agent = context.agent_id.clone();
            let file_note: vak_agent::FileNote = Arc::new(move |query| match query {
                vak_agent::FileQuery::Path(path) => {
                    let state = file_state::try_observe(&file_artifacts, &file_space, &cwd, path)
                        .map_err(|_| "artifact_history_unavailable".to_string())?;
                    let Some(state) = state else { return Ok(None) };
                    let made = state.workspace_made.as_ref().and_then(|made| {
                        let card = file_core.turn_card(&made.session, &made.turn)?;
                        let elsewhere = sid != made.session;
                        Some(card.made_in(elsewhere.then_some(made.session.as_str())))
                    });
                    Ok(file_state::note_with_turn(
                        &state,
                        file_state::Reader {
                            agent: &agent,
                            session: Some(&sid),
                        },
                        made.as_ref(),
                    ))
                }
                vak_agent::FileQuery::Pattern(pattern) => file_state::try_matching_note(
                    &file_artifacts,
                    &file_space,
                    &cwd,
                    pattern,
                    Some(&sid),
                )
                .map_err(|_| "artifact_history_unavailable".to_string()),
            });
            let objects = core
                .objects()
                .map_err(|_| "child_objects_unavailable".to_string())?;
            let home = core.scope().into_root();
            let seq = core.next_checkpoint_seq(&context.session_id);
            let delta: Arc<dyn vak_agent::WorkspaceDelta> = Arc::new(CheckpointDelta {
                objects: objects.clone(),
                home: home.clone(),
                sid: context.session_id.clone(),
                seq,
                cwd: context.cwd.clone(),
            });
            if seq == 0 {
                let (checkpoint, _) = checkpoints::capture(
                    &context.cwd,
                    &core.scope(),
                    objects.as_ref(),
                    &context.session_id,
                    seq,
                    "child run start",
                )
                .map_err(|_| "child_checkpoint_unavailable".to_string())?;
                checkpoints::store(&core.scope(), objects.as_ref(), &checkpoint)
                    .map_err(|_| "child_checkpoint_unavailable".to_string())?;
            }
            let checkpoint_core = core.clone();
            let checkpoint_sid = context.session_id.clone();
            let checkpoint_cwd = context.cwd.clone();
            let before_effect = Arc::new(move || {
                let effect_seq = checkpoint_core.next_checkpoint_seq(&checkpoint_sid);
                if let Ok(objects) = checkpoint_core.objects()
                    && let Ok((checkpoint, _)) = checkpoints::capture(
                        &checkpoint_cwd,
                        &checkpoint_core.scope(),
                        objects.as_ref(),
                        &checkpoint_sid,
                        effect_seq,
                        "child workspace change",
                    )
                {
                    let _ =
                        checkpoints::store(&checkpoint_core.scope(), objects.as_ref(), &checkpoint);
                }
            });
            let (history_recall, history_refresh) =
                core.history_services(context.session_id.clone(), reach.clone());
            let presentation_rebuild = core.presentation_rebuilder();
            Ok(vak_agent::ChildBoundServices {
                artifacts: Some(Arc::new(artifacts::CallSink {
                    artifacts,
                    cwd: context.cwd.clone(),
                    executions: vak_config::scope::executions_root(&context.cwd),
                    agent: context.agent_id,
                })),
                file_note: Some(file_note),
                before_effect: Some(before_effect),
                presentation_rebuild: Some(presentation_rebuild),
                history_recall: Some(history_recall),
                history_refresh: Some(history_refresh),
                workspace_delta: Some(delta),
            })
        })
    }
}
