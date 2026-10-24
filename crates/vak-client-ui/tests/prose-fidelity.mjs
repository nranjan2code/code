import assert from "node:assert/strict";
import { cleanAssistantText, stripControlScaffolding, spokenReplyText } from "../src/structured.ts";

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
assert.ok(spokenReplyText("Quote <conversation>literal transcript example</conversation>").includes("literal transcript example"));
const example = 'Before.\n```vak\n{"semantic_type":"summary","payload":{"text":"Example"}}\n```\nAfter.';
assert.equal(cleanAssistantText(example), example);
console.log("authored prose fidelity ok");
