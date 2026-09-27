import assert from 'node:assert/strict';
import { api } from '../src/api.ts';
let request;
globalThis.fetch = async (path, init) => {
  request = { path: String(path), method: init.method, body: JSON.parse(init.body) };
  return new Response(JSON.stringify({ error: 'route was rejected' }), { status: 409, headers: { 'Content-Type': 'application/json' } });
};
await assert.rejects(api.patchConfigScope('user', { provider: 'test', model: 'test-model' }), /route was rejected/);
assert.equal(request.path, '/config/global');
assert.deepEqual(request.body, { provider: 'test', model: 'test-model' });
await assert.rejects(api.setProviderKey('test', 'fixture-only-never-sent', 'user'), /route was rejected/);
assert.equal(request.path, '/config/key');
assert.equal(request.body.key, 'fixture-only-never-sent');
console.log('admin-api-errors: route and key failures reach the caller');
