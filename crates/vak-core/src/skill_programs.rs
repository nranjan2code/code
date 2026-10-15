//! A runnable skill's verification (docs/design/93-self-evolving-
//! capabilities.md §4, stage E2): the runtime runs the program's own tests
//! through the path every call takes (`vak_tools::capability`, sandboxed
//! by its declaration), in a run of its own, and keeps the result per
//! program digest. A test's verdict is what the runtime observed, never
//! what a model said of it (invariant 33).

use std::path::Path;

use serde::{Deserialize, Serialize};
use vak_tools::Tool as _;
use vak_tools::capability::{CapabilityTool, Program, Writes};

use crate::Core;

/// One test case as the runtime ran it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CaseResult {
    pub name: String,
    pub passed: bool,
    /// Where the answer differed, or how the call failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// A program's tests as one run found them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Verification {
    pub skill: String,
    /// The program verified: a digest over every file of the skill.
    pub digest: String,
    pub run: vak_session::ids::RunId,
    pub at: chrono::DateTime<chrono::Utc>,
    pub cases: Vec<CaseResult>,
}

impl Verification {
    pub fn passed(&self) -> bool {
        self.cases.iter().all(|case| case.passed)
    }
}

impl Core {
    /// Runs the tests of the skill `name` in `dir` and records what they
    /// found under the program's digest.
    pub async fn verify_skill_program(
        &self,
        name: &str,
        dir: &Path,
    ) -> Result<Verification, String> {
        let program =
            Program::load(dir)?.ok_or_else(|| format!("'{name}' has no program to verify"))?;
        let trace = self.mint_trace(None);
        let mut run = self
            .runs()
            .begin(&trace, None, None)
            .map_err(|error| format!("the verification run did not open: {error}"))?;
        let tool = CapabilityTool::new(
            name.to_string(),
            String::new(),
            program.clone(),
            self.tool_worker_exe(),
        );
        let agent = self
            .agent_identity()
            .map_or_else(|| "vak".to_string(), |agent| agent.id.clone());
        let ctx = vak_tools::ToolContext::new(self.cwd().to_path_buf())
            .with_agent_id(agent)
            .with_trace(trace.child());
        let limit_ms = self.time_limits().command_secs.saturating_mul(1000);
        let cases = tracing::Instrument::instrument(
            async {
                let mut cases = Vec::new();
                for (case_name, case) in &program.tests {
                    let mut args = case.arguments.clone();
                    if let Some(object) = args.as_object_mut() {
                        object.insert("timeout_ms".into(), limit_ms.into());
                    }
                    let output = tool.execute(&args, &ctx).await;
                    let judged = case.judge(&output, program.writes == Writes::Drafts);
                    cases.push(CaseResult {
                        name: case_name.clone(),
                        passed: judged.is_ok(),
                        detail: judged.err(),
                    });
                }
                cases
            },
            vak_session::runs::span(&trace),
        )
        .await;
        let verification = Verification {
            skill: name.to_string(),
            digest: program.digest.clone(),
            run: run.id(),
            at: chrono::Utc::now(),
            cases,
        };
        let failed = verification
            .cases
            .iter()
            .filter(|case| !case.passed)
            .count();
        run.settle_with(
            if failed == 0 {
                vak_session::runs::RunOutcome::Completed
            } else {
                vak_session::runs::RunOutcome::Failed {
                    reason: format!("{failed} of {} tests failed", verification.cases.len()),
                }
            },
            None,
        );
        let text = serde_json::to_string(&verification)
            .map_err(|error| format!("the verification did not encode: {error}"))?;
        vak_session::documents::create(&self.verification_path(&program.digest), &text)?;
        Ok(verification)
    }

    /// The latest verification recorded for the program with `digest`.
    pub fn skill_program_verification(&self, digest: &str) -> Option<Verification> {
        vak_session::documents::read(&self.verification_path(digest))
            .ok()
            .flatten()
            .and_then(|text| serde_json::from_str(&text).ok())
    }

    fn verification_path(&self, digest: &str) -> std::path::PathBuf {
        self.scope().capability_checks(self.cwd()).join(digest)
    }
}
