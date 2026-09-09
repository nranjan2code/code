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
    try { copyButtons(); } catch (e) {}
    try { vakMark(); } catch (e) {}
  });
})();
