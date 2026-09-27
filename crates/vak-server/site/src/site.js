(() => {
  const root = document.documentElement;
  const systemTheme = window.matchMedia("(prefers-color-scheme: dark)");
  const theme = document.getElementById("theme");
  const currentTheme = () =>
    root.dataset.theme || (systemTheme.matches ? "dark" : "light");
  const labelTheme = () =>
    theme.setAttribute(
      "aria-label",
      currentTheme() === "dark" ? "Use light theme" : "Use dark theme",
    );
  if (theme) {
    theme.hidden = false;
    labelTheme();
    theme.addEventListener("click", () => {
      root.dataset.theme = currentTheme() === "dark" ? "light" : "dark";
      try {
        localStorage.setItem("vak-theme", root.dataset.theme);
      } catch (_) {}
      labelTheme();
    });
    systemTheme.addEventListener("change", labelTheme);
  }
  const tabs = Array.from(document.querySelectorAll('[role="tab"]'));
  function selectTab(tab) {
    tabs.forEach((item) => {
      const selected = item === tab;
      item.setAttribute("aria-selected", String(selected));
      item.tabIndex = selected ? 0 : -1;
      document.getElementById(item.getAttribute("aria-controls")).hidden =
        !selected;
    });
    const panel = document.getElementById(tab.getAttribute("aria-controls"));
    if (
      !window.matchMedia("(prefers-reduced-motion: reduce)").matches &&
      window.Motion?.animate
    ) {
      window.Motion.animate(panel, { opacity: [0.5, 1] }, { duration: 0.2 });
    }
  }
  tabs.forEach((tab, index) => {
    tab.addEventListener("click", () => selectTab(tab));
    tab.addEventListener("keydown", (event) => {
      let next;
      if (event.key === "ArrowRight") next = (index + 1) % tabs.length;
      if (event.key === "ArrowLeft")
        next = (index + tabs.length - 1) % tabs.length;
      if (event.key === "Home") next = 0;
      if (event.key === "End") next = tabs.length - 1;
      if (next === undefined) return;
      event.preventDefault();
      selectTab(tabs[next]);
      tabs[next].focus();
    });
  });
  document.querySelectorAll("[data-copy]").forEach((button) => {
    button.hidden = false;
    button.addEventListener("click", async () => {
      try {
        await navigator.clipboard.writeText(
          document.getElementById(button.dataset.copy).textContent,
        );
        button.textContent = "Copied";
      } catch (_) {
        button.textContent = "Select text to copy";
      }
    });
  });
  const decision = document.getElementById("decision-result");
  const actions = document.getElementById("decision-actions");
  if (decision && actions) {
    actions.hidden = false;
    actions.addEventListener("click", (event) => {
      const button = event.target.closest("button[data-decision]");
      if (!button) return;
      const allow = button.dataset.decision === "allow";
      decision.textContent = allow
        ? "In this example, the reviewed draft replaces the original. No real file was changed."
        : "In this example, the original stays as it is. The draft is kept for later. No real file was changed.";
    });
  }
  fetch("/version")
    .then((response) => {
      if (!response.ok) throw new Error("Build information unavailable");
      return response.json();
    })
    .then((build) => {
      const stamp = document.getElementById("build-stamp");
      if (stamp && typeof build.version === "string")
        stamp.textContent = "Vakyartha " + build.version;
    })
    .catch(() => {});
})();
