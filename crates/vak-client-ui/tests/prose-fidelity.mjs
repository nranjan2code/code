import assert from "node:assert/strict";
import { assistantParts, cleanAssistantText, stripControlScaffolding, spokenReplyText } from "../src/structured.ts";

const literal = `Outcome: the patient completed treatment.
contract_id: example
vak is the CLI command.
Surface: web client
primary deliverable: completed
This text explains [recovery] and [limit] markers.
<intent>quoted user input</intent>
\`\`\`json
{"semantic_type":"chart","payload":{}}
\`\`\`
completed`;
assert.equal(cleanAssistantText(literal), literal);
assert.equal(stripControlScaffolding(literal), literal);
assert.deepEqual(assistantParts(literal), [{ type: "text", text: literal }]);
assert.ok(spokenReplyText("Quote <conversation>literal transcript example</conversation>").includes("literal transcript example"));
const parts = assistantParts('Before.\n```vak\n{"semantic_type":"summary","payload":{"text":"Result"}}\n```\nAfter.');
assert.deepEqual(parts.map(part => part.type), ["text", "card", "text"]);
assert.equal(parts[0].text, "Before.");
assert.equal(parts[2].text, "After.");
console.log("authored prose fidelity ok");
