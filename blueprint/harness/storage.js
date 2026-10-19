/* One pure forecast used by both the browser and deterministic scenario checks.
   Bytes are incremental physical bytes after existing object reuse, not wire bytes.
   No discount for future compression, dedup, erasure or cold tiering. */
(function () {
  'use strict';
  const MiB = 1024 ** 2, GiB = 1024 ** 3;
  const defaults = {turns: 50, months: 12, growth: 0, ledger: 24, objects: 100,
    cards: 12, artifactRate: 10, artifactMiB: 2, versions: 3, aux: 8,
    temporary: 1, tempDays: 7, logs: 8, logDays: 14, protected: 0,
    index: 25, copies: 2, capacity: 100, reserve: 20, steps: 4,
    prefix: 6000, history: 1500, context: 32000, session: 100,
    scanMiBs: 50, decodeRows: 20000, rows: 20, rebuildMiBs: 20,
    restoreMiBs: 50, recoveryMinutes: 60};
  function forecast(p) {
    for (const key of Object.keys(defaults)) {
      if (!Number.isFinite(p[key]) || p[key] < 0) throw Error('Invalid input: ' + key);
    }
    if (p.months < 1 || p.months > 120 || p.session < 1 || p.steps < 1 ||
        p.scanMiBs <= 0 || p.decodeRows <= 0 || p.rebuildMiBs <= 0 || p.restoreMiBs <= 0 ||
        p.protected > 100 || p.artifactRate > 100 || p.reserve > 100 || p.growth > 100)
      throw Error('Input outside supported planning range');
    const days = Math.round(p.months * 30), monthly = [];
    let turns = 0, everTemp = 0, everLogs = 0, temp = 0, logs = 0;
    let boundedTokens = 0, fullTokens = 0, crossing = null;
    const tempCohorts = [], logCohorts = [];
    const protectedShare = p.protected / 100;
    function live(cohorts, day, keep) {
      return cohorts.reduce((n, c) => n + c.bytes * (day - c.day < keep ? 1 : protectedShare), 0);
    }
    function promptSum(n, cap) {
      // Each session starts with no history; each turn adds history tokens.
      const whole = Math.floor(n / p.session), rest = n % p.session;
      function sessionSum(k) {
        if (!p.history) return k * p.prefix;
        const room = Math.max(0, cap - p.prefix);
        const before = Math.min(k, Math.floor(room / p.history) + 1);
        return before * p.prefix + p.history * before * (before - 1) / 2 + (k - before) * Math.max(p.prefix, cap);
      }
      return (whole * sessionSum(p.session) + sessionSum(rest)) * p.steps;
    }
    for (let day = 0; day < days; day++) {
      const today = Math.round(p.turns * (1 + p.growth / 100) ** Math.floor(day / 30));
      turns += today;
      const t = today * p.temporary * MiB, l = today * p.logs * 1024;
      everTemp += t; everLogs += l;
      tempCohorts.push({day, bytes: t}); logCohorts.push({day, bytes: l});
      temp = live(tempCohorts, day, p.tempDays); logs = live(logCohorts, day, p.logDays);
      const ledger = turns * p.ledger * 1024;
      const objects = turns * p.objects * 1024;
      const cards = turns * p.cards * 1024;
      const artifacts = turns * p.artifactRate / 100 * p.versions * p.artifactMiB * MiB;
      const aux = turns * p.aux * 1024;
      const durable = ledger + objects + cards + artifacts + aux;
      const index = (ledger + cards + aux) * p.index / 100;
      const primary = durable + temp + logs + index;
      // Copies model full backup-equivalent payload, excluding derived index and scratch.
      const backup = (durable + logs) * p.copies;
      const noCleanup = durable + everTemp + everLogs + index;
      if (crossing === null && primary > p.capacity * GiB * (1 - p.reserve / 100)) crossing = day + 1;
      if ((day + 1) % 30 === 0 || day === days - 1) monthly.push({month: (day + 1) / 30, turns,
        ledger, objects, cards, artifacts, aux, temp, logs, index, durable, primary, backup,
        fleet: primary + backup, noCleanup});
    }
    boundedTokens = promptSum(turns, p.context);
    fullTokens = promptSum(turns, Number.MAX_VALUE);
    const last = monthly[monthly.length - 1];
    const largestSessionTurns = Math.min(turns, p.session);
    const sessionBytes = largestSessionTurns * (p.ledger + p.objects + p.cards) * 1024;
    return {monthly, last, crossing, boundedTokens, fullTokens,
      largestSessionTurns, sessionBytes,
      coldSeconds: sessionBytes / (p.scanMiBs * MiB) + largestSessionTurns * p.rows / p.decodeRows,
      rebuildSeconds: (last.ledger + last.cards + last.aux) / (p.rebuildMiBs * MiB),
      restoreSeconds: (last.durable + last.logs) / (p.restoreMiBs * MiB)};
  }
  if (typeof module !== 'undefined') module.exports = {defaults, forecast};
  if (typeof document === 'undefined') return;
  const root = document.getElementById('storage-growth');
  if (!root) return;
  const facts = JSON.parse(document.getElementById('storage-data').textContent);
  defaults.tempDays = facts.default_days.Environment;
  defaults.logDays = facts.default_days.Telemetry;
  const controls = [
    ['Work over time', [['turns','Turns per day',0,1000000],['months','Months',1,120],['growth','Monthly growth (%)',0,100],['session','Turns per conversation',1,1000000]]],
    ['New bytes per turn · assumptions', [['ledger','Ledger metadata (KiB)',0,100000],['objects','Text and tool objects (KiB)',0,100000],['cards','Cards, arguments and state (KiB)',0,100000],['aux','Runs, decisions and other records (KiB)',0,100000],['artifactRate','Turns producing an artifact (%)',0,100],['artifactMiB','Mean artifact version (MiB)',0,100000],['versions','Versions per artifact',0,1000]]],
    ['Temporary data and copies · assumptions', [['temporary','Scratch and environments per turn (MiB)',0,100000],['tempDays','Temporary retention (days)',0,3650],['logs','Telemetry per turn (KiB)',0,100000],['logDays','Telemetry retention (days)',0,3650],['protected','Temporary bytes blocked from expiry (%)',0,100],['index','Index overhead on metadata (%)',0,1000],['copies','Full backup-equivalent copies',0,100]]],
    ['Capacity and recovery · assumptions', [['capacity','Primary disk allocation (GiB)',0,1000000],['reserve','Keep disk free (%)',0,100],['scanMiBs','Cold read throughput (MiB/s)',0.1,100000],['decodeRows','Decode throughput (rows/s)',1,10000000],['rows','Ledger entries per turn',0,100000],['rebuildMiBs','Projection rebuild throughput (MiB/s)',0.1,100000],['restoreMiBs','Restore throughput (MiB/s)',0.1,100000],['recoveryMinutes','Recovery target (minutes)',0,100000]]],
    ['Model requests · assumptions', [['steps','Requests per turn',1,1000],['prefix','Prefix and schemas (tokens/request)',0,10000000],['history','New history tokens per turn',0,10000000],['context','Request token budget',0,10000000]]]
  ];
  const form = document.getElementById('storage-controls');
  controls.forEach(([title, fields], groupIndex) => {
    const set = document.createElement('fieldset'), legend = document.createElement('legend'); legend.textContent = title; set.append(legend);
    fields.forEach(([key,label,min,max]) => {
      const lab = document.createElement('label'); lab.textContent = label;
      const input = document.createElement('input'); input.type='number'; input.min=min; input.max=max;
      input.step='any'; input.value=defaults[key]; input.name=key; input.required=true;
      lab.append(input); set.append(lab);
    });
    if (groupIndex === 0) form.append(set);
    else {
      const details = document.createElement('details'), summary = document.createElement('summary');
      summary.textContent = title; details.append(summary, set); form.append(details);
    }
  });
  const gb = n => (n / GiB).toLocaleString(undefined,{maximumFractionDigits:2}) + ' GiB';
  const num = n => n.toLocaleString(undefined,{maximumFractionDigits:0});
  const duration = n => n < 60 ? n.toFixed(1)+' s' : (n/60).toFixed(1)+' min';
  let result, params;
  function update() {
    if (!form.reportValidity()) return;
    params = Object.fromEntries([...form.elements].filter(x => x.name).map(x => [x.name, Number(x.value)]));
    try { result = forecast(params); } catch (e) { document.getElementById('storage-summary').textContent=e.message; return; }
    const r=result, x=r.last;
    document.getElementById('storage-summary').textContent = `${num(x.turns)} turns · ${gb(x.primary)} primary · ${gb(x.backup)} backup equivalents · ${gb(x.fleet)} total across locations. Primary reserve ${r.crossing ? 'crossed on day '+r.crossing : 'not crossed in this horizon'}.`;
    const tbody=document.getElementById('storage-months'); tbody.replaceChildren();
    r.monthly.forEach(m=>{
      const tr=document.createElement('tr');
      [m.month,num(m.turns),gb(m.primary),gb(m.noCleanup),gb(m.backup)].forEach(v=>{const td=document.createElement('td');td.textContent=v;tr.append(td);}); tbody.append(tr);
    });
    const breakdown=document.getElementById('storage-breakdown'); breakdown.replaceChildren();
    [['Ledger metadata',x.ledger],['Text / tool objects',x.objects],['Cards / state',x.cards],['Artifact versions',x.artifacts],['Runs / decisions / other',x.aux],['Temporary data',x.temp],['Telemetry',x.logs],['Derived indexes',x.index]].forEach(([name,bytes])=>{
      const row=document.createElement('div');row.className='storage-bar';
      const label=document.createElement('span');label.textContent=name;
      const bar=document.createElement('meter');bar.min=0;bar.max=Math.max(1,x.primary);bar.value=bytes;bar.setAttribute('aria-label',name);
      const value=document.createElement('span');value.textContent=gb(bytes);row.append(label,bar,value);breakdown.append(row);
    });
    const max=Math.max(1,...r.monthly.map(m=>m.noCleanup));
    const path=key=>r.monthly.map((m,i)=>`${i?'L':'M'}${50+(i/(Math.max(1,r.monthly.length-1)))*700},${200-m[key]/max*170}`).join(' ');
    document.getElementById('storage-curve-kept').setAttribute('d',path('primary'));
    document.getElementById('storage-curve-all').setAttribute('d',path('noCleanup'));
    document.getElementById('storage-chart-title').textContent=`Primary growth over ${params.months} months. Vertical maximum ${gb(max)}; retained ${gb(x.primary)}, without temporary cleanup ${gb(x.noCleanup)}. Monthly values follow in the table.`;
    document.getElementById('storage-chart-max').textContent='Chart maximum: '+gb(max);
    const notes=[
      `Conversation reopen estimate: ${duration(r.coldSeconds)} for ${num(r.largestSessionTurns)} turns (${gb(r.sessionBytes)}); sequential I/O plus entry decode only. Not a benchmark or UI latency prediction.`,
      `Projection source scan: ${duration(r.rebuildSeconds)}; restore copy: ${duration(r.restoreSeconds)}. Separate simplified stages; verification, key recovery, object hydration and index construction add work.`,
      `Input traffic estimate: ${num(r.boundedTokens)} tokens with a working-set budget; ${num(r.fullTokens)} with full history within each conversation. Both include ${params.steps} requests per turn and a repeated prefix; intra-turn tool growth, output, reasoning and cache billing excluded.`,
      `Context compaction does not remove ledger history. Saved versions and permanent records still grow. ${params.prefix>params.context ? 'The prefix exceeds the request budget: this context scenario is infeasible.' : 'The working-set line assumes the context planner can fit each request.'}`,
      `The cleanup line assumes timely eligibility and collection for temporary data only. Segment boundaries, active work, holds, shared references and trash may delay or prevent release. Telemetry’s shipped size cap is not applied; the estimate is conservative.`,
      `Decide first: ${x.artifacts > x.primary/2 ? 'artifact versions dominate: measure unique version bytes and confirm which drafts are saved before changing retention.' : 'measure physical bytes by class and the largest conversations before choosing a storage change.'}`,
      `${r.crossing ? 'Capacity decision needed within this horizon: add headroom or preview eligible temporary retention changes.' : 'Capacity fits this scenario; permanent growth still needs monitoring beyond the horizon.'} No runtime policy is changed here.`,
      `${r.restoreSeconds+r.rebuildSeconds>params.recoveryMinutes*60 ? 'Recovery estimate exceeds the target even before verification: benchmark a real restore.' : 'Recovery lower-bound estimate fits the target; a verified restore drill must establish actual recovery time.'}`
    ];
    const list=document.getElementById('storage-decisions');list.replaceChildren(); notes.forEach(n=>{const li=document.createElement('li');li.textContent=n;list.append(li);});
  }
  form.addEventListener('input',update); form.addEventListener('change',update); form.addEventListener('submit',e=>e.preventDefault());
  document.getElementById('storage-reset').addEventListener('click',()=>{Object.entries(defaults).forEach(([k,v])=>form.elements.namedItem(k).value=v);update();});
  document.getElementById('storage-export').addEventListener('click',()=>{
    update(); if (!form.checkValidity()) return;
    const blob=new Blob([JSON.stringify({scope:facts.scope,assumptions:params,code:facts,result},null,2)],{type:'application/json'});
    const a=document.createElement('a');a.href=URL.createObjectURL(blob);a.download='vakyartha-storage-scenario.json';a.click();setTimeout(()=>URL.revokeObjectURL(a.href),1000);
  });
  const evidence=document.getElementById('storage-facts');facts.facts.forEach(f=>{const li=document.createElement('li'),a=document.createElement('a');a.href='../'+f.file;a.textContent=f.file+':'+f.line;li.append(f.meaning+' ',a);evidence.append(li);});
  update();
})();
