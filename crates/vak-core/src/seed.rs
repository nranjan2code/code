//! Shared capability seeds, applied by **setup** and never by install.
//!
//! Lives in `vak-core` because every surface that can run setup needs it:
//! the CLI (`vak setup seed`) and the web wizard (`POST /onboarding/seed`)
//! must install the same seeds the same way, and a copy in the binary
//! crate could only ever serve one of them.
//!
//! Placing binaries used to seed skills, plugins, and a disabled hook as a
//! side effect (`docs/design/46-stabilization-install-and-onboarding.md`
//! D6). That had two defects beyond the contract violation: it only ran
//! when the prefix happened to equal the platform default, so any
//! `--prefix` install silently got nothing; and it never ran on update, so
//! a seed shipped in a release reached nobody who upgraded. Setup owns it
//! now, against the workspace the operator actually chose.

use std::path::Path;

use vak_config::HookConfig;
use vak_plugin::{InstallOptions, InstallScope, PluginStore};

const SKILLS: &[(&str, &str, &str)] = &[
    (
        "getting-started",
        "Explain tasks clearly and help a new user choose the simplest next step.",
        "Translate jargon into plain language. Ask only for information that is genuinely needed, then present a short, actionable next step before optional detail.",
    ),
    (
        "research-and-sources",
        "Research a question with traceable sources and clearly separated evidence and inference.",
        "Define the question and freshness requirement, prefer primary sources, record publication dates, and distinguish sourced facts from your own synthesis. Never present an unverified assumption as a citation.",
    ),
    (
        "planning-and-organizing",
        "Turn goals into practical plans, checklists, and prioritised next actions.",
        "Clarify the desired outcome, identify dependencies and decisions, then produce a plan sized to the work. Keep ownership, deadlines, and open questions explicit.",
    ),
    (
        "debugging",
        "Diagnose failures from evidence before proposing or applying a fix.",
        "Reproduce or isolate the failure, capture the first meaningful error, trace inputs to the failing boundary, and test the smallest fix. Separate confirmed cause from hypotheses.",
    ),
    (
        "code-review",
        "Review code for correctness, security, regressions, and maintainability with actionable findings.",
        "Read the diff in context, prioritise concrete defects over style preferences, include impact and a precise location, and say when a concern is unverified rather than overstating it.",
    ),
    (
        "data-and-spreadsheets",
        "Clean, analyse, and explain tabular data without silently changing its meaning.",
        "Inspect headers, types, missing values, and units before transforming data. Keep source data intact, make calculations reproducible, and label estimates, exclusions, and assumptions.",
    ),
];

const PLUGINS: &[(&str, &str, &str, &str)] = &[
    (
        "developer-starter",
        "1.0.0",
        "Core developer workflows for implementation, debugging, and code review.",
        "software-development",
    ),
    (
        "everyday-starter",
        "1.0.0",
        "Plain-language writing, planning, research, and data help for everyday work.",
        "writing-and-editing",
    ),
    (
        "python-sandbox",
        "1.0.1",
        "Sandboxed Python execution runtime for data analytics, scientific scripts, and automation.",
        "python-execution",
    ),
    (
        "react-sandbox",
        "1.0.1",
        "Sandboxed React 18 and Tailwind UI preview compiler and interactive renderer.",
        "react-component-preview",
    ),
];

const PLUGIN_SKILLS: &[(&str, &str, &str)] = &[
    (
        "software-development",
        "Implement and explain software changes with focused verification and clear tradeoffs.",
        "Inspect the existing conventions first. Make the smallest coherent change, preserve public contracts, add targeted tests for changed behavior, and report exactly what was verified.",
    ),
    (
        "writing-and-editing",
        "Draft, rewrite, summarize, and polish documents while preserving the requested voice.",
        "First identify audience, purpose, and format. Preserve facts and explicit constraints, make the smallest useful edit, and call out material ambiguities instead of inventing details.",
    ),
    (
        "python-execution",
        "Execute Python computation, data analysis, and Matplotlib plotting in the sandbox using the `python_eval` tool.",
        "Always use the `python_eval` tool to execute Python code in the sandbox. Follow the write -> execute -> debug -> result workflow: execute your scripts, read stdout/stderr, inspect any traceback to fix errors, and verify the output. Generated figures (Matplotlib) and data files are automatically saved to `.vak/scratch/python/` (accessible via `os.environ['VAK_SCRATCH_DIR']`). Pre-installed packages (matplotlib, pandas, numpy) are ready to use. Connect with React by outputting data (JSON/CSV) that can be visualized in UI previews.",
    ),
    (
        "react-component-preview",
        "Compile and preview interactive React 18 and Tailwind UI components in the sandbox using the `react_preview` tool.",
        "Always use the `react_preview` tool to render interactive UI components, dashboards, and visualizers. Write components using standard React hooks (`useState`, `useEffect`), Tailwind CSS classes, and Lucide icons. Previews are compiled into `.vak/scratch/previews/` and render live in the Desktop Preview dock and in-stream cards. Connect with Python by receiving data via `props` or loading data generated by Python in `.vak/scratch/`. If a render or syntax issue occurs, inspect the error output and adjust the component.",
    ),
];

pub fn seed_shared_capabilities() {
    let root = vak_config::paths::default_workspace().join(".vak");
    if let Err(error) = seed_skills(&root.join("skills")) {
        eprintln!("warning: Shared skill seed failed: {error}");
    }
    if let Err(error) = seed_plugins(&root) {
        eprintln!("warning: Shared plugin seed failed: {error}");
    }
    let hooks = [HookConfig {
        event: "session_start".into(),
        matcher: None,
        command: "/usr/bin/true".into(),
        timeout_ms: Some(1_000),
        enabled: false,
        failure_mode: Some("open".into()),
    }];
    if let Err(error) = vak_config::seed_global_hooks_if_empty(&hooks) {
        eprintln!("warning: Shared automation seed failed: {error}");
    }
    if let Err(error) = vak_config::seed_global_plugins_network_allow_if_empty() {
        eprintln!("warning: Shared plugin network seed failed: {error}");
    }
}

fn seed_skills(root: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(root)?;
    for (name, description, body) in PLUGIN_SKILLS {
        let path = root.join(name).join("SKILL.md");
        let expected = format!("---\nname: {name}\ndescription: {description}\n---\n\n{body}\n");
        if path.is_file()
            && matches!(std::fs::read_to_string(&path), Ok(content) if content == expected)
        {
            std::fs::remove_file(&path)?;
            let _ = std::fs::remove_dir(path.parent().unwrap_or(root));
        }
    }
    for (name, description, body) in SKILLS {
        let dir = root.join(name);
        std::fs::create_dir_all(&dir)?;
        let path = dir.join("SKILL.md");
        if path.exists() {
            continue;
        }
        let content = format!("---\nname: {name}\ndescription: {description}\n---\n\n{body}\n");
        std::fs::write(path, content)?;
    }
    Ok(())
}

fn seed_plugins(root: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let store = PluginStore::new(root);
    // Stage inside the plugin root rather than the system temp dir: same
    // filesystem as the destination, so installing is a rename and never a
    // cross-device copy — the same reason the installer stages inside its
    // own prefix.
    let staging = root.join(".seed-staging");
    let _ = std::fs::remove_dir_all(&staging);
    for (name, version, description, skill_name) in PLUGINS {
        let package = staging.join(name);
        let skill_path = package.join(format!("skills/{skill_name}"));
        std::fs::create_dir_all(&skill_path)?;
        std::fs::write(
            package.join("vak-plugin.json"),
            format!(
                r#"{{"schema":1,"name":"{name}","version":"{version}","description":"{description}","license":"MIT","components":{{"skills":["skills"]}}}}"#
            ),
        )?;
        let (skill_description, body) = PLUGIN_SKILLS
            .iter()
            .find(|(candidate, _, _)| candidate == skill_name)
            .map(|(_, description, body)| (*description, *body))
            .ok_or_else(|| format!("missing seed skill {skill_name}"))?;
        std::fs::write(
            skill_path.join("SKILL.md"),
            format!("---\nname: {skill_name}\ndescription: {skill_description}\n---\n\n{body}\n"),
        )?;
        if let Some(existing) = store
            .list()?
            .into_iter()
            .find(|plugin| plugin.name == *name)
            && existing.capabilities.skills.is_empty()
        {
            let _ = store.remove(name)?;
        }
        let installed = store.install_local(
            &package,
            InstallOptions {
                scope: InstallScope::User,
                allow_unlicensed: false,
            },
        )?;
        if !installed.enabled {
            let _ = store.enable(name)?;
        }
    }
    let _ = std::fs::remove_dir_all(&staging);
    Ok(())
}
