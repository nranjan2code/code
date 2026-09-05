# 50 — Call and evidence contract

**Status: implemented in 3.0.3 for schema admission; semantic evidence
contracts are the next additive extension.**

vak has one call boundary regardless of whether a turn is temporal, coding,
research, data, or ordinary conversation. A provider proposal is not an
execution request until the host has validated it against the admitted tool
schema, authorized it, and admitted its resource claims.

## Call lifecycle

```text
provider proposal
  → schema contract
  → permission / revocation
  → resource claims and budget
  → brokered dispatch
  → typed result and evidence
  → semantic outcome
  → presentation projection
```

Schema validation is deliberately provider-neutral. It validates the
provider-independent subset of the admitted JSON Schema: root and nested
types, required properties, enums, arrays, and item types. A malformed call is
an error value and never reaches a tool. It is not a network failure and must
not enter provider retry or route endurance.

The schema boundary does not pretend to understand domain meaning. Temporal
calls still need an explicit time zone and freshness contract; research needs
source and claim evidence; coding needs workspace and verification evidence;
data comparisons need aligned periods, units, and aggregation basis. Those
are result contracts, not tool-name branches.

## Evidence and rendering

Plain Markdown is always retained as a lossless fallback document. It is not
evidence that a claim is true. A typed semantic result may render as a native
card only after its registered payload schema passes. Future domain payloads
must carry their comparison period, basis, and evidence references when those
fields are material to the claim. If those semantics are absent, the output
must remain an ordinary document with an explicit diagnostic rather than being
labelled verified.

This keeps the renderer honest: a report can be beautifully formatted while
the runtime still distinguishes observed facts, cited facts, model synthesis,
and unsupported claims.

## Non-goals

- Do not infer a domain from incidental field names.
- Do not repair malformed provider calls by inventing missing arguments.
- Do not calculate a weekly, temporal, or comparative metric from whichever
  two values happen to be present.
- Do not replace the original tool result with a rendering-oriented rewrite.
