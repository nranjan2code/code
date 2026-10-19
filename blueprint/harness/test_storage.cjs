// Deterministic decisions and independent arithmetic, not runtime storage tests.
const assert = require('node:assert/strict');
const {defaults:d, forecast:f} = require('./storage.js');
const KiB=1024, MiB=1024**2;
const empty=f({...d,turns:0});
assert.equal(empty.last.primary,0);assert.equal(empty.boundedTokens,0);
const ledgerOnly={...d,months:1,turns:1,ledger:1,objects:0,cards:0,aux:0,artifactRate:0,temporary:0,logs:0,index:0,copies:0};
assert.equal(f(ledgerOnly).last.primary,30*KiB);
assert.equal(f({...ledgerOnly,months:2}).last.primary,60*KiB);
const versions=f({...ledgerOnly,artifactRate:100,artifactMiB:2,versions:3});
assert.equal(versions.last.artifacts,30*2*3*MiB);
const temp=f({...ledgerOnly,temporary:1,tempDays:7});
assert.equal(temp.last.temp,7*MiB);assert.equal(temp.last.noCleanup-temp.last.primary,23*MiB);
assert.equal(f({...ledgerOnly,temporary:1,tempDays:7,protected:100}).last.temp,30*MiB);
assert.equal(f({...ledgerOnly,temporary:1,tempDays:0}).last.temp,0);
assert.equal(f({...ledgerOnly,copies:2}).last.fleet,90*KiB);
assert.equal(f({...ledgerOnly,capacity:0}).crossing,1);
// Sum real turn positions as an independent oracle for capped token traffic.
for (const session of [1,7,100]) {
  const p={...ledgerOnly,prefix:3,history:2,context:10,steps:4,session};
  const expected=Array.from({length:30},(_,i)=>Math.min(10,3+(i%session)*2)).reduce((a,b)=>a+b,0)*4;
  assert.equal(f(p).boundedTokens,expected);
}
assert.ok(f({...d,session:1000000}).fullTokens>f({...d,session:100}).fullTokens);
assert.equal(f({...d,context:1000}).last.primary,f(d).last.primary);
assert.equal(f({...d,months:120,growth:0}).monthly.length,120);
assert.throws(()=>f({...d,scanMiBs:0}));assert.throws(()=>f({...d,turns:NaN}));
for (const turns of [0,1,50,5000]) for (const protectedBytes of [0,50,100]) {
  const r=f({...d,turns,protected:protectedBytes});
  assert.ok(r.last.primary<=r.last.noCleanup+1e-5);
  assert.ok(r.boundedTokens<=r.fullTokens);
  assert.ok(Object.values(r.last).every(Number.isFinite));
}
console.log('Storage scenarios: arithmetic, retention guards, versions, copies, context and horizon checks pass.');
for (const turns of [50,500,5000]) {
 const r=f({...d,turns});console.log(JSON.stringify({turns,annualTurns:r.last.turns,primaryGiB:r.last.primary/1024**3,allGiB:r.last.fleet/1024**3,artifactsGiB:r.last.artifacts/1024**3}));
}
