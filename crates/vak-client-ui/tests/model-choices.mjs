import assert from 'node:assert/strict';
import { availableModels, initialModel } from '../src/modelChoices.ts';
assert.equal(initialModel(['b', 'a'], 'a'), 'a', 'keep the chosen model regardless of list order');
assert.equal(initialModel(['b', 'a'], 'missing'), '', 'multiple models need an explicit choice');
assert.equal(initialModel(['only'], ''), 'only');
assert.deepEqual(availableModels({ provider: 'bedrock', models: ['unknown'] }), ['unknown'], 'provider catalogue is used without a vendor control-plane check');
assert.deepEqual(availableModels({ provider: 'bedrock', models: ['ok', 'no'] }), ['ok', 'no']);
assert.deepEqual(availableModels({ provider: 'service', models: ['b', 'a'] }), ['b', 'a']);
console.log('model-choices: 6 checks passed');
