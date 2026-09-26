import assert from "node:assert/strict";
import { CHARACTER_TIERS, characterTier } from "../src/characterTier.ts";

// The generator writes these sizes (scripts/brand/generate.mjs).
assert.deepEqual([...CHARACTER_TIERS], [64, 128, 256]);
// A mark loads the smallest copy that covers it at the screen's pixel ratio.
assert.equal(characterTier(24), 64);
assert.equal(characterTier(28 * 2), 64);
assert.equal(characterTier(64), 64);
assert.equal(characterTier(60 * 2), 128);
assert.equal(characterTier(104 * 2), 256);
// Past the largest copy it stays at the largest rather than failing.
assert.equal(characterTier(104 * 3), 256);
console.log("character-tier: ok");
