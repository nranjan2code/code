import type { ClientEvent } from "../src/types";

// Generated from fixtures/client-events.json; check it with
// check-client-event-fixture.mjs after changing the Rust wire fixture.
export const wireExamples = [
  "StreamOpened",
  {
    "TurnStart": {
      "turn": 1
    }
  },
  {
    "TextDelta": {
      "delta": "text"
    }
  },
  {
    "ThinkingDelta": {
      "delta": "thinking"
    }
  },
  {
    "ToolCallStart": {
      "id": "call",
      "name": "read",
      "args_json": "{}"
    }
  },
  {
    "CardDrafting": {
      "id": "call",
      "block": "table"
    }
  },
  {
    "ToolCallEnd": {
      "id": "call",
      "name": "read",
      "is_error": false,
      "result_preview": null
    }
  },
  {
    "ApprovalRequested": {
      "id": "approval",
      "tool": "bash",
      "args_json": "{}",
      "reason": "review"
    }
  },
  {
    "WorkerStarted": {
      "label": "worker"
    }
  },
  {
    "WorkerToolCall": {
      "label": "worker",
      "name": "read",
      "is_error": false
    }
  },
  {
    "WorkerFinished": {
      "label": "worker",
      "is_error": false,
      "elapsed_ms": 1
    }
  },
  {
    "WorkerQuestion": {
      "id": "question",
      "label": "worker",
      "question": "Choose",
      "options": [
        "yes"
      ]
    }
  },
  {
    "WorkerQuestionAnswered": {
      "id": "question"
    }
  },
  {
    "WorkerQuestionClosed": {
      "id": "question"
    }
  },
  "Retrying",
  {
    "Sandbox": {
      "kind": "Stdout",
      "execution_id": "exec",
      "chunk": "output"
    }
  },
  {
    "DraftDiscarded": {
      "turn": 1
    }
  },
  {
    "RunFinished": {
      "outcome": "MaxTurns",
      "message": "Stopped at the step limit."
    }
  }
] as const satisfies readonly ClientEvent[];

type Variant<T> = T extends string ? T : keyof T;
export const variantNames = {
  "StreamOpened": true,
  "TurnStart": true,
  "TextDelta": true,
  "ThinkingDelta": true,
  "ToolCallStart": true,
  "CardDrafting": true,
  "ToolCallEnd": true,
  "ApprovalRequested": true,
  "WorkerStarted": true,
  "WorkerToolCall": true,
  "WorkerFinished": true,
  "WorkerQuestion": true,
  "WorkerQuestionAnswered": true,
  "WorkerQuestionClosed": true,
  "Retrying": true,
  "Sandbox": true,
  "DraftDiscarded": true,
  "RunFinished": true
} as const satisfies Record<Variant<ClientEvent>, true>;
