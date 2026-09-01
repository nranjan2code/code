# VAK architecture video storyboard

Target: 90 seconds, 1920×1080, 30 FPS, narrated technical explainer.

| Time | Scene | Visual beat | Narration focus |
|---|---|---|---|
| 00:00–00:10 | The question | Channels converge into one VAK request | Governed, recoverable AI work |
| 00:10–00:20 | Secure entry | Normalize identity, allowlist, policy, route | Trust before execution |
| 00:20–00:30 | Core | `vak-core` assembles the runtime contract | Workspace-scoped coordinator |
| 00:30–00:42 | Agent loop | Project, stream, append, stop gate | Model proposes; system governs |
| 00:42–00:54 | Tools | Permission → broker → worker → sandbox | Every effect crosses the boundary |
| 00:54–01:06 | Normal vs managed | Direct loop splits from WorkContract path | Simple fast; complex durable |
| 01:06–01:17 | Ledger | JSONL grows; projections fan out | Reconstructable history |
| 01:17–01:27 | Delivery + ops | Semantic result becomes surface output; ops observes | Safe rendering and evidence |
| 01:27–01:30 | Close | Full system resolves into one line | Many entry points, one runtime |

## Production rules

- Keep the active idea in the center of the frame and move peripheral labels only when the narration introduces them.
- Never show the model as directly holding a shell, filesystem, channel token, or policy handle.
- When a tool call appears, visibly pass it through the shield, broker, worker, and sandbox.
- When Managed Flow appears, show verification before the completion state.
- When the ledger appears, animate append-only growth rather than editing or deleting an existing line.
- Keep captions in a lower safe area and never cover the primary architecture labels.
