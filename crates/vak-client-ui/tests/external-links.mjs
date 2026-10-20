import assert from 'node:assert/strict';
import { externalLink } from '../src/externalLinks.ts';
for (const value of ['https://example.com/a?q=x#b', 'http://example.com/', 'mailto:person@example.com']) {
  assert.ok(externalLink(value), value);
}
for (const value of ['javascript:alert(1)', 'data:text/html,x', 'file:///tmp/x', 'vak://x', '//example.com', '/app/', 'https://name:secret@example.com', 'https://example.com/\n', '--help']) {
  assert.equal(externalLink(value), null, value);
}
console.log('external-links: ok');
