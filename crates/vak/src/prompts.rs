//! `vak prompts` — the terminal surface for layered prompts
//! (docs/design/45-prompt-layers.md).
//!
//! The CLI carries the full verb set rather than a read-only view, because
//! it is the only surface that works on a machine with no UI.

use std::io::Read as _;
use std::path::PathBuf;

use vak_core::Core;
use vak_core::prompts::{self, PromptBlock};

use crate::cli::{PromptScope, PromptsAction};

fn scope_root(scope: PromptScope, cwd: &std::path::Path) -> PathBuf {
    match scope {
        PromptScope::User => vak_config::paths::default_workspace(),
        PromptScope::Project => cwd.to_path_buf(),
    }
}

fn scope_label(scope: PromptScope) -> &'static str {
    match scope {
        PromptScope::User => "Shared",
        PromptScope::Project => "This project",
    }
}

fn parse_block(raw: &str) -> Result<PromptBlock, i32> {
    PromptBlock::parse(raw).ok_or_else(|| {
        eprintln!(
            "error: unknown block '{raw}'. Editable blocks: identity, operating-rules, \
             guardrails, surface-note."
        );
        eprintln!(
            "note: the capability contract, the Surface line, and the skill/MCP lists are \
             code-owned and describe the interface as it actually is. `surface-note` \
             appends to the Surface line; it cannot rewrite it."
        );
        2
    })
}

pub(crate) fn run(cwd: PathBuf, action: PromptsAction, trusted: bool) -> i32 {
    match action {
        PromptsAction::Show {
            block,
            scope,
            provenance,
        } => show(cwd, block, scope, provenance, trusted),
        PromptsAction::Edit { block, scope } => edit(cwd, block, scope),
        PromptsAction::Set { block, from, scope } => set(cwd, block, from, scope),
        PromptsAction::Reset { block, scope } => reset(cwd, block, scope),
        PromptsAction::Diff => diff(cwd, trusted),
        PromptsAction::Preview { surface, role } => preview(cwd, surface, role, trusted),
        PromptsAction::Roles => roles(cwd, trusted),
    }
}

fn core_at(cwd: PathBuf, trusted: bool) -> Result<Core, i32> {
    Core::new_with_trust(cwd, trusted).map_err(|e| {
        eprintln!("error: {e}");
        2
    })
}

fn show(
    cwd: PathBuf,
    block: Option<String>,
    scope: Option<PromptScope>,
    provenance: bool,
    trusted: bool,
) -> i32 {
    // `--scope` means "show me the layer I would be editing", which is a
    // different question from "show me what the model sees".
    if let Some(scope) = scope {
        let dir = prompts::layer_dir(&scope_root(scope, &cwd));
        let content = prompts::read_layer(&dir);
        println!("# {} — {}", scope_label(scope), dir.display());
        if content.is_empty() {
            println!("\n(nothing set here; every block is inherited)");
            return 0;
        }
        for b in PromptBlock::ALL {
            match content.block(b) {
                Some(text) => println!("\n## {}\n\n{}", b.slug(), text.trim()),
                None => println!("\n## {} — inherited", b.slug()),
            }
        }
        return 0;
    }

    let core = match core_at(cwd, trusted) {
        Ok(c) => c,
        Err(code) => return code,
    };
    let resolution = core.resolve_prompt(&core.capability_descriptors());

    if let Some(raw) = block {
        let block = match parse_block(&raw) {
            Ok(b) => b,
            Err(code) => return code,
        };
        for descriptor in resolution
            .descriptors
            .iter()
            .filter(|d| d.block == block.slug())
        {
            println!(
                "# from {} ({})",
                prompts::layer_label(&descriptor.layer),
                descriptor.source.as_deref().unwrap_or("built in")
            );
        }
        return 0;
    }

    if provenance {
        println!("# Contributing layers\n");
        for d in &resolution.descriptors {
            println!(
                "{:<16} {:<16} {:>6}B  {}",
                d.block,
                prompts::layer_label(&d.layer),
                d.bytes,
                d.source.as_deref().unwrap_or("built in")
            );
        }
        println!(
            "\n# fingerprint {}\n# ~{} tokens\n",
            &resolution.fingerprint()[..16],
            resolution.text.len() / 4
        );
    }
    println!("{}", resolution.text);
    0
}

fn edit(cwd: PathBuf, raw: String, scope: PromptScope) -> i32 {
    let block = match parse_block(&raw) {
        Ok(b) => b,
        Err(code) => return code,
    };
    let dir = prompts::layer_dir(&scope_root(scope, &cwd));
    let existing = prompts::read_layer(&dir).block(block).unwrap_or_default();

    let editor = std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .unwrap_or_else(|_| "vi".to_string());
    let tmp = std::env::temp_dir().join(format!("vak-prompt-{}.md", block.slug()));
    if let Err(e) = std::fs::write(&tmp, &existing) {
        eprintln!("error: cannot stage editor buffer: {e}");
        return 2;
    }
    let status = std::process::Command::new(&editor).arg(&tmp).status();
    match status {
        Ok(s) if s.success() => {}
        Ok(s) => {
            eprintln!("error: {editor} exited with {s}; nothing saved");
            let _ = std::fs::remove_file(&tmp);
            return 2;
        }
        Err(e) => {
            eprintln!("error: cannot run {editor}: {e}");
            let _ = std::fs::remove_file(&tmp);
            return 2;
        }
    }
    let edited = std::fs::read_to_string(&tmp).unwrap_or_default();
    let _ = std::fs::remove_file(&tmp);
    if edited.trim() == existing.trim() {
        println!("No change.");
        return 0;
    }
    save(&dir, block, &edited, scope)
}

fn set(cwd: PathBuf, raw: String, from: String, scope: PromptScope) -> i32 {
    let block = match parse_block(&raw) {
        Ok(b) => b,
        Err(code) => return code,
    };
    let text = if from == "-" {
        let mut buf = String::new();
        if let Err(e) = std::io::stdin().read_to_string(&mut buf) {
            eprintln!("error: cannot read stdin: {e}");
            return 2;
        }
        buf
    } else {
        match std::fs::read_to_string(&from) {
            Ok(text) => text,
            Err(e) => {
                eprintln!("error: cannot read {from}: {e}");
                return 2;
            }
        }
    };
    save(
        &prompts::layer_dir(&scope_root(scope, &cwd)),
        block,
        &text,
        scope,
    )
}

fn save(dir: &std::path::Path, block: PromptBlock, text: &str, scope: PromptScope) -> i32 {
    if let Err(e) = prompts::write_block(dir, block, Some(text)) {
        eprintln!("error: cannot write {}: {e}", block.slug());
        return 2;
    }
    println!(
        "Saved {} to {} ({}).",
        block.slug(),
        scope_label(scope),
        dir.join(block.file_name()).display()
    );
    if block == PromptBlock::Guardrails {
        println!(
            "note: guardrail text instructs the model; it does not enforce anything. \
             Permissions and the sandbox are the enforcement boundary."
        );
    }
    if block == PromptBlock::SurfaceNote {
        println!(
            "note: notes are appended after the generated Surface line and accumulate \
             across layers; nothing narrower can remove one."
        );
    }
    println!("Applies to new sessions; a running turn keeps the prompt it started with.");
    0
}

fn reset(cwd: PathBuf, raw: String, scope: PromptScope) -> i32 {
    let block = match parse_block(&raw) {
        Ok(b) => b,
        Err(code) => return code,
    };
    let dir = prompts::layer_dir(&scope_root(scope, &cwd));
    if let Err(e) = prompts::write_block(&dir, block, None) {
        eprintln!("error: cannot reset {}: {e}", block.slug());
        return 2;
    }
    println!(
        "Reset {} in {}; it is inherited again.",
        block.slug(),
        scope_label(scope)
    );
    0
}

fn diff(cwd: PathBuf, trusted: bool) -> i32 {
    let core = match core_at(cwd, trusted) {
        Ok(c) => c,
        Err(code) => return code,
    };
    let (seed, _, _) = prompts::seed(vak_core::APP_VERSION);
    let layers = core.prompt_layers(seed.clone());
    let mut changed = false;
    for block in PromptBlock::ALL {
        let winners: Vec<_> = layers
            .iter()
            .filter(|l| l.content.block(block).is_some())
            .collect();
        let overridden = winners
            .iter()
            .any(|l| l.layer != prompts::PromptLayer::Seed);
        if !overridden {
            continue;
        }
        changed = true;
        println!("## {}", block.slug());
        if matches!(block, PromptBlock::Guardrails | PromptBlock::SurfaceNote) {
            // These only ever get added to, so the useful diff is the
            // additions — there is no such thing as a removal here.
            for layer in &winners {
                if layer.layer == prompts::PromptLayer::Seed {
                    continue;
                }
                let items = if block == PromptBlock::Guardrails {
                    &layer.content.guardrails
                } else {
                    &layer.content.surface_notes
                };
                for rule in items {
                    println!("+ [{}] {rule}", layer.layer.label());
                }
            }
        } else {
            if let Some(winner) = winners.last() {
                let text = winner.content.block(block).unwrap_or_default();
                println!("- [shipped default]");
                println!(
                    "+ [{}] {}",
                    winner.layer.label(),
                    text.lines().next().unwrap_or("")
                );
            }
        }
        println!();
    }
    if !changed {
        println!("No local prompt changes; running the shipped default.");
    }
    0
}

fn parse_surface(raw: &str) -> vak_core::Surface {
    match raw.trim().to_ascii_lowercase().as_str() {
        "" | "unknown" => vak_core::Surface::Unknown,
        "cli" => vak_core::Surface::Cli,
        "desktop" => vak_core::Surface::Desktop,
        "server" => vak_core::Surface::Server,
        "background" => vak_core::Surface::Background,
        "subagent" => vak_core::Surface::Subagent,
        channel => vak_core::Surface::Chat {
            channel: channel.to_string(),
        },
    }
}

fn preview(cwd: PathBuf, surface: String, role: Option<String>, trusted: bool) -> i32 {
    let core = match core_at(cwd, trusted) {
        Ok(c) => c,
        Err(code) => return code,
    };
    if let Some(role) = role.as_deref()
        && !core.prompt_role_names().iter().any(|n| n == role)
    {
        eprintln!("error: no role '{role}'. Run `vak prompts roles` to list them.");
        return 2;
    }
    let core = core
        .with_surface(parse_surface(&surface))
        .with_prompt_role(role);
    println!("{}", core.system_prompt());
    0
}

fn roles(cwd: PathBuf, trusted: bool) -> i32 {
    let core = match core_at(cwd, trusted) {
        Ok(c) => c,
        Err(code) => return code,
    };
    let names = core.prompt_role_names();
    if names.is_empty() {
        println!("No agent roles defined.");
        println!(
            "Create one at .vak/prompts/agents/<name>/identity.md — a child spawned with \
             task({{role: \"<name>\"}}) runs under it."
        );
        return 0;
    }
    println!("Agent roles (usable as task({{role: \"…\"}})):\n");
    for name in names {
        println!("  {name}");
    }
    println!("\nA role narrows a child; it can never widen guardrails, capabilities, or mode.");
    0
}
