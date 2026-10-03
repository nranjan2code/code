//! Managed release lifecycle: install, reinstall, verify, status, update,
//! uninstall (docs/design/32-release-engineering.md).
//!
//! Design rules this module holds to, each of which had a counterexample
//! in the code it replaces:
//!
//! * One prefix resolution shared by every subcommand, so an install to a
//!   custom prefix stays inspectable and removable.
//! * Every mutation is a transaction that rolls back, so a failure never
//!   leaves a half-installed prefix.
//! * The manifest records a digest per component, so `status` reports
//!   drift instead of assuming its absence.
//! * Version decisions are semantic, never lexical.
//! * A downloaded artifact is verified before it is allowed near the
//!   install root.

pub mod atomic;
pub mod bundle;
pub mod digest;
pub mod feed;
pub mod layout;
pub mod manifest;
pub mod transaction;

#[cfg(not(test))]
use std::io::{IsTerminal as _, Write as _};
use std::path::{Path, PathBuf};

use feed::{Decision, Feed};
use layout::InstallRoot;
use manifest::{Component, Manifest};
use transaction::Transaction;

/// What a release is made of. Only the CLI is required; the rest ride
/// along when the build produced them, and their absence is normal
/// rather than a defect.
struct ComponentSpec {
    name: &'static str,
    required: bool,
}

const COMPONENTS: &[ComponentSpec] = &[
    ComponentSpec {
        name: "vak",
        required: true,
    },
    ComponentSpec {
        name: "vak-desktop",
        required: false,
    },
    ComponentSpec {
        name: "vak-delivery-worker",
        required: false,
    },
];

fn confirm(prompt: &str, yes: bool) -> bool {
    #[cfg(test)]
    {
        let _ = prompt;
        yes
    }
    #[cfg(not(test))]
    {
        if yes || !std::io::stdin().is_terminal() {
            // Non-interactive without --yes is a refusal, not an assumption:
            // a script must say so explicitly before anything is replaced.
            return yes;
        }
        print!("{prompt} [y/N] ");
        let _ = std::io::stdout().flush();
        let mut line = String::new();
        let _ = std::io::stdin().read_line(&mut line);
        line.trim().eq_ignore_ascii_case("y")
    }
}

// ---------------------------------------------------------------- install

/// Install the running binary and its siblings into `prefix`.
pub fn run_install(prefix: Option<PathBuf>, force: bool) -> i32 {
    let root = InstallRoot::resolve(prefix);
    install_and_report(&root, force, None)
}

/// `source` is where the component binaries live, when that is not beside
/// the running executable — `run_reinstall` passes a staged copy, because
/// by the time it installs, the directory it was running from is gone.
fn install_and_report(root: &InstallRoot, force: bool, source: Option<&Path>) -> i32 {
    match install_into(root, force, source) {
        Ok(m) => {
            println!(
                "installed {} ({}) → {}",
                m.version,
                m.git_sha,
                root.prefix().display()
            );
            for c in &m.components {
                println!("  {:<22} {}", c.name, c.path.display());
            }
            let stale = m
                .cli_path()
                .ok()
                .map(|cli| report_stale_services(&cli))
                .unwrap_or(false);
            report_next_steps(root, stale);
            0
        }
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

/// Remove an existing install and place a fresh one. Distinct from
/// `install` because installing over a tree leaves files from the old
/// version that the new manifest does not describe.
pub fn run_reinstall(prefix: Option<PathBuf>, yes: bool) -> i32 {
    let root = InstallRoot::resolve(prefix);
    if root.is_installed()
        && !confirm(
            &format!("remove and reinstall {}?", root.prefix().display()),
            yes,
        )
    {
        println!("aborted");
        return 0;
    }

    // Take a copy of the build BEFORE clearing, when we are running from
    // inside the prefix we are about to delete.
    //
    // `vak self reinstall` is what a person on a broken install reaches
    // for, and the `vak` on their PATH is the installed one — so the
    // overwhelmingly common invocation is the one where the source of the
    // reinstall lives inside its own target. Clearing first deleted that
    // source, the reinstall then failed with "required component vak not
    // found", and the prefix was left EMPTY. A repair command that
    // destroys the thing it repairs is worse than no repair command.
    let staged = match stage_sources(&root) {
        Ok(staged) => staged,
        Err(e) => {
            eprintln!("error: {e}");
            eprintln!("nothing was removed.");
            return 1;
        }
    };

    if root.is_installed() {
        // Services keep running against the old inode until sync; the
        // data home is untouched, so this is not destructive to state.
        if let Err(e) = std::fs::remove_dir_all(root.prefix()) {
            eprintln!("error: clear {}: {e}", root.prefix().display());
            return 1;
        }
    }

    let source = staged.as_ref().map(|d| d.path().to_path_buf());
    let result = install_and_report(&root, true, source.as_deref());
    // `staged` drops here, taking the temporary copy with it.
    drop(staged);
    result
}

/// A temporary copy of the component binaries, when the running executable
/// lives inside `root`.
///
/// `None` means the build is somewhere else (a dev tree, a release
/// tarball) and can be installed from where it already is.
fn stage_sources(root: &InstallRoot) -> Result<Option<tempfile::TempDir>, String> {
    let exe = std::env::current_exe().map_err(|e| format!("cannot locate running binary: {e}"))?;
    let prefix = root
        .prefix()
        .canonicalize()
        .unwrap_or_else(|_| root.prefix().to_path_buf());
    let exe_real = exe.canonicalize().unwrap_or_else(|_| exe.clone());
    if !exe_real.starts_with(&prefix) {
        return Ok(None);
    }

    let build_dir = exe_real
        .parent()
        .ok_or_else(|| "running binary has no parent directory".to_string())?;
    let staged = tempfile::tempdir().map_err(|e| format!("stage a copy of the build: {e}"))?;
    for spec in COMPONENTS {
        let source = build_dir.join(spec.name);
        if !source.exists() {
            if spec.required {
                return Err(format!(
                    "required component {} not found at {}",
                    spec.name,
                    source.display()
                ));
            }
            continue;
        }
        let destination = staged.path().join(spec.name);
        std::fs::copy(&source, &destination)
            .map_err(|e| format!("stage {}: {e}", source.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let _ = std::fs::set_permissions(&destination, std::fs::Permissions::from_mode(0o755));
        }
    }
    Ok(Some(staged))
}

fn install_into(
    root: &InstallRoot,
    force: bool,
    source: Option<&Path>,
) -> Result<Manifest, String> {
    let exe = match source {
        Some(dir) => dir.join("vak"),
        None => {
            std::env::current_exe().map_err(|e| format!("cannot locate running binary: {e}"))?
        }
    };
    let build_dir = exe
        .parent()
        .ok_or_else(|| "running binary has no parent directory".to_string())?
        .to_path_buf();
    let build_dir = build_dir.as_path();

    if root.is_installed() && !force {
        let existing = Manifest::read(root)?;
        if existing.version == manifest::build_version()
            && existing.git_sha == manifest::build_git_sha()
        {
            return Err(format!(
                "{} is already at {} ({}) — use `--force` to reinstall the same build",
                root.prefix().display(),
                existing.version,
                existing.git_sha
            ));
        }
    }

    let bin_dir = root.bin_dir();
    let mut tx = Transaction::begin(root)?;
    let mut components = Vec::new();

    for spec in COMPONENTS {
        // The CLI is the binary we are running; siblings come from the
        // same build directory.
        let source = if spec.name == "vak" {
            exe.clone()
        } else {
            build_dir.join(spec.name)
        };
        if !source.exists() {
            if spec.required {
                return Err(format!(
                    "required component {} not found at {}",
                    spec.name,
                    source.display()
                ));
            }
            continue;
        }
        let destination = bin_dir.join(spec.name);
        tx.stage_file(spec.name, &source, destination.clone(), true)?;
        components.push(Component {
            name: spec.name.to_string(),
            path: destination,
            sha256: digest::of_file(&source)?,
            required: spec.required,
        });
    }

    // Before anything is placed. A bundle that cannot launch is not an
    // install, and refusing after the commit would leave binaries with no
    // manifest beside them — worse than either finishing or not starting.
    let desktop = components.iter().any(|c| c.name == "vak-desktop");
    let frontend = bundle::locate_frontend_assets();
    bundle::require_frontend(root, frontend.as_deref(), desktop)?;

    tx.commit()?;

    let version = manifest::build_version().to_string();
    bundle::write_metadata(root, &version, frontend.as_deref(), desktop)?;

    if let Some(feed_assets) = bundle::locate_feed_assets() {
        let resources = root.resources_dir();
        std::fs::create_dir_all(&resources)
            .map_err(|e| format!("create resources {}: {e}", resources.display()))?;
        let installed_feeds = resources.join("feeds");
        if installed_feeds.exists() {
            std::fs::remove_dir_all(&installed_feeds).map_err(|e| {
                format!(
                    "clear stale feed runtime {}: {e}",
                    installed_feeds.display()
                )
            })?;
        }
        atomic::copy_dir(&feed_assets, &installed_feeds)?;
    }

    // Record the frontend as an installed asset, not just as a side effect.
    //
    // `verify` checks the manifest and nothing else, so anything the
    // manifest does not describe is, as far as every check in this product
    // is concerned, not installed. That is how a bundle could ship with a
    // blank window and still verify clean. The digest is over the whole
    // tree because the failure that happens is the shell arriving without
    // the assets it names, and `index.html` alone would not notice.
    if desktop && root.is_bundle() {
        let resources = root.resources_dir();
        // The manifest itself lands in this directory (a bundle keeps
        // `install.json` under Contents/Resources), and it cannot be inside
        // the digest it is about to carry.
        let manifest_path = root.manifest_path();
        components.push(Component {
            name: "desktop-frontend".to_string(),
            sha256: digest::of_tree_excluding(&resources, &[manifest_path.as_path()])?,
            path: resources,
            required: true,
        });
    }

    let m = Manifest::new(version, root.prefix().to_path_buf(), components);
    m.write(root)?;

    // Verify what we just wrote rather than trusting that we wrote it.
    let defects = m.verify();
    if !defects.is_empty() {
        return Err(format!(
            "install completed but does not verify: {}",
            defects
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("; ")
        ));
    }
    Ok(m)
}

/// Directories a `vak` symlink plausibly lives in, most-preferred first.
///
/// `/usr/local/bin` is NOT the answer on Apple Silicon: Homebrew moved to
/// `/opt/homebrew/bin` there, and `/usr/local/bin` is frequently absent
/// from PATH entirely. Advising it unconditionally sent ARM Mac users to
/// create a link their shell would never find.
///
/// Shared with [`remove_dangling_cli_symlink`] so uninstall cleans up
/// exactly the places install can send someone — the two used to disagree,
/// which is how a `vak` that fails with "no such file" survived an
/// uninstall.
fn cli_link_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(home) = std::env::var_os("HOME") {
        dirs.push(PathBuf::from(home).join(".local/bin"));
    }
    dirs.push(PathBuf::from("/opt/homebrew/bin"));
    dirs.push(PathBuf::from("/usr/local/bin"));
    dirs
}

/// The best place to suggest linking the CLI: a directory that already
/// exists AND is already on this user's PATH, so the advice works when
/// followed rather than being technically true.
fn suggested_link_dir() -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    let on_path: Vec<PathBuf> = std::env::split_paths(&path).collect();
    cli_link_dirs()
        .into_iter()
        .find(|dir| dir.is_dir() && on_path.contains(dir))
}

/// Which `vak` a shell would run, relative to the installed CLI.
#[derive(Debug, PartialEq, Eq)]
enum CliOnPath {
    ThisInstall,
    /// An earlier PATH entry holds a different `vak`.
    Shadowed(PathBuf),
    Absent,
}

/// Resolve `cli`'s file name the way a shell does — the first executable
/// match in `path_var` wins — and compare canonical paths, so a symlink
/// into the install (`~/.local/bin/vak`) counts as this install. Checking
/// only whether the install's own directory is on PATH reported a linked
/// CLI as missing.
fn cli_on_path(path_var: Option<&std::ffi::OsStr>, cli: &Path) -> CliOnPath {
    let (Some(path_var), Some(name)) = (path_var, cli.file_name()) else {
        return CliOnPath::Absent;
    };
    let target = std::fs::canonicalize(cli).unwrap_or_else(|_| cli.to_path_buf());
    for dir in std::env::split_paths(path_var) {
        let candidate = dir.join(name);
        if !is_executable_file(&candidate) {
            continue;
        }
        return match std::fs::canonicalize(&candidate) {
            Ok(real) if real == target => CliOnPath::ThisInstall,
            _ => CliOnPath::Shadowed(candidate),
        };
    }
    CliOnPath::Absent
}

fn is_executable_file(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        meta.is_file() && meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        meta.is_file()
    }
}

fn report_next_steps(root: &InstallRoot, stale_services: bool) {
    let cli = root.bin_dir().join("vak");
    let resolved = cli_on_path(std::env::var_os("PATH").as_deref(), &cli);
    match &resolved {
        CliOnPath::ThisInstall => {}
        // A link placed after the shadowing entry would change nothing, so
        // only the PATH order is offered.
        CliOnPath::Shadowed(other) => {
            println!();
            println!(
                "`vak` on PATH is {}, not this install; put this one first:",
                other.display()
            );
            println!("  export PATH=\"{}:$PATH\"", root.bin_dir().display());
        }
        CliOnPath::Absent => {
            println!();
            println!("the CLI is not on PATH; either add it:");
            println!("  export PATH=\"{}:$PATH\"", root.bin_dir().display());
            println!("or link it:");
            match suggested_link_dir() {
                Some(dir) => {
                    let link = dir.join("vak");
                    // No `sudo` when the directory is already writable; asking
                    // for root to write a directory the user owns teaches people
                    // to sudo things that do not need it.
                    let sudo = if is_writable_dir(&dir) { "" } else { "sudo " };
                    println!("  {sudo}ln -sf {} {}", cli.display(), link.display());
                }
                None => {
                    println!("  sudo ln -sf {} /usr/local/bin/vak", cli.display());
                    println!("  (then make sure /usr/local/bin is on your PATH)");
                }
            }
        }
    }
    println!();
    if stale_services {
        // The fresh-machine advice is wrong and actively misleading on a
        // machine that already has services: `setup` does not restart them.
        println!("next: vak self services-sync   (restart them onto this build)");
    } else {
        println!("next: vak setup   (choose a workspace, connect a model, activate services)");
    }
}

/// Every service unit this install is responsible for, with its live state.
///
/// Shared by `self status` and the post-install report below, so the two
/// can never disagree about what "stale" means.
/// Each managed service of the installed build that is failing: its name,
/// the status it last exited with, and where its log is. Empty when
/// nothing is installed.
pub(crate) fn failing_services() -> Vec<(String, i32)> {
    let Ok(manifest) = Manifest::read(&InstallRoot::resolve(None)) else {
        return Vec::new();
    };
    let Ok(cli) = manifest.cli_path() else {
        return Vec::new();
    };
    service_rows(&cli)
        .into_iter()
        .filter_map(|row| row.failed_exit.map(|status| (row.name, status)))
        .collect()
}

fn service_rows(cli: &Path) -> Vec<vak_ops::services::ServiceRow> {
    let data_home = vak_config::paths::data_home();
    let names = vak_ops::services::default_service_names(cli);
    let mut specs: Vec<_> = vak_ops::services::resolve_specs(cli, &names)
        .into_iter()
        .flatten()
        .collect();
    specs.extend(vak_ops::services::configured_bot_service_specs(
        cli,
        &data_home,
        &vak_ops::OpsConfig::detect().base_url(),
    ));
    vak_ops::services::status_specs(
        &specs,
        &vak_ops::services::Paths::default(),
        &vak_ops::services::SystemRunner,
    )
}

/// Services still running the build this install just replaced.
///
/// `self install` swaps binaries on disk and deliberately does not restart
/// anything — one writer, and killing a running agent mid-turn in order to
/// place a file is not a trade an installer gets to make by itself.
///
/// Saying nothing, however, is worse than either. That exact sequence has
/// happened here: the app on disk was current, `self verify` was clean, the
/// install printed "next: vak setup" — and every browser surface served the
/// previous build for hours, because launchd was still running the old
/// inode and nothing anywhere said so. An install that leaves the running
/// system on the old build has not finished, and has to be the one to
/// mention it.
///
/// Returns true when something is stale, so the caller can make the closing
/// line the command that fixes it rather than the one for a fresh machine.
fn report_stale_services(cli: &Path) -> bool {
    let rows = service_rows(cli);
    let stale: Vec<_> = rows
        .iter()
        .filter(|r| r.unit_present && (!r.unit_points_at_installed || r.binary_stale))
        .collect();
    if stale.is_empty() {
        return false;
    }
    println!();
    println!(
        "{} service{} still running the build this replaced:",
        stale.len(),
        if stale.len() == 1 { " is" } else { "s are" }
    );
    for r in &stale {
        let why = if !r.unit_points_at_installed {
            "execs outside the managed prefix"
        } else {
            "still on the previous binary"
        };
        let state = match r.running_pid {
            Some(pid) => format!("pid {pid}"),
            None => "down".to_string(),
        };
        println!("  {:<28} {state} — {why}", r.name);
    }
    println!();
    println!("Until they restart, every surface they serve is the OLD build.");
    true
}

/// Whether this process could create a file in `dir`.
fn is_writable_dir(dir: &Path) -> bool {
    let probe = dir.join(".vak-write-probe");
    match std::fs::File::create(&probe) {
        Ok(_) => {
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

// ----------------------------------------------------------------- verify

/// Check an install against its manifest.
pub fn run_verify(prefix: Option<PathBuf>) -> i32 {
    let root = InstallRoot::resolve(prefix);
    let m = match Manifest::read(&root) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };
    let defects = m.verify();
    println!("prefix    {}", root.prefix().display());
    println!("version   {} ({})", m.version, m.git_sha);
    for c in &m.components {
        let mark = if defects.iter().any(|d| defect_names(d) == c.name) {
            "✗"
        } else if c.path.exists() {
            "✓"
        } else {
            "·"
        };
        println!("  {mark} {:<22} {}", c.name, c.path.display());
    }
    if defects.is_empty() {
        println!("install verifies clean");
        0
    } else {
        for d in &defects {
            eprintln!("defect: {d}");
        }
        eprintln!("repair with: vak self reinstall");
        1
    }
}

fn defect_names(d: &manifest::Defect) -> &str {
    match d {
        manifest::Defect::Missing { name, .. }
        | manifest::Defect::Corrupt { name, .. }
        | manifest::Defect::Unreadable { name, .. } => name,
    }
}

// ----------------------------------------------------------------- status

pub fn run_status(prefix: Option<PathBuf>) -> i32 {
    let root = InstallRoot::resolve(prefix);
    let build = manifest::build_version();
    println!("build     {} ({})", build, manifest::build_git_sha());

    let m = match Manifest::read(&root) {
        Ok(m) => m,
        Err(e) => {
            println!("manifest  — ({e})");
            return 1;
        }
    };
    println!(
        "prefix    {}{}",
        root.prefix().display(),
        if root.is_bundle() {
            " (app bundle)"
        } else {
            ""
        }
    );
    println!("manifest  {} installed {}", m.version, m.installed_at);

    let mut drifted = false;
    let defects = m.verify();
    for d in &defects {
        eprintln!("drift: {d}");
        drifted = true;
    }

    let cli = match m.cli_path() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("drift: {e}");
            return 1;
        }
    };
    let rows = service_rows(&cli);
    for r in &rows {
        let state = match r.running_pid {
            Some(pid) => format!("running (pid {pid})"),
            None => "down".to_string(),
        };
        // "never synced" and "synced against the wrong binary" both leave
        // unit_points_at_installed false, but they need different
        // instructions — and a fresh install is always the former.
        let flag = if !r.unit_present {
            "not registered — run `self services-sync`".to_string()
        } else if !r.unit_points_at_installed {
            "✗ execs outside the managed prefix — run `self services-sync`".to_string()
        } else if let Some(status) = r.failed_exit {
            format!(
                "✗ failing: last exited with status {status} — see its log in {}",
                vak_config::paths::logs_dir().display()
            )
        } else if r.binary_stale {
            "⚠ stale process — run `self services-sync`".to_string()
        } else {
            "✓".to_string()
        };
        println!(
            "service   {} {state} · {} · {flag}",
            r.name,
            r.unit_path.display()
        );
        if !r.unit_points_at_installed || r.binary_stale || r.failed_exit.is_some() {
            drifted = true;
        }
    }

    // Compare the manifest against the binary it describes, not against
    // whichever build happens to be running this command — otherwise a
    // dev build always reports drift against a good install.
    if let Ok(installed_version) = installed_cli_version(&cli)
        && installed_version != m.version
    {
        eprintln!(
            "drift: manifest says {} but the installed binary reports {installed_version}",
            m.version
        );
        drifted = true;
    }
    if let Some(note) = untagged_build_note(&m.version, &m.git_sha) {
        println!("note      {note}");
    }
    if m.version != build {
        println!(
            "note      running build {build} differs from the install ({}) — expected when running from a source tree",
            m.version
        );
    }

    if drifted { 1 } else { 0 }
}

/// When git can tell (this runs inside a clone that has the installed
/// commit), whether the install is the tagged release it claims to be. An
/// abandoned release once left an installed build whose version existed in
/// no tag, and nothing said so.
fn untagged_build_note(version: &str, sha: &str) -> Option<String> {
    if sha.is_empty() || sha == "unknown" {
        return None;
    }
    let git = |args: &[&str]| {
        std::process::Command::new("git")
            .args(args)
            .output()
            .ok()
            .filter(|out| out.status.success())
            .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
    };
    let commit = git(&[
        "rev-parse",
        "--verify",
        "--quiet",
        &format!("{sha}^{{commit}}"),
    ])?;
    let tagged = git(&[
        "rev-parse",
        "--verify",
        "--quiet",
        &format!("refs/tags/v{version}^{{commit}}"),
    ]);
    match tagged {
        Some(tag_commit) if tag_commit == commit => None,
        Some(_) => Some(format!(
            "installed {version} was built from {sha}, not from tag v{version}: a development build"
        )),
        None => Some(format!(
            "installed {version} has no tag v{version}: a development build, not a release"
        )),
    }
}

/// Ask the installed binary what version it is. Used only to detect
/// manifest drift; a failure here is not itself a defect.
fn installed_cli_version(cli: &Path) -> Result<String, String> {
    let out = std::process::Command::new(cli)
        .arg("--version")
        .output()
        .map_err(|e| format!("exec {}: {e}", cli.display()))?;
    let text = String::from_utf8_lossy(&out.stdout);
    text.split_whitespace()
        .find_map(|t| feed::parse_version(t).ok())
        .map(|v| v.to_string())
        .ok_or_else(|| format!("no version in `{} --version` output", cli.display()))
}

// ---------------------------------------------------------- services-sync

pub fn run_services_sync(prefix: Option<PathBuf>, names: Vec<String>) -> i32 {
    let root = InstallRoot::resolve(prefix);
    let cli = match Manifest::read(&root).and_then(|m| m.cli_path()) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    // Activation is where a durable bearer token is first needed: the
    // bridge units registered below authenticate with it.
    if let Err(e) = vak_core::gateway_token::ensure_gateway_token() {
        eprintln!("warning: could not pin the gateway token: {e}");
    }
    let data_home = vak_config::paths::data_home();
    let default_workspace = vak_config::paths::default_workspace();
    if let Err(error) = std::fs::create_dir_all(&default_workspace) {
        eprintln!(
            "error: could not create default workspace {}: {error}",
            default_workspace.display()
        );
        return 1;
    }

    // Shared skills, plugins, and the global hook table — the capabilities a
    // workspace is expected to have before anyone asks for one.
    //
    // This used to run ONLY from `vak setup` and `POST /onboarding/seed`, so
    // the documented build-and-install path (`scripts/build.sh`, which calls
    // `self install` then `services-sync`) produced a live, service-managed
    // deployment with zero skills, zero plugins, and no hook table — and
    // nothing said so. `doctor` reported "0 skills · 0 hooks" as though that
    // were a normal steady state rather than an install that never finished.
    //
    // Idempotent by construction: a skill already on disk is left alone, and
    // the hook table is only written when empty. Safe to run on every sync,
    // which is what makes it safe to put here rather than behind a flag.
    if let Err(error) = vak_core::seed::seed_shared_capabilities() {
        eprintln!("error: {error}");
        return 1;
    }

    let requested: Vec<&str> = if names.is_empty() {
        vak_ops::services::default_service_names(&cli)
    } else {
        names.iter().map(String::as_str).collect()
    };

    let outcomes = vak_ops::services::services_sync(
        &cli,
        &requested,
        &vak_ops::services::Paths::default(),
        &vak_ops::services::SystemRunner,
    );
    let mut failed = false;
    for o in outcomes {
        match o.action {
            vak_ops::services::SyncAction::Failed(e) => {
                failed = true;
                println!("✗ {}: {e}", o.name);
            }
            action => println!("✓ {}: {action:?}", o.name),
        }
    }

    // Per-bot bridge units (docs/design/34, multi-bot-per-channel) aren't in
    // the static SERVICES table above — there's no fixed count of them, so
    // they're reconciled separately against bots.json every time services
    // are synced (install, update, and manual `self services-sync` alike).
    // Without this, a bot created before the binary that first understood
    // multi-bot units would never get its unit spawned until the next admin
    // console edit touched it.
    let bot_outcomes = vak_ops::services::sync_bots(
        &cli,
        &data_home,
        &vak_ops::OpsConfig::detect().base_url(),
        &vak_ops::services::Paths::default(),
        &vak_ops::services::SystemRunner,
    );
    for o in bot_outcomes {
        match o.action {
            vak_ops::services::SyncAction::Failed(e) => {
                failed = true;
                println!("✗ {}: {e}", o.name);
            }
            action => println!("✓ {}: {action:?}", o.name),
        }
    }

    if failed { 1 } else { 0 }
}

// ----------------------------------------------------------------- update

/// Pull a newer release from a feed and replace every component at once.
///
/// The fetch uses a blocking HTTP client, and these subcommands dispatch
/// from inside the CLI's tokio runtime — constructing or dropping a
/// blocking client there panics ("Cannot drop a runtime in a context
/// where blocking is not allowed"). The whole transfer therefore runs on
/// a dedicated thread, the same confinement `update_check` uses.
pub fn run_update(prefix: Option<PathBuf>, url: &str, yes: bool, dry_run: bool) -> i32 {
    let root = InstallRoot::resolve(prefix);
    let outcome = {
        let root = root.clone();
        let url = url.to_string();
        match std::thread::spawn(move || update(&root, &url, yes, dry_run)).join() {
            Ok(r) => r,
            Err(_) => Err("update thread panicked".to_string()),
        }
    };
    match outcome {
        Ok(Some(version)) => {
            println!("updated → {version}");
            println!("reloading services…");
            run_services_sync(Some(root.prefix().to_path_buf()), Vec::new())
        }
        Ok(None) => 0,
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

fn update(
    root: &InstallRoot,
    url: &str,
    yes: bool,
    dry_run: bool,
) -> Result<Option<String>, String> {
    let mut installed = Manifest::read(root)?;

    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|e| format!("http client: {e}"))?;

    let body = client
        .get(url)
        .send()
        .and_then(reqwest::blocking::Response::error_for_status)
        .map_err(|e| format!("release feed unreachable at {url}: {e}"))?
        .bytes()
        .map_err(|e| format!("release feed body: {e}"))?;
    let feed = Feed::parse(&body)?;

    // Compare against what is installed, not against the running build.
    match feed.decide(&installed.version)? {
        Decision::UpToDate { installed, offered } => {
            println!("up to date ({installed} installed, {offered} offered)");
            if !dry_run {
                vak_core::seed::seed_shared_capabilities()
                    .map_err(|error| format!("capability update failed: {error}"))?;
            }
            return Ok(None);
        }
        Decision::Upgrade { from, to } => {
            println!("update available: {from} → {to}");
            if let Some(notes) = &feed.notes_url {
                println!("notes: {notes}");
            }
            if dry_run {
                return Ok(None);
            }
            if !confirm(&format!("update {from} → {to}?"), yes) {
                println!("aborted");
                return Ok(None);
            }
        }
    }

    let key = feed::platform_key();
    let artifacts = feed.artifacts_for(&key)?;

    // A release feed ships executables and nothing else (scripts/release.sh:
    // REQUIRED/OPTIONAL are all binaries). Inside a macOS bundle the desktop
    // app's UI is a *separate tree* under Contents/Resources, so replacing
    // `vak-desktop` here would leave a new binary driving the previous
    // version's frontend — the same binary/bundle skew that shipped a blank
    // admin console twice, and one `verify` cannot see, because the files it
    // digests did not change.
    //
    // Refusing is the honest answer. An update that knowingly produces a
    // mismatched app is worse than one that says it cannot do this.
    if root.is_bundle() && artifacts.iter().any(|a| a.name == "vak-desktop") {
        return Err(format!(
            "this install is a macOS application bundle at {}, and the release feed carries \
             executables only — updating `vak-desktop` here would leave it driving the \
             previous version's frontend.\nInstall the {} disk image instead, or update a \
             non-bundle prefix with `--prefix`.",
            root.prefix().display(),
            feed.version()?,
        ));
    }

    // Download and verify every artifact before touching the install.
    let mut tx = Transaction::begin(root)?;
    let mut next_components = Vec::new();
    for artifact in &artifacts {
        println!("  fetching {}…", artifact.name);
        let bytes = client
            .get(&artifact.url)
            .send()
            .and_then(reqwest::blocking::Response::error_for_status)
            .map_err(|e| format!("download {}: {e}", artifact.name))?
            .bytes()
            .map_err(|e| format!("read {}: {e}", artifact.name))?;
        let actual = digest::of_bytes(&bytes);
        if !digest::matches(&artifact.sha256, &actual) {
            return Err(format!(
                "{} failed integrity check — expected {}, got {actual}. Nothing was installed.",
                artifact.name,
                artifact.sha256.trim()
            ));
        }
        let destination = installed
            .component(&artifact.name)
            .map(|c| c.path.clone())
            .unwrap_or_else(|| root.bin_dir().join(&artifact.name));
        tx.stage_bytes(&artifact.name, &bytes, destination.clone(), true)?;
        next_components.push(Component {
            name: artifact.name.clone(),
            path: destination,
            sha256: actual,
            required: artifact.required,
        });
    }

    if tx.is_empty() {
        return Err(format!("release {} offers nothing for {key}", feed.version));
    }
    tx.commit()?;

    // Capability seeds live in the canonical Shared workspace rather than
    // inside the executable prefix. Reconcile them after every real update,
    // including releases that add no new binary component. The seed manifest
    // advances untouched shipped content and preserves user edits.
    vak_core::seed::seed_shared_capabilities()
        .map_err(|error| format!("capability update failed: {error}"))?;

    // Carry forward components the feed did not ship, so the manifest
    // keeps describing the whole install rather than only what moved.
    for existing in installed.components.clone() {
        if !next_components.iter().any(|c| c.name == existing.name) {
            next_components.push(existing);
        }
    }
    installed.version = feed.version()?.to_string();
    installed.installed_at = manifest::now_rfc3339();
    installed.components = next_components;
    installed.write(root)?;

    // `desktop: false` — the refusal above guarantees this update carried
    // no `vak-desktop`, so there is no frontend requirement to enforce and
    // nothing under Resources to replace. All this call still does is
    // restamp Info.plist with the new version.
    bundle::write_metadata(root, &installed.version, None, false)?;
    Ok(Some(installed.version.clone()))
}

// -------------------------------------------------------------- uninstall

pub fn run_uninstall(prefix: Option<PathBuf>, yes: bool, purge: bool) -> i32 {
    let root = InstallRoot::resolve(prefix);
    if !confirm(
        &format!(
            "stop services and remove the managed install at {}?",
            root.prefix().display()
        ),
        yes,
    ) {
        println!("aborted");
        return 0;
    }

    // Only tear down units that actually exec from THIS prefix.
    //
    // Service teardown used to run against every known unit name with the
    // real `Paths::default()`, whatever `--prefix` said — so uninstalling a
    // throwaway prefix unregistered the operator's real services. Running
    // the test suite on a machine with vak installed stopped its gateway,
    // tray, and bridges, silently. A unit that execs a different install
    // belongs to that install.
    let paths = vak_ops::services::Paths::default();
    let owned: Vec<&str> = vak_ops::services::SERVICES
        .iter()
        .map(|d| d.name)
        .filter(|name| vak_ops::services::unit_belongs_to_prefix(name, root.prefix(), &paths))
        .collect();
    if !owned.is_empty()
        && let Err(e) =
            vak_ops::services::services_uninstall(&owned, &paths, &vak_ops::services::SystemRunner)
    {
        eprintln!("warning: service teardown incomplete: {e}");
    }

    // Per-bot bridge units are generated from bots.json rather than living
    // in the static table, and are filtered the same way.
    let data_home = vak_config::paths::data_home();
    let owned_bots: Vec<String> = vak_ops::services::configured_bot_service_names_all(&data_home)
        .into_iter()
        .filter(|name| vak_ops::services::unit_belongs_to_prefix(name, root.prefix(), &paths))
        .collect();
    if !owned_bots.is_empty() {
        let refs: Vec<&str> = owned_bots.iter().map(String::as_str).collect();
        if let Err(e) =
            vak_ops::services::services_uninstall(&refs, &paths, &vak_ops::services::SystemRunner)
        {
            eprintln!("warning: bridge teardown incomplete: {e}");
        }
    }

    // Report components installed elsewhere, which removing this prefix
    // will not reach.
    if let Ok(m) = Manifest::read(&root) {
        for c in &m.components {
            if !c.path.starts_with(root.prefix()) {
                eprintln!(
                    "note: {} lives outside the prefix and was left in place: {}",
                    c.name,
                    c.path.display()
                );
            }
        }
    }

    match std::fs::remove_dir_all(root.prefix()) {
        Ok(()) => println!("removed {}", root.prefix().display()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            println!("nothing installed at {}", root.prefix().display());
        }
        Err(e) => eprintln!("warning: remove {}: {e}", root.prefix().display()),
    }

    // The symlink `report_next_steps` tells people to create. Leaving it
    // is how an uninstall leaves a dangling `vak` on PATH that fails with
    // a confusing "no such file" months later.
    remove_dangling_cli_symlink(root.prefix());

    if purge {
        return purge_state(yes);
    }
    println!();
    println!("configuration and data were kept; `--purge` removes them too");
    0
}

/// Remove any `vak` symlink that points into the prefix we just deleted.
///
/// Only ever a symlink, and only when it resolves into this prefix: a real
/// binary someone else installed there is theirs, not ours to remove.
///
/// Every directory `report_next_steps` can send someone to is checked, not
/// just `/usr/local/bin`. That single hardcoded path was the bug: on Apple
/// Silicon the link lands in `/opt/homebrew/bin`, so an uninstall left a
/// dangling `vak` on PATH that failed with "no such file or directory" —
/// which reads like a broken install rather than a removed one.
fn remove_dangling_cli_symlink(prefix: &Path) {
    for dir in cli_link_dirs() {
        let link = dir.join("vak");
        let Ok(meta) = std::fs::symlink_metadata(&link) else {
            continue;
        };
        if !meta.file_type().is_symlink() {
            continue;
        }
        let Ok(target) = std::fs::read_link(&link) else {
            continue;
        };
        if !target.starts_with(prefix) {
            continue;
        }
        match std::fs::remove_file(&link) {
            Ok(()) => println!("removed the {} symlink", link.display()),
            Err(e) => eprintln!(
                "note: {} points into the removed prefix; remove it with: sudo rm {} ({e})",
                link.display(),
                link.display()
            ),
        }
    }
}

/// Gives the owner write permission on every directory under `dir`, so a
/// frozen Review candidate (write-protected on purpose) can be removed.
/// Symlinks are not followed.
fn make_writable(dir: &std::path::Path) {
    let Ok(meta) = std::fs::symlink_metadata(dir) else {
        return;
    };
    if !meta.is_dir() {
        return;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = meta.permissions();
        permissions.set_mode(permissions.mode() | 0o700);
        let _ = std::fs::set_permissions(dir, permissions);
    }
    #[cfg(not(unix))]
    {
        let mut permissions = meta.permissions();
        permissions.set_readonly(false);
        let _ = std::fs::set_permissions(dir, permissions);
    }
    for child in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        make_writable(&child.path());
    }
}

/// Removes the Vak-owned root `root` wholesale, except the path down to
/// `keep` when `keep` lies inside it (the Shared layer under a `VAK_HOME`
/// override). A symlinked root is refused rather than followed.
fn remove_wholesale(root: &std::path::Path, keep: &std::path::Path) -> std::io::Result<()> {
    if std::fs::symlink_metadata(root).is_ok_and(|meta| meta.file_type().is_symlink()) {
        return Err(std::io::Error::other(
            "is a symlink; remove it by hand if you meant to",
        ));
    }
    if !keep.starts_with(root) {
        make_writable(root);
        return match std::fs::remove_dir_all(root) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            other => other,
        };
    }
    if keep == root {
        return Ok(());
    }
    let mut refused = Vec::new();
    for child in std::fs::read_dir(root)? {
        let path = child?.path();
        if path.is_symlink() {
            // Following it could delete a directory Vak does not own.
            refused.push(path.display().to_string());
        } else if keep.starts_with(&path) {
            remove_wholesale(&path, keep)?;
        } else if path.is_dir() {
            make_writable(&path);
            std::fs::remove_dir_all(&path)?;
        } else {
            std::fs::remove_file(&path)?;
        }
    }
    if refused.is_empty() {
        Ok(())
    } else {
        Err(std::io::Error::other(format!(
            "refused {}: a symlink; remove it by hand if you meant to",
            refused.join(", ")
        )))
    }
}

/// Everything the state registry says a purge removes.
///
/// Derived from `vak_core::state`, not from a list kept here, and framed
/// as a **preserve rule** rather than a delete list: when a new durable
/// file appears, the safe default is to leave it alone, and the registry
/// test fails the build until someone declares what should happen to it
/// (doc 46 D3, Part VII.2).
fn purge_state(yes: bool) -> i32 {
    use vak_core::state::{OnPurge, Root};

    let shared_root = vak_core::state::root_path(Root::Shared);
    let mut targets: Vec<(PathBuf, String)> = Vec::new();
    // Roots Vak owns go wholesale, whatever older layouts left in them; the
    // Shared layer keeps everything but its declared entries.
    let mut owned: Vec<PathBuf> = Vec::new();
    for root in Root::ALL {
        let base = vak_core::state::root_path(root);
        if root.is_owned() {
            if base.exists() && !owned.iter().any(|kept| base.starts_with(kept)) {
                owned.retain(|kept| !kept.starts_with(&base));
                owned.push(base);
            }
            continue;
        }
        for entry in vak_core::state::entries_for(root) {
            if entry.on_purge != OnPurge::Remove {
                continue;
            }
            // A pattern entry (`agents/{agent}/…`) names one path per Agent
            // home; `expand` lists the ones present.
            for relative in entry.expand(&base) {
                let path = vak_core::state::resolve(&base, &relative);
                targets.push((path, relative.display().to_string()));
            }
        }
    }

    if targets.is_empty() && owned.is_empty() {
        println!("nothing to purge — no declared state is present");
        return 0;
    }

    // Name what dies. A confirmation that says "delete everything?" is one
    // people answer without reading.
    println!();
    println!("--purge will permanently delete:");
    for path in &owned {
        println!("  {} (all of it)", path.display());
    }
    for (path, _) in &targets {
        println!("  {}", path.display());
    }
    println!();
    println!("this includes your Shared configuration, your stored keys, every session");
    println!("ledger, and every installed skill and plugin.");
    println!("project `.vak` directories inside your own repositories are NOT touched.");
    if !confirm("permanently delete all of that?", yes) {
        println!("kept everything; nothing was deleted");
        return 0;
    }

    let mut failed = false;
    for path in &owned {
        match remove_wholesale(path, &shared_root) {
            Ok(()) => println!("  removed {}", path.display()),
            Err(e) => {
                eprintln!("  warning: {}: {e}", path.display());
                failed = true;
            }
        }
    }
    for (path, declared) in &targets {
        // A symlinked root would make `remove_dir_all` follow the link and
        // delete somebody else's directory. Refuse rather than guess.
        match std::fs::symlink_metadata(path) {
            Ok(meta) if meta.file_type().is_symlink() => {
                eprintln!(
                    "refused: {} is a symlink; remove it by hand if you meant to",
                    path.display()
                );
                failed = true;
                continue;
            }
            _ => {}
        }
        let outcome = if path.is_dir() {
            std::fs::remove_dir_all(path)
        } else {
            std::fs::remove_file(path)
        };
        match outcome {
            Ok(()) => println!("  removed {declared}"),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                eprintln!("  warning: {}: {e}", path.display());
                failed = true;
            }
        }
    }
    for root in Root::ALL {
        vak_core::state::remove_empty_pattern_dirs(root, &vak_core::state::root_path(root));
    }

    println!();
    println!("purged. the next install is a genuine first run.");
    i32::from(failed)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn confirm_refuses_rather_than_assumes_when_not_interactive() {
        // A piped stdin with no --yes must not be read as consent.
        assert!(!confirm("do the thing?", false));
        assert!(confirm("do the thing?", true));
    }

    #[test]
    fn every_optional_component_is_declared_optional() {
        let required: Vec<&str> = COMPONENTS
            .iter()
            .filter(|c| c.required)
            .map(|c| c.name)
            .collect();
        assert_eq!(
            required,
            vec!["vak"],
            "only the CLI is required; the rest ride along when built"
        );
    }

    #[cfg(unix)]
    fn executable(path: &Path) {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::write(path, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn cli_on_path_follows_a_symlink_into_the_install() {
        let tmp = tempfile::tempdir().unwrap();
        let bin = tmp.path().join("App/Contents/MacOS");
        let link_dir = tmp.path().join("local-bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::create_dir_all(&link_dir).unwrap();
        let cli = bin.join("vak");
        executable(&cli);
        std::os::unix::fs::symlink(&cli, link_dir.join("vak")).unwrap();

        let path = std::env::join_paths([&link_dir]).unwrap();
        assert_eq!(cli_on_path(Some(&path), &cli), CliOnPath::ThisInstall);
        let path = std::env::join_paths([&bin]).unwrap();
        assert_eq!(cli_on_path(Some(&path), &cli), CliOnPath::ThisInstall);
    }

    #[cfg(unix)]
    #[test]
    fn cli_on_path_reports_an_earlier_different_vak_as_shadowing() {
        let tmp = tempfile::tempdir().unwrap();
        let bin = tmp.path().join("install");
        let stale = tmp.path().join("cargo-bin");
        let not_exec = tmp.path().join("plain");
        for dir in [&bin, &stale, &not_exec] {
            std::fs::create_dir_all(dir).unwrap();
        }
        let cli = bin.join("vak");
        executable(&cli);
        executable(&stale.join("vak"));
        std::fs::write(not_exec.join("vak"), "not a program").unwrap();

        // A non-executable `vak` is skipped, as a shell would skip it.
        let path = std::env::join_paths([&not_exec, &bin]).unwrap();
        assert_eq!(cli_on_path(Some(&path), &cli), CliOnPath::ThisInstall);
        let path = std::env::join_paths([&stale, &bin]).unwrap();
        assert_eq!(
            cli_on_path(Some(&path), &cli),
            CliOnPath::Shadowed(stale.join("vak"))
        );
    }

    #[test]
    fn cli_on_path_is_absent_without_a_match() {
        let tmp = tempfile::tempdir().unwrap();
        let cli = tmp.path().join("vak");
        let empty = tempfile::tempdir().unwrap();
        let path = std::env::join_paths([empty.path()]).unwrap();
        assert_eq!(cli_on_path(Some(&path), &cli), CliOnPath::Absent);
        assert_eq!(cli_on_path(None, &cli), CliOnPath::Absent);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod purge_tests {
    use super::remove_wholesale;

    #[test]
    fn purge_removes_owned_roots_wholesale() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("data");
        let shared = data.join("vak-home");
        for path in [
            "sessions/a/s.jsonl",
            "agents/x/old.json",
            "stray-6x-layout/z",
            "logs/vak.jsonl",
        ] {
            let file = data.join(path);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(&file, "x").unwrap();
        }
        std::fs::create_dir_all(shared.join(".vak")).unwrap();
        std::fs::write(shared.join("notes.txt"), "mine").unwrap();
        let frozen = data.join("agents/vak/sandbox/candidates/c1");
        std::fs::create_dir_all(&frozen).unwrap();
        std::fs::write(frozen.join("draft.docx"), "x").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&frozen, std::fs::Permissions::from_mode(0o555)).unwrap();
        }
        remove_wholesale(&data, &shared).unwrap();
        let left: Vec<String> = std::fs::read_dir(&data)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            left,
            vec!["vak-home".to_string()],
            "only the Shared layer survives"
        );
        assert_eq!(
            std::fs::read_to_string(shared.join("notes.txt")).unwrap(),
            "mine"
        );

        let elsewhere = dir.path().join("cache");
        std::fs::create_dir_all(elsewhere.join("fts")).unwrap();
        remove_wholesale(&elsewhere, &shared).unwrap();
        assert!(
            !elsewhere.exists(),
            "a root that holds no Shared layer goes entirely"
        );
    }
}
