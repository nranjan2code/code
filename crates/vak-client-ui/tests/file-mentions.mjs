import assert from 'node:assert/strict';
import { detectMention, fileMention } from '../src/fileMentions.ts';

for (const path of ['notes.txt', 'src/main.rs', 'space name.txt', 'नोट.txt', 'a,b.txt', 'say "hi".txt', 'back\\slash.txt', 'trailing.']) {
  const token = fileMention(path);
  assert.equal(token.startsWith('@"') ? JSON.parse(token.slice(1)) : token.slice(1), path);
}
assert.equal(fileMention('space name.txt'), '@"space name.txt"');
for (const [text, query] of [['read @no', 'no'], ['(@नोट', 'नोट'], ['read @"space na', 'space na'], ['@"say \\"hi', 'say "hi']]) {
  assert.equal(detectMention(text, text.length)?.query, query);
}
for (const text of ['mail@notes.txt', '@"complete.txt"', '@"bad\\q']) assert.equal(detectMention(text, text.length), null);
console.log('file mentions: ok');
