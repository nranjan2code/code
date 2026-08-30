use std::path::{Path, PathBuf};

use vak_plugin::{InstallOptions, InstallScope, PluginStore};

use crate::cli::{MarketplaceTrustArg, PluginAction, PluginScopeArg};

pub(crate) fn run_plugins(cwd: PathBuf, action: PluginAction) -> i32 {
    match action {
        PluginAction::CatalogInspect { path, json } => match vak_plugin::inspect_catalog(&path) {
            Ok(catalog) => print_value(&catalog, json, |value| {
                println!(
                    "{}  {:?}  {} entries\ndigest: {}\ntrace: {}",
                    value.name,
                    value.format,
                    value.entries.len(),
                    value.digest,
                    value.trace_id
                );
                for warning in &value.warnings {
                    println!("warning: {warning}");
                }
            }),
            Err(error) => fail(error),
        },
        PluginAction::CatalogRegister {
            path,
            label,
            trust,
            scope,
            json,
        } => {
            let trust = match trust {
                MarketplaceTrustArg::ManualReview => vak_plugin::MarketplaceTrust::ManualReview,
                MarketplaceTrustArg::PinnedCommit => vak_plugin::MarketplaceTrust::PinnedCommit,
                MarketplaceTrustArg::LocalOnly => vak_plugin::MarketplaceTrust::LocalOnly,
            };
            match store(&cwd, scope).register_catalog_source(&path, &label, trust) {
                Ok(source) => print_value(&source, json, |value| {
                    println!(
                        "registered {} {:?} disabled\ntrace: {}",
                        value.label, value.format, value.trace_id
                    )
                }),
                Err(error) => fail(error),
            }
        }
        PluginAction::CatalogSources { scope, json } => match store(&cwd, scope).list_sources() {
            Ok(sources) => print_value(&sources, json, |values| {
                for source in values {
                    println!(
                        "{} {} {}",
                        source.label,
                        if source.enabled {
                            "enabled"
                        } else {
                            "disabled"
                        },
                        source.trace_id
                    );
                }
            }),
            Err(error) => fail(error),
        },
        PluginAction::CatalogInstall {
            catalog,
            name,
            scope,
            allow_unlicensed,
            json,
        } => {
            let store = store(&cwd, scope);
            let inspection = match vak_plugin::inspect_catalog(&catalog) {
                Ok(value) => value,
                Err(error) => return fail(error),
            };
            let entry = match inspection.entries.iter().find(|entry| entry.name == name) {
                Some(entry) => entry,
                None => return fail(vak_plugin::PluginError::NotInstalled(name)),
            };
            let staging = store
                .packages_root()
                .join("catalog-staging")
                .join(&entry.name);
            let package = match vak_plugin::materialize_catalog_entry(&catalog, entry, &staging) {
                Ok(path) => path,
                Err(error) => return fail(error),
            };
            let result = store.install_local(
                &package,
                InstallOptions {
                    scope: install_scope(scope),
                    allow_unlicensed,
                },
            );
            let _ = std::fs::remove_dir_all(&staging);
            match result {
                Ok(plugin) => print_value(&plugin, json, |value| {
                    println!(
                        "installed catalog entry {} {} disabled\ntrace: {}",
                        value.name, value.version, value.trace_id
                    );
                }),
                Err(error) => fail(error),
            }
        }
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
        PluginAction::Update {
            path,
            scope,
            allow_unlicensed,
            json,
        } => match store(&cwd, scope).update_local(
            &path,
            InstallOptions {
                scope: install_scope(scope),
                allow_unlicensed,
            },
        ) {
            Ok(plugin) => print_value(&plugin, json, |value| {
                println!(
                    "staged update {} {} disabled\ndigest: {}\ntrace: {}",
                    value.name, value.version, value.digest, value.trace_id
                );
            }),
            Err(error) => fail(error),
        },
        PluginAction::Enable { name, scope, json } => match store(&cwd, scope).enable(&name) {
            Ok(plugin) => print_value(&plugin, json, |value| {
                println!(
                    "enabled {} {} ({})",
                    value.name, value.version, value.digest
                );
            }),
            Err(error) => fail(error),
        },
        PluginAction::Disable { name, scope, json } => match store(&cwd, scope).disable(&name) {
            Ok(plugin) => print_value(&plugin, json, |value| {
                println!(
                    "disabled {} {} ({})",
                    value.name, value.version, value.digest
                );
            }),
            Err(error) => fail(error),
        },
        PluginAction::Rollback { name, scope, json } => match store(&cwd, scope).rollback(&name) {
            Ok(plugin) => print_value(&plugin, json, |value| {
                println!(
                    "rolled back {} {} disabled ({})",
                    value.name, value.version, value.digest
                );
            }),
            Err(error) => fail(error),
        },
        PluginAction::Versions { name, scope, json } => match store(&cwd, scope).versions(&name) {
            Ok(plugins) => print_value(&plugins, json, |values| {
                for value in values {
                    let state = if value.enabled { "enabled" } else { "disabled" };
                    println!(
                        "{} {} {} {}",
                        value.name, value.version, state, value.digest
                    );
                }
            }),
            Err(error) => fail(error),
        },
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
