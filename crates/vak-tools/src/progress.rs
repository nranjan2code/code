//! The agent loop owns progress; this tool only declares its strict contract.
use crate::{Fault, Tool, ToolContext, ToolOutput};
use async_trait::async_trait;
use serde_json::{Value, json};

pub struct ProgressTool;

pub fn parse_progress_args(args: &Value) -> Result<vak_intent::progress::ProgressList, String> {
    crate::validate_input(&ProgressTool.schema(), args).map_err(|error| error.to_string())?;
    let progress: vak_intent::progress::ProgressList =
        serde_json::from_value(args.clone()).map_err(|error| error.to_string())?;
    progress.validate()?;
    Ok(progress)
}

#[async_trait]
impl Tool for ProgressTool {
    fn name(&self) -> &str {
        "progress"
    }
    fn description(&self) -> &str {
        "Optionally show progress for work with several steps. Send the whole list each time, updating states as work finishes; give a reason for anything dropped. This list does not constrain tools, cards or the answer. Skip it for a direct answer."
    }
    fn schema(&self) -> Value {
        json!({"type":"object","additionalProperties":false,"properties":{
            "items":{"type":"array","description":"The whole current list, replacing the previous list for this turn.","items":{
                "type":"object","additionalProperties":false,"properties":{
                    "label":{"type":"string","description":"The step to do, in words the person recognises."},
                    "state":{"type":"string","enum":["pending","in_progress","done","dropped"],"description":"Whether this step is waiting, underway, finished or intentionally left out."},
                    "reason":{"type":["string","null"],"description":"A short explanation when the step is dropped; otherwise null."}
                },"required":["label","state","reason"]}
            }
        },"required":["items"]})
    }
    fn looks_up(&self) -> bool {
        false
    }
    fn always_loaded(&self) -> bool {
        true
    }
    fn permission(&self) -> vak_permission::Declared {
        vak_permission::Declared::reach(vak_permission::Reach::Coordination)
    }
    async fn execute(&self, _args: &Value, _ctx: &ToolContext) -> ToolOutput {
        ToolOutput::error(
            Fault::Tool,
            "Progress must be answered by the agent loop, not dispatched to a worker.",
        )
    }
}
