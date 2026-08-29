use std::path::{Path, PathBuf};

use vak_plugin::{InstallOptions, InstallScope, PluginStore};

use crate::cli::{PluginAction, PluginScopeArg};

pub(crate) fn run_plugins(cwd: PathBuf, action: PluginAction) -> i32 {
    match action {
        PluginAction::Inspect { path, json } => match vak_plugin::inspect_package(&path) {
            Ok(inspection) => print_value(&inspection, json, |value| {
                println!(
                    "{} {}  {}  {} files / {} bytes",
                    value.manifest.name,
                    value.manifest.version,
                    value.digest,
                    value.file_count,
                    value.total_bytes
                );
                print_capabilities(&value.capabilities);
                for warning in &value.warnings {
                    println!("warning: {warning}");
                }
            }),
            Err(error) => fail(error),
        },
        PluginAction::Install {
            path,
            scope,
            allow_unlicensed,
            json,
        } => {
            let store = store(&cwd, scope);
            let options = InstallOptions {
                scope: install_scope(scope),
                allow_unlicensed,
            };
            match store.install_local(&path, options) {
                Ok(plugin) => print_value(&plugin, json, |value| {
                    println!(
                        "installed {} {} disabled\ndigest: {}\ntrace: {}",
                        value.name, value.version, value.digest, value.trace_id
                    );
                    print_capabilities(&value.capabilities);
                }),
                Err(error) => fail(error),
            }
        }
        PluginAction::List { scope, json } => match store(&cwd, scope).list() {
            Ok(plugins) => print_value(&plugins, json, |values| {
                if values.is_empty() {
                    println!("no plugins installed for {} scope", scope_name(scope));
                }
                for plugin in values {
                    let state = if plugin.enabled {
                        "enabled"
                    } else {
                        "disabled"
                    };
                    println!(
                        "{} {}  {}  {}  {}",
                        plugin.name,
                        plugin.version,
                        state,
                        plugin.format_name(),
                        plugin.digest
                    );
                }
            }),
            Err(error) => fail(error),
        },
        PluginAction::Audit { scope, json } => match store(&cwd, scope).load() {
            Ok(registry) => print_value(&registry.audit, json, |events| {
                if events.is_empty() {
                    println!("no plugin audit events for {} scope", scope_name(scope));
                }
                for event in events {
                    println!(
                        "{}  {:?}  {} {}  {}",
                        event.generation, event.action, event.plugin, event.version, event.trace_id
                    );
                }
            }),
            Err(error) => fail(error),
        },
        PluginAction::Remove { name, scope, json } => match store(&cwd, scope).remove(&name) {
            Ok(plugin) => print_value(&plugin, json, |value| {
                println!(
                    "removed {} {} ({})",
                    value.name, value.version, value.digest
                );
            }),
            Err(error) => fail(error),
        },
    }
}

fn store(cwd: &Path, scope: PluginScopeArg) -> PluginStore {
    let home = match scope {
        PluginScopeArg::User => vak_config::paths::data_home(),
        PluginScopeArg::Workspace => cwd.join(".vak"),
    };
    PluginStore::new(home)
}

fn install_scope(scope: PluginScopeArg) -> InstallScope {
    match scope {
        PluginScopeArg::User => InstallScope::User,
        PluginScopeArg::Workspace => InstallScope::Workspace,
    }
}

fn scope_name(scope: PluginScopeArg) -> &'static str {
    match scope {
        PluginScopeArg::User => "user",
        PluginScopeArg::Workspace => "workspace",
    }
}

fn print_value<T: serde::Serialize>(value: &T, json: bool, text: impl FnOnce(&T)) -> i32 {
    if json {
        match serde_json::to_string_pretty(value) {
            Ok(output) => println!("{output}"),
            Err(error) => {
                eprintln!("error: could not serialize plugin result: {error}");
                return 1;
            }
        }
    } else {
        text(value);
    }
    0
}

fn print_capabilities(value: &vak_plugin::CapabilityInventory) {
    println!(
        "components: {} skills, {} commands, {} MCP, {} hooks, {} agents, {} LSP",
        value.skills.len(),
        value.commands.len(),
        value.mcp_manifests.len(),
        value.hooks.len(),
        value.agents.len(),
        value.lsp_manifests.len()
    );
    println!(
        "local execution: {} scripts, {} executable files",
        value.scripts.len(),
        value.executables.len()
    );
}

fn fail(error: vak_plugin::PluginError) -> i32 {
    eprintln!("error: {error}");
    1
}

trait ManifestFormatName {
    fn format_name(&self) -> &'static str;
}

impl ManifestFormatName for vak_plugin::InstalledPlugin {
    fn format_name(&self) -> &'static str {
        match self.format {
            vak_plugin::ManifestFormat::Vak => "vak",
            vak_plugin::ManifestFormat::Codex => "codex",
            vak_plugin::ManifestFormat::AgentPlugin => "agent-plugin",
            vak_plugin::ManifestFormat::Claude => "claude",
            vak_plugin::ManifestFormat::Copilot => "copilot",
            vak_plugin::ManifestFormat::Cursor => "cursor",
            vak_plugin::ManifestFormat::Gemini => "gemini",
            vak_plugin::ManifestFormat::AgentSkill => "agent-skill",
        }
    }
}
