//! The control plane's decisions (docs/design/88-control-plane.md).
//!
//! The model proposes a plan, a call and an answer; the runtime decides at a
//! closed set of decision points, from recorded facts, and every decision is
//! one of the typed values here. Nothing in this crate reads text, holds a
//! session, or calls a provider.

use serde::{Deserialize, Serialize};

/// Who made a decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Decider {
    Runtime,
    Person,
    Provider,
}

/// A recorded fact a decision was made from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
pub enum Evidence {
    /// A ledger entry, by its id.
    Entry(String),
    /// A tool call, by its `tool_use_id`.
    Call(String),
    /// A recorded presentation, by its id.
    Presentation(String),
}

/// The closed set of decision points (doc 88 §3). Adding one is a code
/// change, on purpose.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionPoint {
    AdmitTurn,
    FrameStep,
    AcceptCall,
    AcceptAnswer,
    Correct,
    EndTurn,
}

/// One decision: who made it, from what, and what it chose. The point is
/// the choice's own tag, so a record cannot name one point and carry
/// another's output.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Decision {
    pub decider: Decider,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<Evidence>,
    pub choice: Choice,
}

impl Decision {
    pub fn by_runtime(choice: Choice) -> Self {
        Self {
            decider: Decider::Runtime,
            evidence: Vec::new(),
            choice,
        }
    }

    pub fn with(mut self, evidence: Evidence) -> Self {
        self.evidence.push(evidence);
        self
    }

    pub fn point(&self) -> DecisionPoint {
        self.choice.point()
    }
}

/// What a decision point chose.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "point", rename_all = "snake_case")]
pub enum Choice {
    AdmitTurn(Admission),
    FrameStep(Framing),
    AcceptCall(CallVerdict),
    AcceptAnswer(AnswerVerdict),
    Correct(Correction),
    EndTurn(Ending),
}

impl Choice {
    pub fn point(&self) -> DecisionPoint {
        match self {
            Self::AdmitTurn(_) => DecisionPoint::AdmitTurn,
            Self::FrameStep(_) => DecisionPoint::FrameStep,
            Self::AcceptCall(_) => DecisionPoint::AcceptCall,
            Self::AcceptAnswer(_) => DecisionPoint::AcceptAnswer,
            Self::Correct(_) => DecisionPoint::Correct,
            Self::EndTurn(_) => DecisionPoint::EndTurn,
        }
    }
}

/// The turn started, under this Agent, on this model, bounded by the
/// operator's step limit. The route each step was carried on is its
/// receipt's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Admission {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    pub model: String,
    pub max_steps: u32,
}

/// What a step is for. A fact the runtime has, never a reading of the
/// request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Purpose {
    Plan,
    Act,
    Answer,
    Repair,
    Classify,
    Compact,
    Judge,
    Summarise,
}

/// How much the model reasons on a step.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", content = "depth", rename_all = "snake_case")]
pub enum Reasoning {
    /// The model's own default; nothing asked.
    #[default]
    Default,
    Off,
    Depth(Depth),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Depth {
    Low,
    Medium,
    High,
    XHigh,
    Max,
}

/// Sampling settings; `None` leaves each to the model.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct Sampling {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_k: Option<u32>,
}

/// The settings a request carries for one step: how much the model
/// reasons and how it samples.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct StepSettings {
    pub reasoning: Reasoning,
    #[serde(default)]
    pub sampling: Sampling,
}

/// Whether a model takes one setting.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Support {
    Yes,
    No,
    /// Nobody reported it and no request has refused it yet: sent, and
    /// learned from the provider's answer.
    #[default]
    Unknown,
}

impl Support {
    fn sends(self) -> bool {
        self != Self::No
    }

    /// The narrower of two reports: a `No` from either side wins.
    pub fn meet(self, other: Self) -> Self {
        match (self, other) {
            (Self::No, _) | (_, Self::No) => Self::No,
            (Self::Yes, _) | (_, Self::Yes) => Self::Yes,
            _ => Self::Unknown,
        }
    }
}

/// How a model's reasoning can be set.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReasoningSupport {
    #[default]
    Unknown,
    /// The model does not reason, or the wire cannot set it.
    Fixed,
    /// Reasoning can be turned on or off, not graded.
    Toggle,
    /// Reasoning takes a depth.
    Graded,
}

/// What a model on a wire can honour (doc 88 §4): what the adapter can pass
/// through, narrowed by what the provider reported and what a request it
/// refused showed. Nothing here names a model or a provider.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelCapabilities {
    pub reasoning: ReasoningSupport,
    pub temperature: Support,
    pub top_p: Support,
    pub top_k: Support,
}

impl ModelCapabilities {
    /// The settings this model can honour of those asked: what it cannot
    /// take is left to the model's default, never approximated.
    pub fn honour(&self, asked: StepSettings) -> StepSettings {
        let reasoning = match (self.reasoning, asked.reasoning) {
            (ReasoningSupport::Fixed, _) => Reasoning::Default,
            (ReasoningSupport::Toggle, Reasoning::Depth(_)) => Reasoning::Default,
            (_, reasoning) => reasoning,
        };
        let keep = |support: Support, value| if support.sends() { value } else { None };
        StepSettings {
            reasoning,
            sampling: Sampling {
                temperature: keep(self.temperature, asked.sampling.temperature),
                top_p: keep(self.top_p, asked.sampling.top_p),
                top_k: if self.top_k.sends() {
                    asked.sampling.top_k
                } else {
                    None
                },
            },
        }
    }

    /// These capabilities with every setting `sent` carried marked as not
    /// taken: what a request refused for its settings, and accepted
    /// without them, showed.
    pub fn refusing(mut self, sent: StepSettings) -> Self {
        if sent.reasoning != Reasoning::Default {
            self.reasoning = ReasoningSupport::Fixed;
        }
        if sent.sampling.temperature.is_some() {
            self.temperature = Support::No;
        }
        if sent.sampling.top_p.is_some() {
            self.top_p = Support::No;
        }
        if sent.sampling.top_k.is_some() {
            self.top_k = Support::No;
        }
        self
    }

    /// The narrower of two reports, setting by setting.
    pub fn meet(self, other: Self) -> Self {
        use ReasoningSupport::*;
        let reasoning = match (self.reasoning, other.reasoning) {
            (Fixed, _) | (_, Fixed) => Fixed,
            (Toggle, _) | (_, Toggle) => Toggle,
            (Graded, _) | (_, Graded) => Graded,
            _ => Unknown,
        };
        Self {
            reasoning,
            temperature: self.temperature.meet(other.temperature),
            top_p: self.top_p.meet(other.top_p),
            top_k: self.top_k.meet(other.top_k),
        }
    }
}

/// What an adapter sent for one dispatch, and what it knew of the model
/// when it did.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Applied {
    pub settings: StepSettings,
    pub capabilities: ModelCapabilities,
}

/// The form the step's output must take.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "form", rename_all = "snake_case")]
pub enum OutputForm {
    Free,
    /// The step must call this one tool.
    Tool {
        name: String,
    },
    /// The step must produce output matching the schema with this digest.
    Schema {
        digest: String,
    },
}

/// One step's frame (doc 88 §4).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StepContract {
    pub purpose: Purpose,
    pub reasoning: Reasoning,
    #[serde(default)]
    pub sampling: Sampling,
    /// The tools the request offers, by name, in request order.
    pub tools: Vec<String>,
    pub output: OutputForm,
}

/// The contract asked for a step. What each dispatch of it applied is on
/// the step's receipt, attempt by attempt.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Framing {
    pub step: u32,
    pub asked: StepContract,
}

/// Whether a proposed call runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "verdict", content = "by", rename_all = "snake_case")]
pub enum CallVerdict {
    Accepted,
    Refused(Refusal),
}

/// What refused a call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Refusal {
    /// The arguments do not match the tool's schema.
    Schema,
    /// The tool itself refused these arguments.
    Tool,
    /// The capability was revoked during the turn.
    Revoked,
    /// The same call was repeated in this run.
    Repeated,
    /// A permission rule or mode denied it.
    Policy,
    /// The person asked declined it.
    Person,
    /// No approver could be asked.
    Unattended,
}

/// Whether a proposed answer ends the turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "verdict", content = "by", rename_all = "snake_case")]
pub enum AnswerVerdict {
    Accepted,
    Held(Cause),
}

/// What a correction answers: the check whose verdict failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Cause {
    StopHook,
    StopGuard,
    Grounding,
    Fence,
    DuplicateCard,
    Presentation,
    CardCompletion,
    CardCompleteness,
    Freshness,
    EmptyStep,
    SteeringDrift,
    TopicMismatch,
    Repair,
    Plan,
    ToolCallRepair,
}

/// The rung of the correction ladder (doc 88 §5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Rung {
    /// Tell once, with the evidence.
    Tell,
    /// Change the next step's frame.
    Reframe,
    /// End with what exists.
    End,
}

/// What the model is told, and what changes in its frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Correction {
    pub cause: Cause,
    pub rung: Rung,
}

/// How a turn ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ending {
    pub end: End,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum End {
    Answered,
    Aborted,
    Failed,
    StepLimit,
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn a_record_names_its_point_by_its_choice() {
        let decision =
            Decision::by_runtime(Choice::AcceptCall(CallVerdict::Refused(Refusal::Schema)))
                .with(Evidence::Call("toolu_1".into()));
        let json = serde_json::to_value(&decision).unwrap();
        assert_eq!(json["choice"]["point"], "accept_call");
        assert_eq!(json["choice"]["by"], "schema");
        assert_eq!(json["evidence"][0]["kind"], "call");
        let back: Decision = serde_json::from_value(json).unwrap();
        assert_eq!(back, decision);
        assert_eq!(back.point(), DecisionPoint::AcceptCall);
    }

    #[test]
    fn every_point_serialises_inside_its_tag() {
        for choice in [
            Choice::AcceptAnswer(AnswerVerdict::Accepted),
            Choice::AcceptAnswer(AnswerVerdict::Held(Cause::Plan)),
            Choice::Correct(Correction {
                cause: Cause::EmptyStep,
                rung: Rung::Tell,
            }),
            Choice::EndTurn(Ending {
                end: End::StepLimit,
            }),
            Choice::AdmitTurn(Admission {
                agent: None,
                model: "m".into(),
                max_steps: 8,
            }),
        ] {
            let decision = Decision::by_runtime(choice);
            let text = serde_json::to_string(&decision).unwrap();
            assert_eq!(serde_json::from_str::<Decision>(&text).unwrap(), decision);
        }
    }

    #[test]
    fn a_model_is_asked_only_what_it_can_honour() {
        let asked = StepSettings {
            reasoning: Reasoning::Depth(Depth::Low),
            sampling: Sampling {
                temperature: Some(0.2),
                top_p: Some(0.9),
                top_k: Some(20),
            },
        };
        let toggle = ModelCapabilities {
            reasoning: ReasoningSupport::Toggle,
            top_k: Support::No,
            ..Default::default()
        };
        let applied = toggle.honour(asked);
        assert_eq!(applied.reasoning, Reasoning::Default);
        assert_eq!(applied.sampling.temperature, Some(0.2));
        assert_eq!(applied.sampling.top_k, None);
        let off = StepSettings {
            reasoning: Reasoning::Off,
            ..Default::default()
        };
        assert_eq!(toggle.honour(off).reasoning, Reasoning::Off);
        assert_eq!(ModelCapabilities::default().honour(asked), asked);
    }

    #[test]
    fn a_refused_request_narrows_only_what_it_sent() {
        let sent = StepSettings {
            reasoning: Reasoning::Depth(Depth::High),
            sampling: Sampling::default(),
        };
        let caps = ModelCapabilities::default().refusing(sent);
        assert_eq!(caps.reasoning, ReasoningSupport::Fixed);
        assert_eq!(caps.temperature, Support::Unknown);
        let reported = ModelCapabilities {
            reasoning: ReasoningSupport::Graded,
            temperature: Support::Yes,
            ..Default::default()
        };
        assert_eq!(reported.meet(caps).reasoning, ReasoningSupport::Fixed);
        assert_eq!(reported.meet(caps).temperature, Support::Yes);
    }

    #[test]
    fn a_framing_round_trips() {
        let decision = Decision::by_runtime(Choice::FrameStep(Framing {
            step: 2,
            asked: StepContract {
                purpose: Purpose::Repair,
                reasoning: Reasoning::Depth(Depth::Low),
                sampling: Sampling::default(),
                tools: vec!["read".into()],
                output: OutputForm::Free,
            },
        }));
        let text = serde_json::to_string(&decision).unwrap();
        assert_eq!(serde_json::from_str::<Decision>(&text).unwrap(), decision);
    }
}
