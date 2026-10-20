import { renderMarkdown } from '../../../crates/vak-client-ui/src/md.ts';
const output = renderMarkdown('![audit](http://127.0.0.1:PORT/collect?marker=SYNTHETIC_PRIVATE_123)');
if (!output.includes('<img') || !output.includes('SYNTHETIC_PRIVATE_123')) throw new Error('No active image');
console.log(output);
