/* Shared behaviour for every page of the public site.
 *
 * Motion (motion.dev, vendored at src/vendor/motion.js) is loaded `defer`,
 * so it has executed by the time DOMContentLoaded fires — but it is never
 * required. Everything here has a path for `window.Motion` being absent:
 * the CSS transitions in styles.css are the floor, and a reader with no
 * JavaScript at all gets the finished page immediately, because `.reveal`
 * only hides anything once the inline head script has added `html.js`.
 *
 * The order that matters: nothing in this file may leave content invisible
 * if a later step throws. Reveal work therefore runs first and defensively.
 */
(function () {
  "use strict";

  var $ = function (id) { return document.getElementById(id); };
  var $$ = function (sel, root) {
    return Array.prototype.slice.call((root || document).querySelectorAll(sel));
  };
  var reduceQuery = window.matchMedia("(prefers-reduced-motion: reduce)");
  var EASE = [0.16, 1, 0.3, 1];

  function ready(fn) {
    if (document.readyState === "loading") {
      document.addEventListener("DOMContentLoaded", fn, { once: true });
    } else {
      fn();
    }
  }

  // --- build stamp ---------------------------------------------------------
  // Fetched rather than templated in, so these stay static files with no
  // server-side rendering. If it fails, the readout simply never appears.
  function buildStamp() {
    fetch("/version", { credentials: "same-origin" })
      .then(function (r) { return r.ok ? r.json() : null; })
      .then(function (v) {
        if (!v || !v.version) return;
        var sha = v.git_sha && v.git_sha !== "unknown" ? v.git_sha : null;
        var stamp = $("stamp");
        if (stamp) stamp.textContent = "vak " + v.version + (sha ? " · " + sha : "");
        if ($("bv")) {
          $("bv").textContent = v.version;
          $("bc").textContent = sha || "—";
          $("buildcard").hidden = false;
        }
      })
      .catch(function () {});
  }

  // --- theme ---------------------------------------------------------------
  // Three states, the same as the app: system (no attribute), light, dark.
  function theme() {
    var btn = $("theme");
    if (!btn) return;
    var order = ["system", "light", "dark"];
    var stored = "system";
    try { stored = localStorage.getItem("vak-theme") || "system"; } catch (e) {}

    function apply(next) {
      if (next === "system") delete document.documentElement.dataset.theme;
      else document.documentElement.dataset.theme = next;
      try { localStorage.setItem("vak-theme", next); } catch (e) {}
      btn.setAttribute("aria-label", "Theme: " + next + ". Switch theme.");
      btn.title = "Theme: " + next;
    }
    apply(order.indexOf(stored) >= 0 ? stored : "system");
    btn.addEventListener("click", function () {
      var now = document.documentElement.dataset.theme || "system";
      apply(order[(order.indexOf(now) + 1) % order.length]);
    });
  }

  // --- rail ----------------------------------------------------------------
  function rail(M, reduce) {
    var el = $("rail");
    if (!el) return;
    var onScroll = function () { el.classList.toggle("scrolled", window.scrollY > 8); };
    onScroll();
    window.addEventListener("scroll", onScroll, { passive: true });

    // The progress rule is scroll-linked, not time-based, so it is honest
    // about position rather than animating on its own. Strictly decoration:
    // with no Motion it simply stays at zero and nothing else changes.
    var bar = $("railprogress");
    if (!bar || !M || !M.scroll || reduce) return;
    try {
      M.scroll(M.animate(bar, { transform: ["scaleX(0)", "scaleX(1)"] }, { ease: "linear" }));
    } catch (e) {}
  }

  // --- reveals -------------------------------------------------------------
  // Two paths to the same end state. Whichever runs, an element that has
  // entered the viewport is visible; nothing here can leave one hidden.
  function reveals(M, reduce) {
    var items = $$(".reveal");
    if (!items.length) return;

    var showAll = function () { items.forEach(function (el) { el.classList.add("in"); }); };
    if (reduce || !("IntersectionObserver" in window)) { showAll(); return; }

    if (M && M.inView && M.animate) {
      items.forEach(function (el) {
        var delay = (parseFloat(el.style.getPropertyValue("--d")) || 0) / 1000;
        M.inView(
          el,
          function () {
            // `.in` first: if the animation throws for any reason, the CSS
            // transition has already been armed to finish the job.
            el.classList.add("in");
            try {
              M.animate(
                el,
                { opacity: [0, 1], transform: ["translateY(14px)", "translateY(0px)"] },
                { duration: 0.72, delay: delay, ease: EASE }
              );
            } catch (e) {}
          },
          { amount: 0.12, margin: "0px 0px -8% 0px" }
        );
      });
      return;
    }

    var io = new IntersectionObserver(function (entries, obs) {
      entries.forEach(function (e) {
        if (!e.isIntersecting) return;
        e.target.classList.add("in");
        obs.unobserve(e.target);
      });
    }, { rootMargin: "0px 0px -12% 0px", threshold: 0.08 });
    items.forEach(function (el) { io.observe(el); });
  }

  // --- the session console (home) -----------------------------------------
  // The page's one authored moment. Rows never leave the flow — they only
  // fade — so playing the tape cannot reflow anything below it.
  function console_(M, reduce) {
    var tape = $("tape");
    if (!tape) return;
    var steps = $$(".step", tape);
    var approved = $("approved");
    var denied = $("denied");
    var gateActions = $("gateactions");
    var gateVerdict = $("gateverdict");
    var runword = $("runword");
    var runstate = $("runstate");
    var footnote = $("footnote");
    var timers = [];
    var settled = false;

    function dot(kind, word) {
      runstate.firstElementChild.className = "dot " + kind;
      runword.textContent = word;
    }
    function clearTimers() { timers.forEach(clearTimeout); timers = []; }
    function hideOutcomes() {
      approved.classList.remove("shown");
      denied.classList.remove("shown");
    }
    function play(el) {
      el.classList.add("played");
      if (!M || !M.animate || reduce) return;
      try {
        M.animate(el, { opacity: [0, 1], transform: ["translateY(4px)", "translateY(0px)"] },
          { duration: 0.34, ease: EASE });
      } catch (e) {}
    }

    function resolve(choice) {
      if (settled) return;
      settled = true;
      hideOutcomes();
      var shown;
      if (choice === "allow") {
        gateVerdict.textContent = "allow";
        gateVerdict.className = "verdict allow";
        gateActions.innerHTML =
          '<span class="gate-resolved">approved by you · 14:02:21 · recorded in the ledger</span>';
        shown = approved;
        dot("done", "finished");
        footnote.textContent = "Approved. The command ran, and the approval is an entry like any other.";
      } else {
        gateVerdict.textContent = "deny";
        gateVerdict.className = "verdict deny";
        gateActions.innerHTML =
          '<span class="gate-resolved">denied by you · 14:02:31 · recorded in the ledger</span>';
        shown = denied;
        dot("stop", "stopped");
        footnote.textContent = "Denied. The run stops, the work it had already done is kept, and the denial is recorded.";
      }
      shown.classList.add("shown");
      if (M && M.animate && !reduce) {
        try {
          M.animate($$(".row", shown), { opacity: [0, 1], transform: ["translateY(6px)", "translateY(0px)"] },
            { duration: 0.4, delay: M.stagger ? M.stagger(0.07) : 0, ease: EASE });
        } catch (e) {}
      }
    }

    function bindGate() {
      var a = $("approve");
      var d = $("deny");
      if (a) a.addEventListener("click", function () { resolve("allow"); });
      if (d) d.addEventListener("click", function () { resolve("deny"); });
    }

    function start(instant) {
      clearTimers();
      settled = false;
      hideOutcomes();
      gateVerdict.textContent = "ask";
      gateVerdict.className = "verdict ask";
      gateActions.innerHTML =
        '<button class="btn primary" type="button" id="approve">Approve</button>' +
        '<button class="btn danger" type="button" id="deny">Deny</button>' +
        '<span class="gate-hint">Nothing has run yet.</span>';
      bindGate();
      footnote.textContent = "A real session, replayed. The gate is yours to answer.";

      if (instant) {
        steps.forEach(function (s) { s.classList.add("played"); });
        dot("wait", "awaiting you");
        return;
      }
      steps.forEach(function (s) { s.classList.remove("played"); });
      dot("run", "running");
      steps.forEach(function (s, i) {
        timers.push(setTimeout(function () {
          play(s);
          if (i === steps.length - 1) dot("wait", "awaiting you");
        }, 220 + i * 320));
      });
    }

    bindGate();
    if (reduce) {
      start(true);
    } else if (M && M.inView) {
      var stop = M.inView(tape, function () { if (stop) stop(); start(false); }, { amount: 0.25 });
    } else if ("IntersectionObserver" in window) {
      var once = new IntersectionObserver(function (entries, obs) {
        entries.forEach(function (e) {
          if (!e.isIntersecting) return;
          obs.disconnect();
          start(false);
        });
      }, { threshold: 0.25 });
      once.observe(tape);
    } else {
      start(true);
    }
    var replay = $("replay");
    if (replay) replay.addEventListener("click", function () { start(reduce); });
  }

  // --- permission modes (home) --------------------------------------------
  function modes() {
    var body = $("mode-body");
    if (!body) return;
    var A = '<span class="verb a">allow</span> ';
    var K = '<span class="verb k">ask</span> ';
    var D = '<span class="verb d">deny</span> ';
    var TAIL = '<hr /><span class="dim">your own rules are evaluated first, and deny still wins</span>';
    var copy = {
      ro: ["Read what is there, change nothing",
        "The agent may read the workspace and answer about it. Writes and shell commands are denied outright — not queued for approval, denied. Use it on a repository you have not decided to trust yet.",
        A + 'read(<span class="hit">&lt;workspace&gt;/**</span>)<br />' +
        D + 'read(<span class="hit">outside the workspace</span>)<br />' +
        K + 'anything that reaches the network<br />' +
        D + 'write · bash · everything else' + TAIL],
      ww: ["Change this workspace, ask before anything else",
        "The default. Reads and writes inside the canonical workspace path go through; a shell command, or a path outside the boundary, stops and asks — the gate you answered above. A symlink pointing out of the workspace does not count as inside it.",
        A + 'read · write(<span class="hit">&lt;workspace&gt;/**</span>)<br />' +
        K + 'write(<span class="hit">outside the workspace</span>)<br />' +
        K + 'bash(<span class="hit">*</span>)<br />' +
        K + 'everything else, by default' + TAIL],
      fa: ["Everything, and you said so out loud",
        "No implicit boundary — which is why it cannot be reached by accident. It is named explicitly, and your own rules still sit on top of it, so this is the mode’s default and not the last word.",
        A + 'every tool, on every path<br />' +
        D + '<span class="hit">your deny rules</span>, which still win<br />' +
        K + '<span class="hit">your ask rules</span>, which still stop the run<br />' +
        '<span class="dim">and a policy change still cancels work in flight</span>' + TAIL]
    };
    var tabs = $$(".modes button");
    tabs.forEach(function (t, i) {
      t.addEventListener("click", function () {
        tabs.forEach(function (o) { o.setAttribute("aria-selected", String(o === t)); });
        var c = copy[t.dataset.mode];
        $("mode-title").textContent = c[0];
        $("mode-text").textContent = c[1];
        $("mode-rules").innerHTML = c[2];
      });
      t.addEventListener("keydown", function (e) {
        var d = e.key === "ArrowRight" ? 1 : e.key === "ArrowLeft" ? -1 : 0;
        if (!d) return;
        e.preventDefault();
        var next = tabs[(i + d + tabs.length) % tabs.length];
        next.focus();
        next.click();
      });
    });
  }

  // --- surface switcher (surfaces page) ------------------------------------
  function switcher(M, reduce) {
    $$(".switch").forEach(function (root) {
      var tabs = $$(".switch-tabs button", root);
      var panels = $$(".switch-panel", root);
      tabs.forEach(function (t, i) {
        t.addEventListener("click", function () {
          tabs.forEach(function (o) { o.setAttribute("aria-selected", String(o === t)); });
          panels.forEach(function (p) { p.hidden = p.dataset.panel !== t.dataset.panel; });
          var shown = panels.filter(function (p) { return !p.hidden; })[0];
          if (!shown || !M || !M.animate || reduce) return;
          try {
            M.animate(shown, { opacity: [0, 1], transform: ["translateY(8px)", "translateY(0px)"] },
              { duration: 0.4, ease: EASE });
          } catch (e) {}
        });
        t.addEventListener("keydown", function (e) {
          var d = e.key === "ArrowRight" ? 1 : e.key === "ArrowLeft" ? -1 : 0;
          if (!d) return;
          e.preventDefault();
          var next = tabs[(i + d + tabs.length) % tabs.length];
          next.focus();
          next.click();
        });
      });
    });
  }

  // --- gate simulator (security page) --------------------------------------
  // Runs the same precedence the engine runs: an explicit deny rule, then an
  // explicit ask rule, then an explicit allow rule, then the mode's own
  // default. Getting that order wrong on a page about a policy engine would
  // be worse than not having the page.
  function simulator(M, reduce) {
    var root = $("sim");
    if (!root) return;
    var verdictBox = $("sim-verdict");
    var word = $("sim-word");
    var why = $("sim-why");
    var trace = $("sim-trace");

    // The illustrative ruleset shown alongside; keep in step with the markup.
    var RULES = [
      { decision: "deny", match: function (a) { return a.tool === "bash" && /rm\s+-rf/.test(a.arg); }, label: "deny bash(rm -rf *)" },
      { decision: "deny", match: function (a) { return a.tool === "write" && /\.env$/.test(a.arg); }, label: "deny write(**/.env)" }
    ];

    var MODE_DEFAULT = {
      ro: function (a) {
        if (a.tool === "read") {
          return a.outside
            ? ["deny", "read-only allows reads, but only inside the workspace."]
            : ["allow", "Reading inside the workspace is what this mode is for."];
        }
        if (a.tool === "fetch") return ["ask", "It reaches outside the workspace, so it stops and asks."];
        return ["deny", "read-only denies " + a.tool + " outright. Switch modes or add a rule."];
      },
      ww: function (a) {
        if (a.tool === "read") {
          return a.outside
            ? ["deny", "Reads are confined to the workspace here too; the path is outside it."]
            : ["allow", "Reading inside the workspace needs no approval."];
        }
        if (a.tool === "write") {
          return a.outside
            ? ["ask", "The path is outside the workspace boundary, so it needs you."]
            : ["allow", "Writing inside the canonical workspace path is what this mode grants."];
        }
        return ["ask", "A shell command or an outward call always needs approval in this mode."];
      },
      fa: function (a) {
        return ["allow", "full-access has no implicit boundary — which is why it has to be named explicitly."];
      }
    };

    var state = { mode: "ww", action: null };

    function evaluate() {
      var a = state.action;
      if (!a) return null;
      for (var i = 0; i < RULES.length; i++) {
        if (RULES[i].decision === "deny" && RULES[i].match(a)) {
          return { verdict: "deny", why: "An explicit deny rule matched, and deny always wins — before the mode is even consulted.", step: "rule-deny" };
        }
      }
      var d = MODE_DEFAULT[state.mode](a);
      return { verdict: d[0], why: d[1], step: "mode" };
    }

    var STEPS = [
      ["rule-deny", "deny rules"],
      ["rule-ask", "ask rules"],
      ["rule-allow", "allow rules"],
      ["mode", "the mode’s own default"]
    ];

    function render() {
      var r = evaluate();
      if (!r) {
        verdictBox.className = "sim-verdict";
        word.textContent = "—";
        why.textContent = "Pick an action to see what the engine would answer.";
        trace.innerHTML = "";
        return;
      }
      verdictBox.className = "sim-verdict " + r.verdict;
      word.textContent = r.verdict;
      why.textContent = r.why;
      trace.innerHTML = STEPS.map(function (s) {
        var reached = STEPS.findIndex(function (x) { return x[0] === r.step; }) >= STEPS.findIndex(function (x) { return x[0] === s[0]; });
        var hit = s[0] === r.step;
        return '<li class="' + (hit ? "hit" : "") + '"><span class="mark">' +
          (hit ? "→" : reached ? "·" : " ") + "</span><span>" + s[1] +
          (hit ? " — decided here" : reached ? " — no match" : " — not reached") + "</span></li>";
      }).join("");
      if (!M || !M.animate || reduce) return;
      try {
        M.animate(word, { opacity: [0, 1], transform: ["translateY(6px)", "translateY(0px)"] }, { duration: 0.3, ease: EASE });
        M.animate($$("li", trace), { opacity: [0, 1] }, { duration: 0.28, delay: M.stagger ? M.stagger(0.05) : 0 });
      } catch (e) {}
    }

    $$(".sim-mode button", root).forEach(function (b) {
      b.addEventListener("click", function () {
        state.mode = b.dataset.mode;
        $$(".sim-mode button", root).forEach(function (o) { o.setAttribute("aria-selected", String(o === b)); });
        render();
      });
    });
    $$(".sim-actions button", root).forEach(function (b) {
      b.addEventListener("click", function () {
        state.action = { tool: b.dataset.tool, arg: b.dataset.arg || "", outside: b.dataset.outside === "1" };
        $$(".sim-actions button", root).forEach(function (o) { o.setAttribute("aria-pressed", String(o === b)); });
        render();
      });
    });
    render();
  }

  // --- copy buttons ---------------------------------------------------------
  function copyButtons() {
    $$(".copy").forEach(function (b) {
      b.addEventListener("click", function () {
        var target = $(b.dataset.copy);
        if (!target) return;
        var text = target.textContent;
        var done = function () {
          var was = b.textContent;
          b.textContent = "Copied";
          setTimeout(function () { b.textContent = was; }, 1400);
        };
        if (navigator.clipboard && navigator.clipboard.writeText) {
          navigator.clipboard.writeText(text).then(done, function () {});
        }
      });
    });
  }

  function vakMark() {
    var mark = document.querySelector(".vak-glyph");
    var prompt = $("vak-prompt");
    if (!mark || !prompt || reduceQuery.matches) return;
    mark.addEventListener("pointerenter", function () { prompt.textContent = "you bring the intent"; });
    mark.addEventListener("pointerleave", function () { prompt.textContent = "intent · expression · action"; });
    var svg = mark.querySelector("svg");
    if (svg) svg.classList.add("alive");
    var words = $$(".signal-word");
    if (words.length) {
      var index = 0;
      setInterval(function () {
        words[index].classList.remove("active");
        index = (index + 1) % words.length;
        words[index].classList.add("active");
      }, 2400);
    }
  }

  // --- turn engine kinetic simulation (surfaces page) ----------------------
  function turnEngine(M, reduce) {
    var root = $("turn-engine");
    if (!root) return;

    var canvas = $("te-canvas");
    var ctx = canvas ? canvas.getContext("2d") : null;
    var stagesEl = $("te-stages");
    var inspector = $("te-inspector");
    var playBtn = $("te-play");
    var stepBtn = $("te-step");
    var speedBtn = $("te-speed");
    var resetBtn = $("te-reset");
    var statusEl = $("te-status");

    var hudSid = $("hud-sid");
    var hudTurns = $("hud-turns");
    var hudRoute = $("hud-route");
    var hudTokens = $("hud-tokens");
    var hudSpend = $("hud-spend");
    var hudEntries = $("hud-entries");
    var hudInvariants = $("hud-invariants");

    var STAGES = [
      {
        id: "surface",
        num: "01",
        title: "Surface Inbound",
        state: "normalized payload",
        source: "crates/vak/src/cli.rs · crates/vak-server/src/gateway.rs",
        desc: "Request arrives via CLI args, terminal PTY, chat webhook (Telegram/Slack/Discord), HTTP/SSE endpoint, or scheduled heartbeat. The gateway maps inbound remote identity into InboundRequest, enforcing the three-segment bot identity (surface:chat:bot_id).",
        rule: "Rule 24: One physical chat served by several bots gets one identity per bot. Gateway binds to canonical workspace ~/vak-home.",
        code: 'InboundRequest {\n    surface: Surface::Telegram,\n    chat_id: "chat_98241",\n    bot_id: Some("bot_sec_ops"),\n    sender: "alice",\n    text: "fix failing parser test",\n}'
      },
      {
        id: "admission",
        num: "02",
        title: "Admission & Policy",
        state: "allowlist & CorePool",
        source: "crates/vak-server/src/gateway.rs · allowlist.json",
        desc: "Allowlist resolves inbound authorization. An unknown chat lands as reviewable 'pending' (fail-closed, Rule 15). Workspace Core is borrowed from CorePool with security ceiling check. Capability overlay filters out disabled tools before turn admission.",
        rule: "Rule 15: Unattended surfaces fail closed. Empty allowlist rejects every chat unless explicitly opened. CorePool entry never escalates permissions.",
        code: 'AllowlistEntry {\n    status: AllowlistStatus::Allowed,\n    workspace: "/home/vak/project",\n    permission_mode: PermissionMode::WorkspaceWrite,\n    inherit_bot_policy: true,\n}'
      },
      {
        id: "ledger",
        num: "03",
        title: "Session Ledger",
        state: "append-only JSONL",
        source: "crates/vak-session/src/log.rs",
        desc: "Request is appended immediately to the session ledger as SessionEntry::UserMessage. Every model-visible token, tool result, or plan must be reconstructable via derive_messages(). The session log is strictly append-only; entries are never rewritten or deleted.",
        rule: "Invariant 1 & 2: Model-visible means logged. Append-only sessions: never rewrite or delete session entries. Branching = parent_id.",
        code: 'SessionEntry::Message(MessageRecord {\n    id: "msg_01K7Q4WCE2_01",\n    parent_id: Some("msg_01K7Q4WCE2_00"),\n    role: Role::User,\n    content: [ContentBlock::Text("fix failing parser test")],\n})'
      },
      {
        id: "intent",
        num: "04",
        title: "Intent & Commitments",
        state: "commitment kernel",
        source: "crates/vak-intent/src/lib.rs · crates/vak-core/src/commitments.rs",
        desc: "The commitment kernel classifies the reading into an explicit objective, horizon, and capability slice. Projections only narrow: uncertainty reduces authority, never widens it. A durable work episode is opened and stamped with a commitment ID.",
        rule: "Long-horizon contract: Intent can subtract authority; it never creates it. Reading horizon derives direct vs managed execution.",
        code: 'OutcomeSpec {\n    objective: "fix failing parser test",\n    horizon: Horizon::Direct,\n    required_domains: ["engineering"],\n    evidence_max_age_secs: Some(86400),\n}'
      },
      {
        id: "route",
        num: "05",
        title: "Route Ladder & FinOps",
        state: "per-turn ladder assembly",
        source: "crates/vak-core/src/lib.rs (plan_route_ladder) · finops.rs",
        desc: "Provider route ladder is assembled fresh every turn using live evidence ledger, warm discovery cache, and operator effective_route(). SpendGate verifies dispatch ceiling and token budget admission before contacting model provider.",
        rule: "Invariant 7: Route ladder is planned fresh on every turn. Invariant 8: Secrets live in canonical .env, never git. Invariant 9: Model catalogues discovered, never hardcoded.",
        code: 'RouteLadder {\n    primary: RouteLeg { provider: "anthropic", model: "claude-3-7-sonnet" },\n    fallbacks: [RouteLeg { provider: "openai", model: "gpt-4o" }],\n    budget_remaining_usd: 4.979,\n}'
      },
      {
        id: "context",
        num: "06",
        title: "Context & Compaction",
        state: "token budget gate",
        source: "crates/vak-agent/src/context.rs",
        desc: "Token estimation checks the prompt projection against model window. If over budget: proactive retrieval fetches relevant prior turns verbatim, older turns are summarized via compaction LLM request, and a CompactionEntry is appended to the ledger.",
        rule: "Invariant 4: Compaction is an append-only entry, never deletion. If still over budget, reset-with-handoff rescue activates.",
        code: 'CompactionPlan {\n    tokens_before: 182400,\n    keep_recent: 8,\n    proactive_retrieved: 2,\n    summary_tokens: 340,\n}'
      },
      {
        id: "llm",
        num: "07",
        title: "LLM Streaming",
        state: "delta + snapshot stream",
        source: "crates/vak-llm/src/lib.rs · crates/vak-agent/src/lib.rs",
        desc: "Provider client streams response chunks with watchdog timer. Every streaming AgentEvent carries delta AND snapshot. Transient faults retry with backoff under circuit breaker; user abort preserves partial output.",
        rule: "Invariant 4: Every streaming event carries delta AND snapshot. Invariant 5: Abort preserves partial output.",
        code: 'AgentEvent::Delta {\n    delta: "Running cargo test...",\n    snapshot: "I will first run cargo test...",\n    usage: Usage { prompt_tokens: 6118, completion_tokens: 412 },\n}'
      },
      {
        id: "broker",
        num: "08",
        title: "Broker Boundary",
        state: "__tool_worker process",
        source: "crates/vak-tools/src/lib.rs · crates/vak-sandbox",
        desc: "Assistant tool calls cross a broker boundary. Built-in tools (bash, read, edit) run through the versioned __tool_worker protocol in a disposable process group. MCP tools run as separately sandboxed worker subprocesses with strict env sanitization.",
        rule: "Invariant 14: Model tools cross a broker boundary. Workers never hold policy engine, provider credentials, or session store handles.",
        code: 'PendingToolCall {\n    id: "call_tool_01",\n    name: "bash",\n    input: { "command": "cargo test -p vak-parser" },\n    worker: ToolWorker::DisposableGroup,\n}'
      },
      {
        id: "permission",
        num: "09",
        title: "Permission Engine",
        state: "precedence: Deny -> Ask -> Allow",
        source: "crates/vak-permission/src/engine.rs",
        desc: "Evaluates the tool call before dispatch: 1. Explicit deny rules (deny wins outright!). 2. Explicit ask rules. 3. Explicit allow rules. 4. Mode default (read-only, workspace-write, full-access). If Ask, pauses for human approval or fails closed on timeout.",
        rule: "Invariant 13: FullAccess is an explicit human trust decision, never auto-widened. Invariant 15: Unattended approvals fail closed.",
        code: 'Decision::Ask {\n    reason: "shell command requires approval in workspace-write mode",\n    source: AskSource::ModeDefault,\n}'
      },
      {
        id: "receipt",
        num: "10",
        title: "Receipt & Delivery",
        state: "settled audit record",
        source: "crates/vak-agent/src/spend.rs · crates/vak-delivery/src/lib.rs",
        desc: "Execution output is appended as ToolResult. WorkReceipt records exact tokens, cost, latency, provider, and model leg. FinOps spend gate settles ledger. DeliveryHub adapts outcome for the surface (terminal ANSI, diff review, or chat chunks).",
        rule: "Invariant 3: Errors are values (is_error). WorkReceipt is the authoritative per-turn dispatch record for audit.",
        code: 'WorkReceipt {\n    provider: "anthropic",\n    model: "claude-3-7-sonnet",\n    usage: { input: 6118, output: 412 },\n    latency_ms: 1240,\n    settled_usd: 0.021,\n}'
      }
    ];

    // Build stage buttons in DOM
    stagesEl.innerHTML = STAGES.map(function (s, i) {
      return '<button role="tab" type="button" class="pipeline-node' + (i === 0 ? " active" : "") + '" data-idx="' + i + '">' +
        '<span class="node-num">Stage ' + s.num + '</span>' +
        '<span class="node-title">' + s.title + '</span>' +
        '<span class="node-state">' + s.state + '</span>' +
      '</button>';
    }).join("");

    var nodes = $$(".pipeline-node", stagesEl);

    var state = {
      scale: "single", // single, multi, swarm
      surface: "cli",
      playing: true,
      speed: 1,
      stepIndex: 0,
      activeStage: 0,
      multiTurn: 1,
      swarmPackets: [],
      particles: [],
      fault: null,
      tokensIn: 6118,
      tokensOut: 412,
      entries: 14,
      spend: 0.021,
      turnsCompleted: 1,
      totalTurns: 1
    };

    function updateInspector(idx) {
      state.activeStage = idx;
      nodes.forEach(function (n, i) {
        n.classList.toggle("active", i === idx);
      });
      var s = STAGES[idx];
      if (!s) return;
      $("insp-badge").textContent = "Stage " + s.num + " · " + s.title;
      $("insp-source").textContent = s.source;
      $("insp-desc").textContent = s.desc;
      $("insp-rule").innerHTML = "<b>Rule / Invariant:</b> " + s.rule;
      var code = s.code;
      if (state.fault === "rate_limit" && idx === 4) {
        code = '// Fault injected: 429 RateLimit on primary leg!\nRouteLadder {\n    primary: RouteLeg { provider: "anthropic" (FAILING: 429) },\n    failover_active: RouteLeg { provider: "openai", model: "gpt-4o" },\n    endurance_retry: 1,\n}';
      } else if (state.fault === "deny_rule" && idx === 8) {
        code = '// Fault injected: Prohibited action detected!\nDecision::Deny {\n    reason: "explicit deny rule matched: bash(rm -rf *)",\n    source: AskSource::Rule,\n}';
      } else if (state.fault === "ask_gate" && idx === 8) {
        code = '// Action stops at gate: Awaiting human verdict\nDecision::Ask {\n    reason: "out-of-bounds path (~/.ssh/config)",\n    source: AskSource::Boundary,\n}';
      }
      $("insp-code").textContent = code;
    }

    nodes.forEach(function (btn, i) {
      btn.addEventListener("click", function () {
        updateInspector(i);
      });
    });

    // Toolbar selectors
    $$(".btn-tab", root).forEach(function (btn) {
      btn.addEventListener("click", function () {
        $$(".btn-tab", root).forEach(function (b) { b.setAttribute("aria-selected", "false"); });
        btn.setAttribute("aria-selected", "true");
        state.scale = btn.dataset.scale;
        resetSimulation();
      });
    });

    $$(".btn-chip", root).forEach(function (btn) {
      btn.addEventListener("click", function () {
        $$(".btn-chip", root).forEach(function (b) { b.setAttribute("aria-selected", "false"); });
        btn.setAttribute("aria-selected", "true");
        state.surface = btn.dataset.surface;
        statusEl.textContent = "Switched inbound surface to " + btn.textContent.trim();
        resetSimulation();
      });
    });

    if (playBtn) {
      playBtn.addEventListener("click", function () {
        state.playing = !state.playing;
        playBtn.textContent = state.playing ? "Pause" : "Play";
      });
    }

    if (stepBtn) {
      stepBtn.addEventListener("click", function () {
        state.playing = false;
        if (playBtn) playBtn.textContent = "Play";
        stepSimulation();
      });
    }

    if (speedBtn) {
      speedBtn.addEventListener("click", function () {
        var speeds = [1, 2, 5];
        var cur = speeds.indexOf(state.speed);
        state.speed = speeds[(cur + 1) % speeds.length];
        speedBtn.textContent = state.speed + "×";
      });
    }

    if (resetBtn) {
      resetBtn.addEventListener("click", function () {
        resetSimulation();
      });
    }

    // Fault injectors
    var faultRate = $("fault-rate-limit");
    if (faultRate) {
      faultRate.addEventListener("click", function () {
        state.fault = "rate_limit";
        statusEl.textContent = "⚡ Injected 429 Overload: Stage 5 failover leg engaged";
        hudRoute.textContent = "openai · gpt-4o (ladder failover)";
        updateInspector(4);
      });
    }
    var faultDeny = $("fault-deny-rule");
    if (faultDeny) {
      faultDeny.addEventListener("click", function () {
        state.fault = "deny_rule";
        statusEl.textContent = "🛑 Injected rm -rf /: Stage 9 Deny rule triggered outright";
        updateInspector(8);
      });
    }
    var faultAsk = $("fault-ask-gate");
    if (faultAsk) {
      faultAsk.addEventListener("click", function () {
        state.fault = "ask_gate";
        statusEl.textContent = "✋ Injected Out-of-Bounds Write: Stage 9 Ask gate waiting for approval";
        updateInspector(8);
      });
    }
    var faultContext = $("fault-context-spike");
    if (faultContext) {
      faultContext.addEventListener("click", function () {
        state.fault = "context_spike";
        statusEl.textContent = "📦 Injected Context Overflow: Stage 6 Compaction & Proactive Retrieval armed";
        updateInspector(5);
      });
    }

    function resetSimulation() {
      state.stepIndex = 0;
      state.fault = null;
      state.particles = [];
      state.swarmPackets = [];
      if (state.scale === "single") {
        state.totalTurns = 1;
        state.turnsCompleted = 1;
        hudTurns.textContent = "1 / 1";
        hudTokens.textContent = "6,118 / 412";
        hudSpend.textContent = "$0.021 / $5.00";
        hudEntries.textContent = "14 appended";
        hudRoute.textContent = "anthropic · claude-3-7-sonnet";
        statusEl.textContent = "Single turn: Detailed anatomical inspection";
      } else if (state.scale === "multi") {
        state.totalTurns = 3;
        state.turnsCompleted = 1;
        state.multiTurn = 1;
        hudTurns.textContent = "1 / 3";
        hudTokens.textContent = "18,420 / 1,240";
        hudSpend.textContent = "$0.063 / $5.00";
        hudEntries.textContent = "38 appended";
        hudRoute.textContent = "anthropic · claude-3-7-sonnet";
        statusEl.textContent = "Multi-turn tool chain: Turn 1 of 3 (cargo test green obligation)";
      } else {
        state.totalTurns = 100;
        state.turnsCompleted = 0;
        hudTurns.textContent = "0 / 100";
        hudTokens.textContent = "0 / 0";
        hudSpend.textContent = "$0.000 / $10.00";
        hudEntries.textContent = "0 appended";
        hudRoute.textContent = "parallel multi-provider ladder";
        statusEl.textContent = "100-turn swarm: High-throughput pipeline streaming";
        for (var i = 0; i < 28; i++) {
          spawnSwarmPacket(Math.random() * 0.8);
        }
      }
      updateInspector(0);
    }

    function stepSimulation() {
      state.stepIndex = (state.stepIndex + 1) % STAGES.length;
      updateInspector(state.stepIndex);
      nodes.forEach(function (n, i) {
        n.classList.toggle("processing", i === state.stepIndex);
      });
    }

    function spawnSwarmPacket(offsetProgress) {
      state.swarmPackets.push({
        id: Math.floor(Math.random() * 100000),
        progress: offsetProgress || 0,
        speed: 0.002 + Math.random() * 0.003,
        surface: ["cli", "chat", "web", "cron"][Math.floor(Math.random() * 4)],
        tokensIn: Math.floor(4000 + Math.random() * 8000),
        tokensOut: Math.floor(200 + Math.random() * 600),
        color: Math.random() > 0.85 ? "#ee9278" : "#73a982"
      });
    }

    // Canvas animation loop
    var lastTime = performance.now();
    var animId = null;

    function resizeCanvas() {
      if (!canvas) return;
      var rect = canvas.getBoundingClientRect();
      var dpr = window.devicePixelRatio || 1;
      var w = Math.round(rect.width * dpr);
      var h = Math.round(rect.height * dpr);
      if (canvas.width !== w || canvas.height !== h) {
        canvas.width = w;
        canvas.height = h;
      }
    }
    window.addEventListener("resize", resizeCanvas);
    resizeCanvas();

    function draw() {
      if (!ctx || !canvas) return;
      var now = performance.now();
      var dt = (now - lastTime) / 1000;
      lastTime = now;
      if (dt > 0.1) dt = 0.1;

      var dpr = window.devicePixelRatio || 1;
      var width = canvas.width / dpr;
      var height = canvas.height / dpr;

      ctx.save();
      ctx.scale(dpr, dpr);
      ctx.clearRect(0, 0, width, height);

      // Node layout positions (2 rows of 5 or 10 across)
      var cols = width > 700 ? 5 : 2;
      var rows = cols === 5 ? 2 : 5;
      var nodePositions = [];
      var padX = 70;
      var padY = 55;
      var availW = width - padX * 2;
      var availH = height - padY * 2;

      for (var i = 0; i < STAGES.length; i++) {
        var c = i % cols;
        var r = Math.floor(i / cols);
        // S-curve snake connection
        if (r % 2 === 1 && cols === 5) {
          c = (cols - 1) - c;
        }
        var x = padX + (availW / (cols - 1)) * c;
        var y = padY + (availH / (rows - 1)) * r;
        nodePositions.push({ x: x, y: y });
      }

      // Draw bus corridors (connecting lines)
      ctx.lineWidth = 2;
      ctx.strokeStyle = "rgba(145, 142, 134, 0.22)";
      ctx.setLineDash([4, 6]);
      ctx.beginPath();
      for (var j = 0; j < nodePositions.length; j++) {
        var p = nodePositions[j];
        if (j === 0) ctx.moveTo(p.x, p.y);
        else ctx.lineTo(p.x, p.y);
      }
      ctx.stroke();
      ctx.setLineDash([]);

      // Simulation stepping & packet progression
      if (state.playing) {
        if (state.scale === "single" || state.scale === "multi") {
          state.stepIndex += dt * 0.9 * state.speed;
          if (state.stepIndex >= STAGES.length) {
            state.stepIndex = 0;
            if (state.scale === "multi") {
              state.multiTurn = (state.multiTurn % 3) + 1;
              hudTurns.textContent = state.multiTurn + " / 3";
              statusEl.textContent = "Multi-turn chain: Turn " + state.multiTurn + " of 3";
            }
          }
          var currentIdx = Math.floor(state.stepIndex);
          if (currentIdx !== state.activeStage) {
            updateInspector(currentIdx);
            nodes.forEach(function (n, k) {
              n.classList.toggle("processing", k === currentIdx);
            });
          }
        } else {
          // Swarm mode: update many packets
          for (var s = state.swarmPackets.length - 1; s >= 0; s--) {
            var pkt = state.swarmPackets[s];
            pkt.progress += pkt.speed * state.speed;
            if (pkt.progress >= 1) {
              state.swarmPackets.splice(s, 1);
              state.turnsCompleted = Math.min(100, state.turnsCompleted + 1);
              state.tokensIn += pkt.tokensIn;
              state.tokensOut += pkt.tokensOut;
              state.entries += Math.floor(10 + Math.random() * 4);
              state.spend = Math.min(9.99, state.spend + 0.019);

              hudTurns.textContent = state.turnsCompleted + " / 100";
              hudTokens.textContent = state.tokensIn.toLocaleString() + " / " + state.tokensOut.toLocaleString();
              hudEntries.textContent = state.entries.toLocaleString() + " appended";
              hudSpend.textContent = "$" + state.spend.toFixed(3) + " / $10.00";

              if (state.turnsCompleted < 100 && state.swarmPackets.length < 32) {
                spawnSwarmPacket(0);
              }
            }
          }
          if (state.turnsCompleted >= 100 && state.swarmPackets.length === 0) {
            statusEl.textContent = "Swarm complete: 100 turns audited, 0 unrecorded effects, 100% budget compliance";
          }
        }
      }

      // Draw single/multi turn animated packet
      if (state.scale === "single" || state.scale === "multi") {
        var progress = state.stepIndex;
        var fromIdx = Math.floor(progress);
        var toIdx = Math.min(STAGES.length - 1, fromIdx + 1);
        var frac = progress - fromIdx;
        var pFrom = nodePositions[fromIdx] || nodePositions[0];
        var pTo = nodePositions[toIdx] || pFrom;
        var curX = pFrom.x + (pTo.x - pFrom.x) * frac;
        var curY = pFrom.y + (pTo.y - pFrom.y) * frac;

        // Glowing packet halo
        var grad = ctx.createRadialGradient(curX, curY, 2, curX, curY, 20);
        grad.addColorStop(0, "rgba(223, 121, 95, 0.9)");
        grad.addColorStop(0.5, "rgba(223, 121, 95, 0.25)");
        grad.addColorStop(1, "rgba(223, 121, 95, 0)");
        ctx.fillStyle = grad;
        ctx.beginPath();
        ctx.arc(curX, curY, 20, 0, Math.PI * 2);
        ctx.fill();

        // Core bead
        ctx.fillStyle = "#df795f";
        ctx.beginPath();
        ctx.arc(curX, curY, 6, 0, Math.PI * 2);
        ctx.fill();
        ctx.strokeStyle = "#eeeae2";
        ctx.lineWidth = 1.5;
        ctx.stroke();
      } else {
        // Draw swarm packets
        for (var sp = 0; sp < state.swarmPackets.length; sp++) {
          var spkt = state.swarmPackets[sp];
          var totalDist = STAGES.length - 1;
          var pIdxFloat = spkt.progress * totalDist;
          var fIdx = Math.floor(pIdxFloat);
          var tIdx = Math.min(totalDist, fIdx + 1);
          var fFrac = pIdxFloat - fIdx;
          var posF = nodePositions[fIdx];
          var posT = nodePositions[tIdx];
          if (!posF || !posT) continue;
          var px = posF.x + (posT.x - posF.x) * fFrac;
          var py = posF.y + (posT.y - posF.y) * fFrac;

          ctx.fillStyle = spkt.color;
          ctx.beginPath();
          ctx.arc(px, py, 4, 0, Math.PI * 2);
          ctx.fill();
        }
      }

      // Draw node stations
      for (var k = 0; k < nodePositions.length; k++) {
        var np = nodePositions[k];
        var isCurrent = (state.scale !== "swarm") && (k === Math.floor(state.stepIndex));
        var isPassed = (state.scale !== "swarm") && (k <= Math.floor(state.stepIndex));

        // Outer ring
        ctx.fillStyle = isCurrent ? "rgba(223, 121, 95, 0.18)" : "rgba(34, 34, 31, 0.85)";
        ctx.beginPath();
        ctx.arc(np.x, np.y, 14, 0, Math.PI * 2);
        ctx.fill();
        ctx.strokeStyle = isCurrent ? "#df795f" : isPassed ? "#73a982" : "#34342f";
        ctx.lineWidth = isCurrent ? 2 : 1.2;
        ctx.stroke();

        // Inner marker
        ctx.fillStyle = isCurrent ? "#ee9278" : isPassed ? "#73a982" : "#918e86";
        ctx.beginPath();
        ctx.arc(np.x, np.y, 4, 0, Math.PI * 2);
        ctx.fill();

        // Label above / below node
        ctx.font = "10.5px 'SF Mono', ui-monospace, Menlo, monospace";
        ctx.fillStyle = isCurrent ? "#eeeae2" : "#8b8880";
        ctx.textAlign = "center";
        var labelY = (k >= cols && cols === 5) ? np.y + 24 : np.y - 20;
        ctx.fillText(STAGES[k].num + " " + STAGES[k].title, np.x, labelY);
      }

      ctx.restore();
      animId = requestAnimationFrame(draw);
    }

    if (!reduce) {
      animId = requestAnimationFrame(draw);
    } else {
      updateInspector(0);
    }
  }

  // =========================================================================
  // 3D Virtual World Tour Game Engine: The Vak Citadel
  // =========================================================================
  function tour3D(M, reduce) {
    var root = $("world-viewport");
    if (!root) return;

    var canvas = $("world-canvas");
    var ctx = canvas ? canvas.getContext("2d") : null;
    var minimap = $("minimap-canvas");
    var miniCtx = minimap ? minimap.getContext("2d") : null;

    var btnTour = $("btn-mode-tour");
    var btnOrbit = $("btn-mode-orbit");
    var btnSound = $("btn-sound");
    var btnReset = $("btn-reset-cam");
    var soundLabel = $("sound-label");
    var statusBadge = $("hud-status-badge");

    var codexChip = $("codex-chip");
    var codexCoords = $("codex-coords");
    var codexTitle = $("codex-title");
    var codexDesc = $("codex-desc");
    var codexRule = $("codex-rule");
    var codexCodeLabel = $("codex-code-label");
    var codexCode = $("codex-code");
    var ribbon = $("sector-nav-ribbon");

    var modal = $("world-ask-modal");
    var btnApprove = $("btn-world-approve");
    var btnDeny = $("btn-world-deny");
    var askActionText = $("ask-action");

    var btnMusic = $("btn-music");
    var musicLabel = $("music-label");
    var tooltip = $("world-tooltip");
    var ttNum = $("tt-num");
    var ttTitle = $("tt-title");

    // =========================================================================
    // Generative Web Audio Synthesizer: Cybernetic Citadel Soundtrack & SFX
    // =========================================================================
    var audioCtx = null;
    var soundEnabled = true;
    var musicEnabled = true;
    var musicGain = null;
    var musicTimer = null;
    var musicDroneOsc1 = null;
    var musicDroneOsc2 = null;
    var musicChordIndex = 0;
    var musicStep = 0;

    // D-minor pentatonic chords: Dm, F, Am, Bb, C, Gm
    var CHORDS = [
      [146.83, 220.00, 261.63, 293.66, 349.23], // Dm9 (D3, A3, C4, D4, F4)
      [174.61, 220.00, 261.63, 329.63, 392.00], // Fmaj7 (F3, A3, C4, E4, G4)
      [110.00, 164.81, 220.00, 261.63, 329.63], // Am7 (A2, E3, A3, C4, E4)
      [116.54, 174.61, 233.08, 293.66, 349.23], // Bbmaj7 (Bb2, F3, Bb3, D4, F4)
      [130.81, 196.00, 261.63, 329.63, 392.00], // C9 (C3, G3, C4, E4, G4)
      [98.00,  146.83, 196.00, 233.08, 293.66]  // Gm7 (G2, D3, G3, Bb3, D4)
    ];

    function initAudio() {
      if (!audioCtx && (window.AudioContext || window.webkitAudioContext)) {
        try {
          audioCtx = new (window.AudioContext || window.webkitAudioContext)();
        } catch (e) {}
      }
      if (audioCtx && audioCtx.state === "suspended") {
        audioCtx.resume();
      }
      if (musicEnabled && !musicTimer && audioCtx) {
        startGenerativeMusic();
      }
    }

    function startGenerativeMusic() {
      if (!audioCtx || musicTimer) return;
      try {
        var masterT = audioCtx.currentTime;

        // Music Master Gain
        musicGain = audioCtx.createGain();
        musicGain.gain.setValueAtTime(0.001, masterT);
        musicGain.gain.exponentialRampToValueAtTime(0.09, masterT + 2.0);

        // Low-pass resonant filter
        var filter = audioCtx.createBiquadFilter();
        filter.type = "lowpass";
        filter.frequency.setValueAtTime(420, masterT);
        filter.Q.setValueAtTime(2.2, masterT);

        // Dual feedback delay lines for tape space
        var delay = audioCtx.createDelay();
        delay.delayTime.setValueAtTime(0.36, masterT);
        var delayFeedback = audioCtx.createGain();
        delayFeedback.gain.setValueAtTime(0.42, masterT);

        delay.connect(delayFeedback);
        delayFeedback.connect(delay);
        delay.connect(filter);

        filter.connect(musicGain);
        musicGain.connect(audioCtx.destination);

        // Sub-bass drone (55Hz / 110Hz detuned)
        musicDroneOsc1 = audioCtx.createOscillator();
        musicDroneOsc2 = audioCtx.createOscillator();
        musicDroneOsc1.type = "sawtooth";
        musicDroneOsc2.type = "triangle";
        musicDroneOsc1.frequency.setValueAtTime(55.0, masterT);
        musicDroneOsc2.frequency.setValueAtTime(55.4, masterT);

        var droneGain = audioCtx.createGain();
        droneGain.gain.setValueAtTime(0.04, masterT);
        musicDroneOsc1.connect(droneGain);
        musicDroneOsc2.connect(droneGain);
        droneGain.connect(filter);

        musicDroneOsc1.start();
        musicDroneOsc2.start();

        // Arpeggiator pulse timer
        musicTimer = setInterval(function () {
          if (!audioCtx || !musicEnabled) return;
          var curTime = audioCtx.currentTime;
          var chord = CHORDS[musicChordIndex % CHORDS.length];
          var noteFreq = chord[musicStep % chord.length];

          // Occasional chord shift
          if (musicStep % 8 === 0 && Math.random() > 0.4) {
            musicChordIndex = (musicChordIndex + 1) % CHORDS.length;
          }
          musicStep++;

          // Arpeggio chime note
          var noteOsc = audioCtx.createOscillator();
          var noteGain = audioCtx.createGain();
          noteOsc.type = (musicStep % 3 === 0) ? "sine" : "triangle";
          noteOsc.frequency.setValueAtTime(noteFreq, curTime);

          noteGain.gain.setValueAtTime(0.001, curTime);
          noteGain.gain.exponentialRampToValueAtTime(0.045, curTime + 0.02);
          noteGain.gain.exponentialRampToValueAtTime(0.0001, curTime + 0.38);

          noteOsc.connect(noteGain);
          noteGain.connect(delay);
          noteGain.connect(filter);

          noteOsc.start(curTime);
          noteOsc.stop(curTime + 0.4);
        }, 340);
      } catch (e) {}
    }

    function stopGenerativeMusic() {
      if (musicTimer) {
        clearInterval(musicTimer);
        musicTimer = null;
      }
      if (musicGain && audioCtx) {
        try {
          var t = audioCtx.currentTime;
          musicGain.gain.setValueAtTime(musicGain.gain.value, t);
          musicGain.gain.exponentialRampToValueAtTime(0.0001, t + 0.6);
          setTimeout(function () {
            if (musicDroneOsc1) { try { musicDroneOsc1.stop(); } catch (e) {} musicDroneOsc1 = null; }
            if (musicDroneOsc2) { try { musicDroneOsc2.stop(); } catch (e) {} musicDroneOsc2 = null; }
          }, 700);
        } catch (e) {}
      }
    }

    function playSound(type) {
      if (!soundEnabled || !audioCtx) return;
      try {
        var t = audioCtx.currentTime;
        var osc = audioCtx.createOscillator();
        var gain = audioCtx.createGain();
        osc.connect(gain);
        gain.connect(audioCtx.destination);

        if (type === "tick") {
          osc.type = "sine";
          osc.frequency.setValueAtTime(2400, t);
          osc.frequency.exponentialRampToValueAtTime(800, t + 0.015);
          gain.gain.setValueAtTime(0.04, t);
          gain.gain.exponentialRampToValueAtTime(0.001, t + 0.015);
          osc.start(t);
          osc.stop(t + 0.015);
        } else if (type === "click") {
          osc.type = "triangle";
          osc.frequency.setValueAtTime(1400, t);
          osc.frequency.exponentialRampToValueAtTime(300, t + 0.03);
          gain.gain.setValueAtTime(0.08, t);
          gain.gain.exponentialRampToValueAtTime(0.001, t + 0.03);
          osc.start(t);
          osc.stop(t + 0.03);
        } else if (type === "blip") {
          osc.type = "sine";
          osc.frequency.setValueAtTime(880, t);
          osc.frequency.exponentialRampToValueAtTime(1760, t + 0.08);
          gain.gain.setValueAtTime(0.08, t);
          gain.gain.exponentialRampToValueAtTime(0.001, t + 0.08);
          osc.start(t);
          osc.stop(t + 0.08);
        } else if (type === "warp") {
          osc.type = "triangle";
          osc.frequency.setValueAtTime(160, t);
          osc.frequency.exponentialRampToValueAtTime(940, t + 0.28);
          gain.gain.setValueAtTime(0.14, t);
          gain.gain.exponentialRampToValueAtTime(0.001, t + 0.28);
          osc.start(t);
          osc.stop(t + 0.28);
        } else if (type === "deny") {
          osc.type = "sawtooth";
          osc.frequency.setValueAtTime(360, t);
          osc.frequency.setValueAtTime(180, t + 0.08);
          osc.frequency.setValueAtTime(70, t + 0.16);
          gain.gain.setValueAtTime(0.24, t);
          gain.gain.exponentialRampToValueAtTime(0.001, t + 0.38);
          osc.start(t);
          osc.stop(t + 0.38);
        } else if (type === "ask") {
          osc.type = "sine";
          osc.frequency.setValueAtTime(659.25, t);
          osc.frequency.setValueAtTime(880, t + 0.1);
          gain.gain.setValueAtTime(0.14, t);
          gain.gain.exponentialRampToValueAtTime(0.001, t + 0.32);
          osc.start(t);
          osc.stop(t + 0.32);
        } else if (type === "success") {
          osc.type = "triangle";
          osc.frequency.setValueAtTime(523.25, t); // C5
          osc.frequency.setValueAtTime(659.25, t + 0.08); // E5
          osc.frequency.setValueAtTime(783.99, t + 0.16); // G5
          osc.frequency.setValueAtTime(1046.50, t + 0.24); // C6
          gain.gain.setValueAtTime(0.14, t);
          gain.gain.exponentialRampToValueAtTime(0.001, t + 0.48);
          osc.start(t);
          osc.stop(t + 0.48);
        }
      } catch (e) {}
    }

    if (btnMusic) {
      btnMusic.addEventListener("click", function () {
        initAudio();
        musicEnabled = !musicEnabled;
        musicLabel.textContent = "Music: " + (musicEnabled ? "ON" : "OFF");
        btnMusic.classList.toggle("active", musicEnabled);
        if (musicEnabled) {
          startGenerativeMusic();
        } else {
          stopGenerativeMusic();
        }
        playSound("click");
      });
    }

    if (btnSound) {
      btnSound.addEventListener("click", function () {
        initAudio();
        soundEnabled = !soundEnabled;
        soundLabel.textContent = "SFX: " + (soundEnabled ? "ON" : "OFF");
        btnSound.classList.toggle("active", soundEnabled);
        if (soundEnabled) playSound("blip");
      });
    }

    // Attach hover sound to all interactive elements
    $$(".game-btn, .quest-btn, .ribbon-sector-btn", root).forEach(function (el) {
      el.addEventListener("mouseenter", function () {
        playSound("tick");
      });
      el.addEventListener("click", function () {
        playSound("click");
      });
    });

    // 10 Sector Megastructures of the Vak Citadel
    var SECTORS = [
      {
        id: "surface",
        num: "01",
        title: "The Surface Bastion & Request Port",
        coords: { x: -280, y: 0, z: -180 },
        height: 65,
        baseSize: 55,
        color: "#df795f",
        wireColor: "#ee9278",
        shape: "pyramid_spaceport",
        source: "crates/vak/src/cli.rs · crates/vak-server/src/gateway.rs",
        desc: "The planetary gateway port where requests arrive from CLI args, terminal PTY, chat webhooks (Telegram/Discord/Slack), or cron schedules. The inbound identity is bound to a strict 3-segment bot key (surface:chat:bot_id).",
        rule: "Rule 24: One physical chat served by several bots gets one identity per bot. Gateway binds to canonical workspace ~/vak-home.",
        code: 'InboundRequest {\n    surface: Surface::Telegram,\n    chat_id: "chat_98241",\n    bot_id: Some("bot_sec_ops"),\n    sender: "alice",\n    text: "cargo test -p vak-parser",\n}'
      },
      {
        id: "admission",
        num: "02",
        title: "The Security Citadel & Allowlist Vault",
        coords: { x: -140, y: 0, z: -250 },
        height: 80,
        baseSize: 60,
        color: "#d4a85d",
        wireColor: "#eed18b",
        shape: "octagonal_fortress",
        source: "crates/vak-server/src/gateway.rs · <sessions_home>/gateway/allowlist.json",
        desc: "The fortified gate perimeter. Resolves inbound permission mode (ReadOnly, WorkspaceWrite, FullAccess). An unknown chat lands in the reviewable 'pending' quarantine (fail-closed, Rule 15). Workspace Core is leased from CorePool.",
        rule: "Rule 15: Unattended surfaces fail closed. CorePool entries recheck persisted ceilings on cache hits and cancel old leases before narrowing.",
        code: 'AllowlistEntry {\n    status: AllowlistStatus::Allowed,\n    workspace: "/Users/alice/vak/project",\n    permission_mode: PermissionMode::WorkspaceWrite,\n    inherit_bot_policy: true,\n}'
      },
      {
        id: "ledger",
        num: "03",
        title: "The Monolith of the Immutable Ledger",
        coords: { x: 0, y: 0, z: -290 },
        height: 120,
        baseSize: 45,
        color: "#73a982",
        wireColor: "#9cd2ab",
        shape: "obelisk_monolith",
        source: "crates/vak-session/src/log.rs",
        desc: "The bedrock crystalline tower where every model-visible byte, tool invocation, and decision is permanently engraved into append-only JSONL. Lines are never deleted or rewritten; compaction is an append-only entry.",
        rule: "Invariant 1 & 2: Model-visible means logged. Append-only sessions: never rewrite or delete entries. derive_messages() reconstructs truth.",
        code: 'SessionEntry::Message(MessageRecord {\n    id: "msg_01K7Q4WCE2_01",\n    parent_id: Some("msg_01K7Q4WCE2_00"),\n    role: Role::User,\n    content: [ContentBlock::Text("cargo test -p vak-parser")],\n})'
      },
      {
        id: "intent",
        num: "04",
        title: "The Intent Observatory & Astrolabe",
        coords: { x: 140, y: 0, z: -250 },
        height: 75,
        baseSize: 55,
        color: "#7c9fc9",
        wireColor: "#a3c2e8",
        shape: "domed_observatory",
        source: "crates/vak-intent/src/lib.rs · crates/vak-core/src/commitments.rs",
        desc: "Rotating celestial dome that focuses user prompts into an explicit OutcomeSpec. Derives work horizon (Direct vs Managed Work), narrows capability domain slices, and opens an audit episode stamped with a commitment ID.",
        rule: "Commitment Kernel: Intent projections only narrow authority; uncertainty never expands privilege. Horizon derives from enumeration.",
        code: 'OutcomeSpec {\n    objective: "Verify parser test suite",\n    horizon: Horizon::Direct,\n    required_domains: ["engineering"],\n    evidence_max_age_secs: Some(86400),\n}'
      },
      {
        id: "route",
        num: "05",
        title: "The Route Spire & FinOps Treasury",
        coords: { x: 280, y: 0, z: -180 },
        height: 110,
        baseSize: 50,
        color: "#d4a85d",
        wireColor: "#eed18b",
        shape: "antenna_spire",
        source: "crates/vak-core/src/lib.rs (plan_route_ladder) · finops.rs",
        desc: "High-altitude relay spire that plans a fresh route ladder on every single turn using live evidence ledgers and warm model caches. SpendGate locks in token budgets and enforces dispatch ceilings before provider contact.",
        rule: "Invariant 7: Route ladder is planned fresh on every turn. Invariant 8: Secrets live in canonical .env, never git. Invariant 9: Discovered catalogues.",
        code: 'RouteLadder {\n    primary: RouteLeg { provider: "anthropic", model: "claude-3-7-sonnet" },\n    fallbacks: [RouteLeg { provider: "openai", model: "gpt-4o" }],\n    budget_remaining_usd: 4.979,\n}'
      },
      {
        id: "context",
        num: "06",
        title: "The Context Tokamak & Memory Matrix",
        coords: { x: 260, y: 0, z: 30 },
        height: 70,
        baseSize: 65,
        color: "#df795f",
        wireColor: "#ee9278",
        shape: "torus_tokamak",
        source: "crates/vak-agent/src/context.rs · crates/vak-agent/src/memory.rs",
        desc: "Magnetic fusion chamber managing token pressures. If context exceeds input budgets: proactive retrieval tractors relevant turns verbatim, older turns are summarized via compaction LLM calls, or handoff rescue capsules deploy.",
        rule: "Context Policy: Proactive retrieval keeps relevant turns verbatim during compaction. Still over budget triggers handoff reset.",
        code: 'CompactionPlan {\n    tokens_before: 182400,\n    keep_recent: 8,\n    proactive_retrieved: 2,\n    summary_tokens: 340,\n}'
      },
      {
        id: "llm",
        num: "07",
        title: "The Neural Inference Core",
        coords: { x: 150, y: 0, z: 200 },
        height: 95,
        baseSize: 60,
        color: "#7c9fc9",
        wireColor: "#a3c2e8",
        shape: "pulsing_polyhedron",
        source: "crates/vak-llm/src/lib.rs · crates/vak-agent/src/lib.rs",
        desc: "The pulsing computational heart where model inference streams. Every AgentEvent carries both delta and snapshot. Monitored by watchdog timers, circuit breakers (429 vs network drop), and run-level endurance retries.",
        rule: "Invariant 4: Streaming carries delta AND snapshot. Invariant 5: Abort preserves partial output. Informed transience feeds endurance.",
        code: 'AgentEvent::Delta {\n    delta: "Running cargo test...",\n    snapshot: "I will now run cargo test -p vak-parser...",\n    usage: Usage { prompt_tokens: 6118, completion_tokens: 412 },\n}'
      },
      {
        id: "broker",
        num: "08",
        title: "The Quarantined Tool Foundry",
        coords: { x: 0, y: 0, z: 250 },
        height: 60,
        baseSize: 70,
        color: "#d4a85d",
        wireColor: "#eed18b",
        shape: "hazmat_compound",
        source: "crates/vak-tools/src/lib.rs · crates/vak-sandbox",
        desc: "Subprocess containment sector. Model tools cross an IPC broker boundary into disposable __tool_worker process groups. Quarantined environments receive zero ambient parent environment secrets, provider credentials, or control-plane handles.",
        rule: "Invariant 14: Model tools cross a broker boundary. Subprocesses receive small operational allowlists, never parent secrets.",
        code: 'PendingToolCall {\n    id: "call_tool_01",\n    name: "bash",\n    input: { "command": "cargo test -p vak-parser" },\n    worker: ToolWorker::DisposableGroup,\n}'
      },
      {
        id: "permission",
        num: "09",
        title: "The Precedence Tribunal & Gate",
        coords: { x: -150, y: 0, z: 200 },
        height: 105,
        baseSize: 65,
        color: "#d86f72",
        wireColor: "#f18d90",
        shape: "tribunal_pillars",
        source: "crates/vak-permission/src/engine.rs",
        desc: "Four Monoliths of Precedence: Deny Rules -> Ask Rules -> Allow Rules -> Mode Default. Deny wins outright before mode evaluation. Ask pauses execution for human approval or fails closed on unattended timeouts.",
        rule: "Invariant 13: FullAccess is an explicit human trust decision. Invariant 15: Unattended surfaces fail closed. Deny always wins.",
        code: 'Decision::Ask {\n    reason: "shell command requires approval in workspace-write mode",\n    source: AskSource::ModeDefault,\n}'
      },
      {
        id: "receipt",
        num: "10",
        title: "The Receipt Forge & Outbound Spaceport",
        coords: { x: -260, y: 0, z: 30 },
        height: 85,
        baseSize: 60,
        color: "#73a982",
        wireColor: "#9cd2ab",
        shape: "launch_hyperrail",
        source: "crates/vak-agent/src/spend.rs · crates/vak-delivery/src/lib.rs",
        desc: "The cryptographic settlement forge. Every turn writes an immutable WorkReceipt recording prompt/completion tokens, cost, latency, provider, and failure domain. DeliveryHub projects ANSI streams, reactive diffs, or chat chunks.",
        rule: "Invariant 3: Errors are values (is_error). WorkReceipt is the authoritative per-turn dispatch record for audit.",
        code: 'WorkReceipt {\n    provider: "anthropic",\n    model: "claude-3-7-sonnet",\n    usage: { input: 6118, output: 412 },\n    latency_ms: 1240,\n    settled_usd: 0.021,\n}'
      }
    ];

    // Build Sector Navigation Ribbon
    ribbon.innerHTML = SECTORS.map(function (s, i) {
      return '<button type="button" class="ribbon-sector-btn' + (i === 0 ? " active" : "") + '" data-idx="' + i + '">' +
        'S' + s.num + ' · ' + s.id.toUpperCase() +
      '</button>';
    }).join("");

    var ribbonBtns = $$(".ribbon-sector-btn", ribbon);

    // 3D Game World Camera & Flight State
    var camera = {
      x: 0,
      y: 190,
      z: 360,
      targetX: -280,
      targetY: 35,
      targetZ: -180,
      pitch: 0.44, // rad
      yaw: -0.85,  // rad
      distance: 380,
      fov: 460
    };

    var game = {
      mode: "tour", // 'tour' or 'orbit'
      activeSector: 0,
      tourProgress: 0,
      tourSpeed: 0.0016,
      quest: "nominal", // nominal, deny, ask, failover, swarm
      pausedForModal: false,
      dronePos: { x: -280, y: 35, z: -180 },
      droneTarget: { x: -280, y: 35, z: -180 },
      shockwave: null,
      particles: [],
      swarmPackets: []
    };

    // Keyboard Input State for WASD Roam and Quick Shortcuts
    var keys = {};
    window.addEventListener("keydown", function (e) {
      initAudio();
      var k = e.key.toLowerCase();
      keys[k] = true;
      if (["w", "a", "s", "d", "arrowup", "arrowdown", "arrowleft", "arrowright"].indexOf(k) >= 0) {
        if (game.mode === "tour") {
          setMode("orbit");
        }
      }

      // Ignore single-letter game hotkeys if user is focusing an input
      if (e.target && (e.target.tagName === "INPUT" || e.target.tagName === "TEXTAREA")) return;

      if (k === "m" && btnMusic) {
        btnMusic.click();
      } else if (k === "s" && btnSound) {
        btnSound.click();
      } else if (k === "t") {
        setMode("tour");
        playSound("warp");
      } else if (k === "o") {
        setMode("orbit");
        playSound("blip");
      } else if (k === "r") {
        warpToSector(0);
        playSound("warp");
      } else if (k >= "1" && k <= "9") {
        var sNum = parseInt(k, 10) - 1;
        if (sNum < SECTORS.length) {
          warpToSector(sNum);
          playSound("warp");
        }
      } else if (k === "0") {
        warpToSector(9);
        playSound("warp");
      }
    });
    window.addEventListener("keyup", function (e) {
      keys[e.key.toLowerCase()] = false;
    });

    // Auto-resume audio on first user touch/pointer anywhere in the Citadel
    root.addEventListener("pointerdown", function () {
      initAudio();
    }, { once: true });

    // Mouse Drag & Hover Raycasting State for 3D Orbit & Landmarks
    var isDragging = false;
    var mouseDownPos = { x: 0, y: 0 };
    var lastMouseX = 0;
    var lastMouseY = 0;
    var hoveredSector = null;
    var lastHoveredSector = null;

    canvas.addEventListener("mousedown", function (e) {
      initAudio();
      isDragging = true;
      mouseDownPos.x = e.clientX;
      mouseDownPos.y = e.clientY;
      lastMouseX = e.clientX;
      lastMouseY = e.clientY;
    });
    window.addEventListener("mouseup", function () { isDragging = false; });

    canvas.addEventListener("mousemove", function (e) {
      if (isDragging) {
        var dx = e.clientX - lastMouseX;
        var dy = e.clientY - lastMouseY;
        lastMouseX = e.clientX;
        lastMouseY = e.clientY;

        if (game.mode === "tour") {
          setMode("orbit");
        }
        camera.yaw += dx * 0.006;
        camera.pitch = Math.max(0.08, Math.min(1.4, camera.pitch - dy * 0.006));
        if (tooltip) tooltip.hidden = true;
        return;
      }

      // Hover Raycasting against 10 Citadel Megastructures
      var rect = canvas.getBoundingClientRect();
      var mx = e.clientX - rect.left;
      var my = e.clientY - rect.top;
      var dpr = window.devicePixelRatio || 1;
      var w = canvas.width / dpr;
      var h = canvas.height / dpr;

      var closestIdx = null;
      var minDist = 34;
      var closestScreenPos = null;

      for (var sc = 0; sc < SECTORS.length; sc++) {
        var secItem = SECTORS[sc];
        var bPos = project3D(secItem.coords.x, secItem.height + 14, secItem.coords.z, w, h);
        if (bPos) {
          var dist = Math.hypot(bPos.x - mx, bPos.y - my);
          if (dist < minDist) {
            minDist = dist;
            closestIdx = sc;
            closestScreenPos = bPos;
          }
        }
      }

      if (closestIdx !== null && closestScreenPos && tooltip) {
        hoveredSector = closestIdx;
        if (hoveredSector !== lastHoveredSector) {
          playSound("tick");
          lastHoveredSector = hoveredSector;
        }
        var hSec = SECTORS[hoveredSector];
        if (ttNum) ttNum.textContent = "SECTOR " + hSec.num;
        if (ttTitle) ttTitle.textContent = hSec.title.split(" & ")[0];
        tooltip.style.left = closestScreenPos.x + "px";
        tooltip.style.top = closestScreenPos.y + "px";
        tooltip.hidden = false;
        canvas.style.cursor = "pointer";
      } else {
        hoveredSector = null;
        lastHoveredSector = null;
        if (tooltip) tooltip.hidden = true;
        canvas.style.cursor = "crosshair";
      }
    });

    canvas.addEventListener("mouseleave", function () {
      hoveredSector = null;
      lastHoveredSector = null;
      if (tooltip) tooltip.hidden = true;
      canvas.style.cursor = "crosshair";
    });

    // Click to Warp onto Hovered 3D Landmark
    canvas.addEventListener("click", function (e) {
      initAudio();
      var moved = Math.hypot(e.clientX - mouseDownPos.x, e.clientY - mouseDownPos.y);
      if (moved < 8 && hoveredSector !== null) {
        warpToSector(hoveredSector);
        playSound("warp");
      }
    });

    // Scroll to Zoom
    canvas.addEventListener("wheel", function (e) {
      e.preventDefault();
      if (game.mode === "tour") setMode("orbit");
      camera.distance = Math.max(120, Math.min(780, camera.distance + e.deltaY * 0.5));
    }, { passive: false });

    function setMode(mode) {
      game.mode = mode;
      btnTour.classList.toggle("active", mode === "tour");
      btnOrbit.classList.toggle("active", mode === "orbit");
      root.classList.toggle("cinematic", mode === "tour");
      statusBadge.textContent = mode.toUpperCase() + " MODE";
      statusBadge.className = "hud-badge" + (mode === "tour" ? " active" : "");
    }

    if (btnTour) btnTour.addEventListener("click", function () { setMode("tour"); playSound("warp"); });
    if (btnOrbit) btnOrbit.addEventListener("click", function () { setMode("orbit"); playSound("blip"); });
    if (btnReset) {
      btnReset.addEventListener("click", function () {
        warpToSector(0);
        playSound("warp");
      });
    }

    // Quest Mission Buttons
    $$(".quest-btn", root).forEach(function (btn) {
      btn.addEventListener("click", function () {
        $$(".quest-btn", root).forEach(function (b) {
          b.classList.remove("active");
          b.setAttribute("aria-checked", "false");
        });
        btn.classList.add("active");
        btn.setAttribute("aria-checked", "true");
        game.quest = btn.dataset.quest;
        game.pausedForModal = false;
        modal.hidden = true;
        initAudio();
        playSound("blip");

        if (game.quest === "swarm") {
          spawnSwarmStorm();
        } else {
          game.swarmPackets = [];
          warpToSector(0);
        }
      });
    });

    function spawnSwarmStorm() {
      game.swarmPackets = [];
      for (var i = 0; i < 48; i++) {
        game.swarmPackets.push({
          progress: Math.random(),
          speed: 0.0018 + Math.random() * 0.003,
          color: ["#df795f", "#73a982", "#d4a85d", "#7c9fc9"][Math.floor(Math.random() * 4)],
          yOffset: 15 + Math.random() * 45
        });
      }
      playSound("warp");
    }

    function warpToSector(idx) {
      game.activeSector = idx;
      game.tourProgress = idx / SECTORS.length;
      var sec = SECTORS[idx];
      game.droneTarget = { x: sec.coords.x, y: sec.height + 25, z: sec.coords.z };
      updateCodex(idx);
    }

    function updateCodex(idx) {
      game.activeSector = idx;
      var s = SECTORS[idx];
      if (!s) return;
      codexChip.textContent = "SECTOR " + s.num + " · " + s.title.toUpperCase();
      codexCoords.textContent = "X: " + (s.coords.x > 0 ? "+" : "") + s.coords.x +
        " Y: +" + s.height + " Z: " + (s.coords.z > 0 ? "+" : "") + s.coords.z;
      codexTitle.textContent = s.title;
      codexDesc.textContent = s.desc;
      codexRule.innerHTML = "<b>Invariant / Contract:</b> " + s.rule;
      codexCodeLabel.textContent = "ACTIVE STRUCT // " + s.source;
      codexCode.textContent = s.code;

      ribbonBtns.forEach(function (b, k) {
        b.classList.toggle("active", k === idx);
      });
    }

    ribbonBtns.forEach(function (b, idx) {
      b.addEventListener("click", function () {
        initAudio();
        playSound("blip");
        warpToSector(idx);
      });
    });

    // In-World Approval Modal Handlers
    if (btnApprove) {
      btnApprove.addEventListener("click", function () {
        playSound("success");
        modal.hidden = true;
        game.pausedForModal = false;
        codexRule.innerHTML = "<b>Approval Granted:</b> Out-of-bounds mutation was approved by human operator · Recorded in append-only ledger.";
      });
    }
    if (btnDeny) {
      btnDeny.addEventListener("click", function () {
        playSound("deny");
        modal.hidden = true;
        game.pausedForModal = false;
        game.shockwave = { x: SECTORS[8].coords.x, z: SECTORS[8].coords.z, radius: 10, maxRadius: 160 };
        codexRule.innerHTML = "<b>Action Denied:</b> The run stopped, partial work was preserved, and denial was logged.";
      });
    }

    // 3D Math & Projection Utilities
    function project3D(x, y, z, width, height) {
      // Translate relative to camera target
      var dx = x - camera.targetX;
      var dy = y - camera.targetY;
      var dz = z - camera.targetZ;

      // Rotate Yaw (around Y axis)
      var cosY = Math.cos(camera.yaw);
      var sinY = Math.sin(camera.yaw);
      var x1 = dx * cosY - dz * sinY;
      var z1 = dx * sinY + dz * cosY;

      // Rotate Pitch (around X axis)
      var cosP = Math.cos(camera.pitch);
      var sinP = Math.sin(camera.pitch);
      var y2 = dy * cosP - (z1 - camera.distance) * sinP;
      var z2 = dy * sinP + (z1 - camera.distance) * cosP;

      // Distance from camera plane
      var depth = z2 + camera.distance;
      if (depth <= 10) return null; // Behind camera

      var scale = camera.fov / depth;
      var px = width * 0.5 + x1 * scale;
      var py = height * 0.5 - y2 * scale;

      return { x: px, y: py, depth: depth, scale: scale };
    }

    // Procedural 3D Polygonal Geometry Drawing
    function drawPolygon3D(pts, fillColor, strokeColor, width, height) {
      var projected = [];
      var avgDepth = 0;
      for (var i = 0; i < pts.length; i++) {
        var p = project3D(pts[i].x, pts[i].y, pts[i].z, width, height);
        if (!p) return null;
        projected.push(p);
        avgDepth += p.depth;
      }
      avgDepth /= pts.length;

      return {
        depth: avgDepth,
        render: function () {
          ctx.beginPath();
          ctx.moveTo(projected[0].x, projected[0].y);
          for (var j = 1; j < projected.length; j++) {
            ctx.lineTo(projected[j].x, projected[j].y);
          }
          ctx.closePath();
          if (fillColor) {
            ctx.fillStyle = fillColor;
            ctx.fill();
          }
          if (strokeColor) {
            ctx.strokeStyle = strokeColor;
            ctx.lineWidth = 1.2;
            ctx.stroke();
          }
        }
      };
    }

    function buildBoxPolygons(cx, cy, cz, w, h, d, baseColor, wireColor, width, height) {
      var polys = [];
      var hw = w * 0.5;
      var hd = d * 0.5;

      var v = [
        { x: cx - hw, y: cy,     z: cz - hd },
        { x: cx + hw, y: cy,     z: cz - hd },
        { x: cx + hw, y: cy + h, z: cz - hd },
        { x: cx - hw, y: cy + h, z: cz - hd },
        { x: cx - hw, y: cy,     z: cz + hd },
        { x: cx + hw, y: cy,     z: cz + hd },
        { x: cx + hw, y: cy + h, z: cz + hd },
        { x: cx - hw, y: cy + h, z: cz + hd }
      ];

      // Top face
      var topPoly = drawPolygon3D([v[3], v[2], v[6], v[7]], baseColor, wireColor, width, height);
      if (topPoly) polys.push(topPoly);
      // Front face
      var frontPoly = drawPolygon3D([v[4], v[5], v[6], v[7]], "rgba(28, 28, 25, 0.9)", wireColor, width, height);
      if (frontPoly) polys.push(frontPoly);
      // Right face
      var rightPoly = drawPolygon3D([v[1], v[5], v[6], v[2]], "rgba(22, 22, 19, 0.9)", wireColor, width, height);
      if (rightPoly) polys.push(rightPoly);
      // Left face
      var leftPoly = drawPolygon3D([v[0], v[4], v[7], v[3]], "rgba(24, 24, 21, 0.9)", wireColor, width, height);
      if (leftPoly) polys.push(leftPoly);

      return polys;
    }

    function buildPyramidPolygons(cx, cy, cz, size, h, baseColor, wireColor, width, height) {
      var polys = [];
      var hs = size * 0.5;
      var apex = { x: cx, y: cy + h, z: cz };
      var b0 = { x: cx - hs, y: cy, z: cz - hs };
      var b1 = { x: cx + hs, y: cy, z: cz - hs };
      var b2 = { x: cx + hs, y: cy, z: cz + hs };
      var b3 = { x: cx - hs, y: cy, z: cz + hs };

      var f1 = drawPolygon3D([b0, b1, apex], baseColor, wireColor, width, height);
      var f2 = drawPolygon3D([b1, b2, apex], "rgba(35, 35, 30, 0.92)", wireColor, width, height);
      var f3 = drawPolygon3D([b2, b3, apex], "rgba(28, 28, 25, 0.95)", wireColor, width, height);
      var f4 = drawPolygon3D([b3, b0, apex], "rgba(32, 32, 28, 0.92)", wireColor, width, height);

      if (f1) polys.push(f1);
      if (f2) polys.push(f2);
      if (f3) polys.push(f3);
      if (f4) polys.push(f4);
      return polys;
    }

    // Resize viewport
    function resizeWorld() {
      if (!canvas) return;
      var rect = canvas.getBoundingClientRect();
      var dpr = window.devicePixelRatio || 1;
      canvas.width = Math.round(rect.width * dpr);
      canvas.height = Math.round(rect.height * dpr);
    }
    window.addEventListener("resize", resizeWorld);
    resizeWorld();

    // Main 3D Simulation Loop
    var lastFrameTime = performance.now();

    function renderWorld(time) {
      var dt = (time - lastFrameTime) / 1000;
      lastFrameTime = time;
      if (dt > 0.1) dt = 0.1;

      var dpr = window.devicePixelRatio || 1;
      var w = canvas.width / dpr;
      var h = canvas.height / dpr;

      ctx.save();
      ctx.scale(dpr, dpr);

      // Dark obsidian space skybox
      ctx.fillStyle = "#0c0c0a";
      ctx.fillRect(0, 0, w, h);

      // Distant cybernetic star particles
      ctx.fillStyle = "rgba(223, 121, 95, 0.15)";
      for (var s = 0; s < 30; s++) {
        var starX = ((s * 137.5) % w);
        var starY = ((s * 89.3 + time * 0.005) % (h * 0.6));
        ctx.fillRect(starX, starY, 1.2, 1.2);
      }

      // WASD Flight input processing in Orbit mode
      if (game.mode === "orbit" && !game.pausedForModal) {
        var moveSpeed = 160 * dt;
        var forwardX = -Math.sin(camera.yaw);
        var forwardZ = Math.cos(camera.yaw);
        var rightX = Math.cos(camera.yaw);
        var rightZ = Math.sin(camera.yaw);

        if (keys["w"] || keys["arrowup"]) {
          camera.targetX += forwardX * moveSpeed;
          camera.targetZ += forwardZ * moveSpeed;
        }
        if (keys["s"] || keys["arrowdown"]) {
          camera.targetX -= forwardX * moveSpeed;
          camera.targetZ -= forwardZ * moveSpeed;
        }
        if (keys["a"] || keys["arrowleft"]) {
          camera.targetX -= rightX * moveSpeed;
          camera.targetZ -= rightZ * moveSpeed;
        }
        if (keys["d"] || keys["arrowright"]) {
          camera.targetX += rightX * moveSpeed;
          camera.targetZ += rightZ * moveSpeed;
        }
        if (keys[" "]) camera.targetY += moveSpeed;
        if (keys["shift"]) camera.targetY = Math.max(0, camera.targetY - moveSpeed);
      }

      // Cinematic Tour Progression
      if (game.mode === "tour" && !game.pausedForModal) {
        game.tourProgress = (game.tourProgress + dt * game.tourSpeed) % 1;
        var totalSectors = SECTORS.length;
        var currentFloat = game.tourProgress * totalSectors;
        var sIndex = Math.floor(currentFloat);
        var nextIndex = (sIndex + 1) % totalSectors;
        var sectorFrac = currentFloat - sIndex;

        var s1 = SECTORS[sIndex];
        var s2 = SECTORS[nextIndex];

        // Smoothly glide camera focus target
        camera.targetX = s1.coords.x + (s2.coords.x - s1.coords.x) * sectorFrac;
        camera.targetZ = s1.coords.z + (s2.coords.z - s1.coords.z) * sectorFrac;
        camera.targetY = (s1.height + s2.height) * 0.5 * 0.6;

        // Cinematic rotating orbit angle
        camera.yaw = -0.9 + Math.sin(game.tourProgress * Math.PI * 2) * 0.4;
        camera.pitch = 0.42 + Math.cos(game.tourProgress * Math.PI * 2) * 0.1;
        camera.distance = 360 + Math.sin(game.tourProgress * Math.PI * 4) * 40;

        // Update active sector codex
        if (sIndex !== game.activeSector) {
          updateCodex(sIndex);
          playSound("blip");

          // Trigger Quest-specific events
          if (game.quest === "deny" && sIndex === 8) {
            playSound("deny");
            game.shockwave = { x: s1.coords.x, z: s1.coords.z, radius: 10, maxRadius: 180 };
          } else if (game.quest === "ask" && sIndex === 8) {
            playSound("ask");
            game.pausedForModal = true;
            modal.hidden = false;
          } else if (game.quest === "failover" && sIndex === 4) {
            playSound("blip");
            codexRule.innerHTML = "<b>Circuit Breaker Active:</b> Primary leg throttled (429) · Route ladder engages OpenAI GPT-4o failover.";
          }
        }
      }

      // List of 3D objects to sort by depth (Painter's Algorithm)
      var renderQueue = [];

      // 1. Draw 3D Ground Cyber-Grid
      var gridSize = 420;
      var gridStep = 42;
      for (var gx = -gridSize; gx <= gridSize; gx += gridStep) {
        var pStart = project3D(gx, 0, -gridSize, w, h);
        var pEnd = project3D(gx, 0, gridSize, w, h);
        if (pStart && pEnd) {
          ctx.strokeStyle = (gx === 0) ? "rgba(223, 121, 95, 0.45)" : "rgba(52, 52, 47, 0.35)";
          ctx.lineWidth = (gx === 0) ? 1.5 : 0.8;
          ctx.beginPath();
          ctx.moveTo(pStart.x, pStart.y);
          ctx.lineTo(pEnd.x, pEnd.y);
          ctx.stroke();
        }
      }
      for (var gz = -gridSize; gz <= gridSize; gz += gridStep) {
        var pStart2 = project3D(-gridSize, 0, gz, w, h);
        var pEnd2 = project3D(gridSize, 0, gz, w, h);
        if (pStart2 && pEnd2) {
          ctx.strokeStyle = (gz === 0) ? "rgba(223, 121, 95, 0.45)" : "rgba(52, 52, 47, 0.35)";
          ctx.lineWidth = (gz === 0) ? 1.5 : 0.8;
          ctx.beginPath();
          ctx.moveTo(pStart2.x, pStart2.y);
          ctx.lineTo(pEnd2.x, pEnd2.y);
          ctx.stroke();
        }
      }

      // 2. Draw Glowing Energy Conduits Connecting the 10 Sectors
      for (var sc = 0; sc < SECTORS.length; sc++) {
        var curSec = SECTORS[sc];
        var nextSec = SECTORS[(sc + 1) % SECTORS.length];
        var p1 = project3D(curSec.coords.x, 2, curSec.coords.z, w, h);
        var p2 = project3D(nextSec.coords.x, 2, nextSec.coords.z, w, h);
        if (p1 && p2) {
          ctx.strokeStyle = "rgba(223, 121, 95, 0.32)";
          ctx.lineWidth = 2.5;
          ctx.beginPath();
          ctx.moveTo(p1.x, p1.y);
          ctx.lineTo(p2.x, p2.y);
          ctx.stroke();

          // Pulse beam traveling conduit
          var beamFrac = (time * 0.0008 + sc * 0.1) % 1;
          var bx = p1.x + (p2.x - p1.x) * beamFrac;
          var by = p1.y + (p2.y - p1.y) * beamFrac;
          ctx.fillStyle = "#ee9278";
          ctx.beginPath();
          ctx.arc(bx, by, 3, 0, Math.PI * 2);
          ctx.fill();
        }
      }

      // 3. Build 3D Sector Megastructures
      for (var i = 0; i < SECTORS.length; i++) {
        var sec = SECTORS[i];
        var isActive = (i === game.activeSector);
        var baseClr = isActive ? "rgba(40, 36, 32, 0.95)" : "rgba(28, 28, 25, 0.92)";
        var wireClr = isActive ? "#ee9278" : sec.wireColor;

        if (sec.shape === "pyramid_spaceport") {
          var pyr = buildPyramidPolygons(sec.coords.x, 0, sec.coords.z, sec.baseSize, sec.height, baseClr, wireClr, w, h);
          renderQueue = renderQueue.concat(pyr);
        } else if (sec.shape === "obelisk_monolith") {
          var obelisk = buildBoxPolygons(sec.coords.x, 0, sec.coords.z, sec.baseSize * 0.6, sec.height, sec.baseSize * 0.6, baseClr, wireClr, w, h);
          renderQueue = renderQueue.concat(obelisk);
        } else {
          var box = buildBoxPolygons(sec.coords.x, 0, sec.coords.z, sec.baseSize, sec.height, sec.baseSize, baseClr, wireClr, w, h);
          renderQueue = renderQueue.concat(box);
        }

        // Add Hovering Beacon Rings & Monolith Crowns
        (function (s, act) {
          var beaconPos = project3D(s.coords.x, s.height + 12 + Math.sin(time * 0.003 + s.coords.x) * 4, s.coords.z, w, h);
          if (beaconPos) {
            renderQueue.push({
              depth: beaconPos.depth,
              render: function () {
                ctx.fillStyle = act ? "#ee9278" : s.color;
                ctx.beginPath();
                ctx.arc(beaconPos.x, beaconPos.y, act ? 6 : 4, 0, Math.PI * 2);
                ctx.fill();

                ctx.strokeStyle = act ? "rgba(238, 146, 120, 0.6)" : "rgba(115, 169, 130, 0.4)";
                ctx.lineWidth = 1.5;
                ctx.beginPath();
                ctx.arc(beaconPos.x, beaconPos.y, act ? 12 : 8, 0, Math.PI * 2);
                ctx.stroke();

                // Floating Sector Label
                ctx.font = (act ? "bold 11px" : "10px") + " 'SF Mono', ui-monospace, Menlo, monospace";
                ctx.fillStyle = act ? "#eeeae2" : "#8b8880";
                ctx.textAlign = "center";
                ctx.fillText("S" + s.num + " · " + s.title.split(" ")[1], beaconPos.x, beaconPos.y - 14);
              }
            });
          }
        })(sec, isActive);
      }

      // 4. Render Active In-World Player Drone
      var curSecData = SECTORS[game.activeSector];
      var droneX = curSecData.coords.x;
      var droneY = curSecData.height + 28 + Math.sin(time * 0.004) * 6;
      var droneZ = curSecData.coords.z;
      var droneProj = project3D(droneX, droneY, droneZ, w, h);

      if (droneProj) {
        renderQueue.push({
          depth: droneProj.depth,
          render: function () {
            // Scanner cone downward
            var groundProj = project3D(droneX, 0, droneZ, w, h);
            if (groundProj) {
              var grad = ctx.createLinearGradient(droneProj.x, droneProj.y, groundProj.x, groundProj.y);
              grad.addColorStop(0, "rgba(223, 121, 95, 0.45)");
              grad.addColorStop(1, "rgba(223, 121, 95, 0)");
              ctx.fillStyle = grad;
              ctx.beginPath();
              ctx.moveTo(droneProj.x, droneProj.y);
              ctx.lineTo(groundProj.x - 24, groundProj.y);
              ctx.lineTo(groundProj.x + 24, groundProj.y);
              ctx.closePath();
              ctx.fill();
            }

            // Drone Orb Core
            ctx.fillStyle = "#df795f";
            ctx.beginPath();
            ctx.arc(droneProj.x, droneProj.y, 8, 0, Math.PI * 2);
            ctx.fill();
            ctx.strokeStyle = "#eeeae2";
            ctx.lineWidth = 2;
            ctx.stroke();
          }
        });
      }

      // 5. Render Swarm Mode High-Throughput Packets
      if (game.quest === "swarm") {
        var totalS = SECTORS.length;
        for (var sw = 0; sw < game.swarmPackets.length; sw++) {
          var sp = game.swarmPackets[sw];
          sp.progress = (sp.progress + dt * sp.speed) % 1;
          var curSecIdx = Math.floor(sp.progress * totalS);
          var nextSecIdx = (curSecIdx + 1) % totalS;
          var sFrac = (sp.progress * totalS) - curSecIdx;

          var sc1 = SECTORS[curSecIdx];
          var sc2 = SECTORS[nextSecIdx];
          var px = sc1.coords.x + (sc2.coords.x - sc1.coords.x) * sFrac;
          var pz = sc1.coords.z + (sc2.coords.z - sc1.coords.z) * sFrac;
          var py = sp.yOffset;

          var swarmProj = project3D(px, py, pz, w, h);
          if (swarmProj) {
            renderQueue.push({
              depth: swarmProj.depth,
              render: function (clr, pt) {
                return function () {
                  ctx.fillStyle = clr;
                  ctx.beginPath();
                  ctx.arc(pt.x, pt.y, 3.5, 0, Math.PI * 2);
                  ctx.fill();
                };
              }(sp.color, swarmProj)
            });
          }
        }
      }

      // 6. Render Security Shockwave (on Deny attack)
      if (game.shockwave) {
        game.shockwave.radius += dt * 140;
        var swProj = project3D(game.shockwave.x, 5, game.shockwave.z, w, h);
        if (swProj && game.shockwave.radius < game.shockwave.maxRadius) {
          ctx.strokeStyle = "rgba(216, 111, 114, " + (1 - game.shockwave.radius / game.shockwave.maxRadius) + ")";
          ctx.lineWidth = 4;
          ctx.beginPath();
          ctx.arc(swProj.x, swProj.y, game.shockwave.radius * (camera.fov / swProj.depth), 0, Math.PI * 2);
          ctx.stroke();
        } else if (game.shockwave.radius >= game.shockwave.maxRadius) {
          game.shockwave = null;
        }
      }

      // Sort all 3D polygons and particles by depth (Painter's Algorithm: deepest first)
      renderQueue.sort(function (a, b) { return b.depth - a.depth; });
      for (var q = 0; q < renderQueue.length; q++) {
        renderQueue[q].render();
      }

      ctx.restore();

      // Render Tactical Radar Minimap
      renderMinimap();

      requestAnimationFrame(renderWorld);
    }

    function renderMinimap() {
      if (!miniCtx || !minimap) return;
      var mw = minimap.width;
      var mh = minimap.height;
      miniCtx.clearRect(0, 0, mw, mh);

      // Center radar grid
      miniCtx.strokeStyle = "rgba(52, 52, 47, 0.5)";
      miniCtx.lineWidth = 1;
      miniCtx.beginPath();
      miniCtx.arc(mw * 0.5, mh * 0.5, mw * 0.44, 0, Math.PI * 2);
      miniCtx.stroke();
      miniCtx.beginPath();
      miniCtx.arc(mw * 0.5, mh * 0.5, mw * 0.22, 0, Math.PI * 2);
      miniCtx.stroke();

      // Draw Sector Radar Blips
      var mapScale = 0.2;
      for (var i = 0; i < SECTORS.length; i++) {
        var s = SECTORS[i];
        var mx = mw * 0.5 + s.coords.x * mapScale;
        var my = mh * 0.5 + s.coords.z * mapScale;

        var isAct = (i === game.activeSector);
        miniCtx.fillStyle = isAct ? "#ee9278" : "#73a982";
        miniCtx.beginPath();
        miniCtx.arc(mx, my, isAct ? 3.5 : 2, 0, Math.PI * 2);
        miniCtx.fill();
      }

      // Draw Camera Frustum Cone
      var camMx = mw * 0.5 + camera.targetX * mapScale;
      var camMy = mh * 0.5 + camera.targetZ * mapScale;
      miniCtx.fillStyle = "rgba(223, 121, 95, 0.2)";
      miniCtx.beginPath();
      miniCtx.moveTo(camMx, camMy);
      var coneLen = 22;
      var coneAngle = 0.5;
      miniCtx.lineTo(camMx + Math.sin(camera.yaw - coneAngle) * coneLen, camMy + Math.cos(camera.yaw - coneAngle) * coneLen);
      miniCtx.lineTo(camMx + Math.sin(camera.yaw + coneAngle) * coneLen, camMy + Math.cos(camera.yaw + coneAngle) * coneLen);
      miniCtx.closePath();
      miniCtx.fill();
    }

    // Minimap Click-to-Warp
    if (minimap) {
      minimap.addEventListener("click", function (e) {
        var rect = minimap.getBoundingClientRect();
        var mx = e.clientX - rect.left - rect.width * 0.5;
        var my = e.clientY - rect.top - rect.height * 0.5;
        var worldX = mx / 0.2;
        var worldZ = my / 0.2;

        // Find closest sector
        var closest = 0;
        var minDist = 99999;
        for (var i = 0; i < SECTORS.length; i++) {
          var dx = SECTORS[i].coords.x - worldX;
          var dz = SECTORS[i].coords.z - worldZ;
          var d = Math.sqrt(dx * dx + dz * dz);
          if (d < minDist) {
            minDist = d;
            closest = i;
          }
        }
        initAudio();
        playSound("warp");
        warpToSector(closest);
      });
    }

    // Initialize View & Start Engine Loop
    setMode("tour");
    warpToSector(0);
    if (!reduce) {
      requestAnimationFrame(renderWorld);
    }
  }

  ready(function () {
    var M = window.Motion;
    var reduce = reduceQuery.matches;
    // Reveals first and on their own: a throw in any later feature must not
    // be able to leave the page's content at opacity 0.
    try { reveals(M, reduce); } catch (e) { $$(".reveal").forEach(function (el) { el.classList.add("in"); }); }
    try { theme(); } catch (e) {}
    try { buildStamp(); } catch (e) {}
    try { rail(M, reduce); } catch (e) {}
    try { console_(M, reduce); } catch (e) {}
    try { modes(); } catch (e) {}
    try { switcher(M, reduce); } catch (e) {}
    try { simulator(M, reduce); } catch (e) {}
    try { turnEngine(M, reduce); } catch (e) {}
    try { tour3D(M, reduce); } catch (e) {}
    try { copyButtons(); } catch (e) {}
    try { vakMark(); } catch (e) {}
  });
})();
