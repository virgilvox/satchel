use anyhow::Result;
use clap::{Parser, Subcommand};
use satchel_rag::{embed, ingest, mcp, rag, server, vault};
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(
    name = "satchel",
    about = "Portable RAG on a stick. Plug in, connect, augment.",
    version,
    long_about = None
)]
struct Cli {
    /// Vault directory. If omitted, uses `<binary-dir>/vault` when it exists
    /// (USB-stick mode), otherwise the platform data directory.
    #[arg(short, long)]
    vault: Option<PathBuf>,

    #[command(subcommand)]
    command: Option<Commands>,
}

/// Resolve where SATCHEL keeps its vault when `--vault` isn't passed.
///
/// Order:
/// 1. `vault/` next to the binary (or, on macOS, next to the `.app`
///    bundle). The USB-stick deployment pattern: binary and vault
///    travel together.
/// 2. macOS App Translocation sandbox case only: fall back to the
///    last-known-good vault path persisted on a previous run, since
///    inside the sandbox `current_exe()` cannot see the user's
///    real filesystem and the sibling vault is invisible.
/// 3. No sibling and we are NOT in a translocation sandbox: this is a
///    fresh deployment at a new location. Create a sibling vault here
///    rather than dragging in a stale "last-known" path from somewhere
///    else. A user who plugs in a second USB stick wants a fresh vault
///    on it, not a pull-back to the first stick. Skipped for
///    system-managed install paths (`/Applications`, `/usr`, `/opt`,
///    `/System`, `/bin`, `/sbin`) where dropping a `vault/` next to
///    the binary would be rude or read-only.
/// 4. Platform data directory: `~/Library/Application Support/satchel`
///    (macOS), `$XDG_DATA_HOME/satchel` or `~/.local/share/satchel`
///    (Linux/BSD), `%APPDATA%/satchel` (Windows).
/// 5. `./vault` as the very last fallback if no env vars are set.
///
/// `ensure_default_vault` (called downstream) creates the vault on
/// disk if it does not yet exist, so returning a not-yet-existing path
/// is fine; the caller materializes it.
fn default_vault_path() -> PathBuf {
    if let Ok(exe) = std::env::current_exe() {
        // 1. Sibling vault wins outright.
        if let Some(p) = vault_next_to_exe(&exe) {
            remember_vault_path(&p);
            return p;
        }
        // 2. Translocation sandbox can never see a real sibling. Fall
        //    back to the breadcrumb when present; otherwise drop to
        //    the data-dir default.
        if is_translocated(&exe) {
            eprintln!(
                "[satchel] Warning: running from macOS App Translocation sandbox\n  \
                 ({}).\n  \
                 The vault next to the .app bundle is invisible from here. \
                 Quit, run `xattr -dr com.apple.quarantine /path/to/Satchel.app`, \
                 then reopen.",
                exe.display(),
            );
            if let Some(p) = recall_vault_path() {
                eprintln!(
                    "[satchel] Using last-known vault from a previous run: {}",
                    p.display()
                );
                return p;
            }
            // No breadcrumb; fall through to data dir.
        } else {
            // 3. Fresh deployment at a real path with no sibling. Place
            //    a new vault next to the binary (or the .app on macOS).
            //    Do NOT consult the breadcrumb here: this is exactly
            //    the "I plugged in a different USB stick, I want a
            //    fresh vault" scenario, and silently pulling in a
            //    stale path would be the wrong default.
            if let Some(p) = preferred_sibling_vault_target(&exe) {
                eprintln!(
                    "[satchel] No vault found next to the binary; creating a fresh one at {}",
                    p.display()
                );
                return p;
            }
        }
    }
    // 4. Platform data dir.
    platform_data_dir().unwrap_or_else(|| PathBuf::from("vault"))
}

/// Where a sibling vault SHOULD live for `exe` when no sibling exists
/// yet. Returns None for system-managed locations where auto-creating
/// `vault/` would be inappropriate (read-only, shared, or against
/// platform conventions). The caller's `ensure_default_vault` will
/// create the directory on first use.
fn preferred_sibling_vault_target(exe: &Path) -> Option<PathBuf> {
    let parent = exe.parent()?;
    // Resolve the "deployment dir" (the dir that holds the .app on
    // macOS, the dir that holds the binary elsewhere).
    let deploy_dir: PathBuf = if parent.ends_with("Contents/MacOS") {
        // <deploy>/<App>.app/Contents/MacOS/<bin> -> <deploy>
        parent.parent()?.parent()?.parent()?.to_path_buf()
    } else {
        parent.to_path_buf()
    };

    if is_system_install_path(&deploy_dir) {
        return None;
    }
    Some(deploy_dir.join("vault"))
}

/// True when `dir` looks like a system-managed install location where
/// SATCHEL should defer to the platform data directory rather than
/// creating a sibling vault. Conservative on purpose: only paths that
/// are unambiguously system-owned across macOS / Linux / Windows.
fn is_system_install_path(dir: &Path) -> bool {
    let s = dir.to_string_lossy();
    // macOS / shared Unix
    if s.starts_with("/Applications")
        || s.starts_with("/System")
        || s.starts_with("/Library")
        || s.starts_with("/usr/")
        || s == "/usr"
        || s.starts_with("/opt/")
        || s == "/opt"
        || s.starts_with("/bin/")
        || s == "/bin"
        || s.starts_with("/sbin/")
        || s == "/sbin"
    {
        return true;
    }
    // Common per-user toolchain bin directories: `cargo install`,
    // `pip install --user`, `pipx`, `npm -g`. Auto-creating a vault
    // there clutters the user's PATH dir.
    if let Ok(home) = std::env::var("HOME") {
        let home = std::path::Path::new(&home);
        for sub in [".cargo/bin", ".local/bin", ".npm/bin", ".pyenv/shims"] {
            if dir == home.join(sub) {
                return true;
            }
        }
    }
    // Windows Program Files. `to_string_lossy` keeps the backslashes
    // verbatim on Windows, so a starts_with check is sufficient.
    if s.starts_with("C:\\Program Files") || s.starts_with("C:\\Windows") {
        return true;
    }
    false
}

/// True when `exe` lives inside macOS's App Translocation sandbox. macOS
/// applies translocation to quarantined apps the user has not moved
/// since download, copying the .app to a randomized read-only path and
/// running from there. Inside the sandbox, `current_exe()` no longer
/// points at the user's filesystem and any sibling-vault probe fails.
fn is_translocated(exe: &Path) -> bool {
    exe.components().any(|c| {
        c.as_os_str()
            .to_str()
            .is_some_and(|s| s == "AppTranslocation")
    })
}

/// Path to the breadcrumb file that records the last vault we
/// successfully opened from a sibling-of-the-binary location. Lives in
/// the platform data dir so it survives across .app reinstalls.
fn last_vault_breadcrumb() -> Option<PathBuf> {
    platform_data_dir().map(|d| d.join("last-vault.txt"))
}

/// Save the resolved vault path so a future translocated launch can
/// fall back to it. Best-effort; failure to create the breadcrumb is
/// not fatal (the user's vault still works for this run).
fn remember_vault_path(path: &Path) {
    let Some(crumb) = last_vault_breadcrumb() else {
        return;
    };
    if let Some(parent) = crumb.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(&crumb, path.to_string_lossy().as_bytes());
}

/// Read the breadcrumb. Returns Some only if the saved path still
/// exists, so a deleted vault directory does not haunt future launches.
fn recall_vault_path() -> Option<PathBuf> {
    let crumb = last_vault_breadcrumb()?;
    let raw = std::fs::read_to_string(&crumb).ok()?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(PathBuf::from(trimmed)).filter(|p| p.exists())
}

/// Probe for a `vault/` directory deployed alongside the binary.
///
/// Linux/Windows: `<binary-dir>/vault`. macOS: also check next to the
/// containing `.app` bundle, since on macOS the binary lives at
/// `Satchel.app/Contents/MacOS/satchel` — a USB-stick layout that puts
/// `Satchel.app` and `vault/` as siblings would otherwise be invisible
/// because `<binary-dir>/vault` resolves to `Contents/MacOS/vault`.
fn vault_next_to_exe(exe: &Path) -> Option<PathBuf> {
    let parent = exe.parent()?;
    let direct = parent.join("vault");
    if direct.exists() {
        return Some(direct);
    }
    if parent.ends_with("Contents/MacOS") {
        let app_sibling = parent.parent()?.parent()?.parent()?.join("vault");
        if app_sibling.exists() {
            return Some(app_sibling);
        }
    }
    None
}

fn platform_data_dir() -> Option<PathBuf> {
    if cfg!(target_os = "macos") {
        std::env::var_os("HOME")
            .map(|h| PathBuf::from(h).join("Library/Application Support/satchel"))
    } else if cfg!(target_os = "windows") {
        std::env::var_os("APPDATA")
            .map(|a| PathBuf::from(a).join("satchel"))
            .or_else(|| {
                std::env::var_os("USERPROFILE")
                    .map(|u| PathBuf::from(u).join("AppData/Roaming/satchel"))
            })
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .map(|x| PathBuf::from(x).join("satchel"))
            .or_else(|| {
                std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share/satchel"))
            })
    }
}

/// Print a hint when a plausibly-prior vault exists somewhere other than the
/// chosen location, so users coming from older defaults don't lose track of it.
fn maybe_warn_legacy_vaults(chosen: &Path) {
    let chosen_canon = chosen
        .canonicalize()
        .unwrap_or_else(|_| chosen.to_path_buf());
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.join("vault"));
    }
    if let Some(home) = std::env::var_os("HOME") {
        candidates.push(PathBuf::from(home).join("vault"));
    }
    for c in candidates {
        let canon = c.canonicalize().unwrap_or_else(|_| c.clone());
        if canon == chosen_canon {
            continue;
        }
        // Only flag if it actually looks like a SATCHEL vault.
        if c.join("satchel.toml").exists() || c.join("vaults").is_dir() {
            eprintln!(
                "[satchel] Note: existing vault at {} (not in use). Launch with --vault {} to use it.",
                c.display(),
                c.display()
            );
        }
    }
}

#[derive(Subcommand)]
enum Commands {
    /// Start the MCP server
    Serve {
        /// Transport: "stdio" or "http"
        #[arg(short, long, default_value = "stdio")]
        transport: String,

        /// Port for HTTP transport
        #[arg(short, long, default_value_t = 7428)]
        port: u16,

        /// Don't auto-open the web UI in a browser (HTTP transport only)
        #[arg(long)]
        no_browser: bool,
    },

    /// Ingest files into the active vault
    Ingest {
        /// Path to file or directory
        path: PathBuf,

        /// Watch for changes and auto-ingest
        #[arg(short, long)]
        watch: bool,

        /// Chunk size in approximate tokens
        #[arg(long, default_value_t = 512)]
        chunk_size: usize,

        /// Chunk overlap in approximate tokens
        #[arg(long, default_value_t = 64)]
        chunk_overlap: usize,
    },

    /// Manage vaults
    Vault {
        #[command(subcommand)]
        action: VaultAction,
    },

    /// Show vault statistics
    Stats,

    /// Initialize a new vault
    Init {
        /// Vault name
        name: String,
    },

    /// Print MCP client config snippet
    Config {
        /// Target: "claude-desktop", "claude-code", "cursor", "browser", "generic"
        #[arg(short, long, default_value = "claude-desktop")]
        client: String,
    },

    /// Delete documents from the active vault
    Delete {
        /// Exact source path or document ID (use --prefix or --type for bulk)
        path: Option<String>,

        /// Delete all docs whose source_path begins with this prefix
        #[arg(long)]
        prefix: Option<String>,

        /// Delete all docs of this file type (e.g. json, pdf, md)
        #[arg(long = "type")]
        file_type: Option<String>,

        /// Show what would be deleted without deleting
        #[arg(long)]
        dry_run: bool,

        /// Skip the interactive confirmation prompt
        #[arg(short, long)]
        yes: bool,
    },

    /// Wipe all documents and chunks from the active vault (schema preserved)
    Clear {
        /// Skip the interactive confirmation prompt
        #[arg(short, long)]
        yes: bool,
    },
}

#[derive(Subcommand)]
enum VaultAction {
    /// List all vaults
    List,
    /// Create a new vault
    Create { name: String },
    /// Set the active vault
    Use { name: String },
}

/// Ensure a default vault exists. Creates one if the vault directory has no vaults.
fn ensure_default_vault(vault_path: &Path) -> Result<()> {
    if vault::active_vault_path(vault_path).is_ok() {
        return Ok(());
    }
    eprintln!("[satchel] No vault found. Creating default vault...");
    std::fs::create_dir_all(vault_path)?;
    vault::create_vault(vault_path, "default")?;
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("satchel=info")),
        )
        .with_writer(std::io::stderr)
        .init();

    let cli = Cli::parse();
    let vault_path = cli.vault.clone().unwrap_or_else(default_vault_path);
    eprintln!("[satchel] Vault: {}", vault_path.display());
    maybe_warn_legacy_vaults(&vault_path);

    let command = cli.command.unwrap_or(Commands::Serve {
        transport: "http".to_string(),
        port: 7428,
        no_browser: false,
    });

    match command {
        Commands::Serve {
            transport,
            port,
            no_browser,
        } => {
            ensure_default_vault(&vault_path)?;
            let vault_dir = vault::active_vault_path(&vault_path)?;
            let db = rag::Database::open(&vault_dir)?;
            let embedder = embed::Embedder::load(&vault_path)?;

            match transport.as_str() {
                "stdio" => mcp::stdio::serve(db, embedder).await?,
                "http" | "sse" => {
                    // Auto-open the UI when running interactively. Three
                    // signals indicate "user double-clicked or ran in a
                    // terminal — show them the UI": (1) stderr is a TTY
                    // (terminal launch), (2) `__CFBundleIdentifier` is set
                    // (Finder launched a macOS .app bundle), (3) the binary
                    // path contains `.app/Contents/MacOS` (covers older
                    // macOS that doesn't set the env var). Headless / CI /
                    // pipe-stderr launches still skip auto-open.
                    use std::io::IsTerminal;
                    let in_macos_bundle = std::env::var_os("__CFBundleIdentifier").is_some()
                        || std::env::current_exe()
                            .ok()
                            .and_then(|p| p.to_str().map(|s| s.contains(".app/Contents/MacOS")))
                            .unwrap_or(false);
                    let open = !no_browser && (std::io::stderr().is_terminal() || in_macos_bundle);
                    server::serve(db, embedder, port, open, vault_path.clone()).await?
                }
                other => anyhow::bail!("Unknown transport: {other}. Use 'stdio' or 'http'."),
            }
        }

        Commands::Ingest {
            path,
            watch,
            chunk_size,
            chunk_overlap,
        } => {
            ensure_default_vault(&vault_path)?;
            let vault_dir = vault::active_vault_path(&vault_path)?;
            let db = rag::Database::open(&vault_dir)?;
            let embedder = embed::Embedder::load(&vault_path)?;
            let config = ingest::IngestConfig {
                chunk_size,
                chunk_overlap,
            };

            if watch {
                ingest::watch_and_ingest(&path, &db, &embedder, &config).await?;
            } else {
                let progress = ingest::Progress::noop();
                ingest::ingest_path(&path, &db, &embedder, &config, &progress)?;
            }
        }

        Commands::Vault { action } => match action {
            VaultAction::List => vault::list_vaults(&vault_path)?,
            VaultAction::Create { name } => vault::create_vault(&vault_path, &name)?,
            VaultAction::Use { name } => vault::set_active(&vault_path, &name)?,
        },

        Commands::Stats => {
            ensure_default_vault(&vault_path)?;
            let vault_dir = vault::active_vault_path(&vault_path)?;
            let db = rag::Database::open(&vault_dir)?;
            rag::print_stats(&db)?;
        }

        Commands::Init { name } => {
            vault::create_vault(&vault_path, &name)?;
        }

        Commands::Config { client } => {
            mcp::print_client_config(&client, &vault_path)?;
        }

        Commands::Delete {
            path,
            prefix,
            file_type,
            dry_run,
            yes,
        } => {
            ensure_default_vault(&vault_path)?;
            let vault_dir = vault::active_vault_path(&vault_path)?;
            let db = rag::Database::open(&vault_dir)?;

            enum Mode {
                Exact(String),
                Prefix(String),
                Type(String),
            }
            let mode = match (path, prefix, file_type) {
                (Some(p), None, None) => Mode::Exact(p),
                (None, Some(pre), None) => Mode::Prefix(pre),
                (None, None, Some(t)) => Mode::Type(t),
                _ => anyhow::bail!("Specify exactly one of: <path>, --prefix <p>, --type <ext>"),
            };

            let do_op = |dry: bool| -> Result<(usize, usize)> {
                match &mode {
                    Mode::Exact(p) => db.delete_by_path_exact(p, dry),
                    Mode::Prefix(p) => db.delete_by_path_prefix(p, dry),
                    Mode::Type(t) => db.delete_by_file_type(t, dry),
                }
            };
            let specifier = match &mode {
                Mode::Exact(p) => format!("path = {p}"),
                Mode::Prefix(p) => format!("prefix = {p}*"),
                Mode::Type(t) => format!("type = .{t}"),
            };

            let (docs, chunks) = do_op(true)?;
            if docs == 0 {
                println!("[satchel] Nothing matches {specifier}");
                return Ok(());
            }

            println!("[satchel] {specifier} → {docs} documents, {chunks} chunks");

            if dry_run {
                println!("[satchel] (--dry-run) no changes made");
                return Ok(());
            }

            if !yes && !confirm(&format!("Delete {docs} documents? [y/N] "))? {
                println!("[satchel] Cancelled");
                return Ok(());
            }

            let (d, c) = do_op(false)?;
            println!("[satchel] Deleted {d} documents, {c} chunks");
        }

        Commands::Clear { yes } => {
            ensure_default_vault(&vault_path)?;
            let vault_dir = vault::active_vault_path(&vault_path)?;
            let db = rag::Database::open(&vault_dir)?;

            let (docs, chunks) = db.clear_all(true)?;
            if docs == 0 {
                println!("[satchel] Vault is already empty");
                return Ok(());
            }
            println!("[satchel] About to wipe {docs} documents and {chunks} chunks");
            if !yes && !confirm("Type 'wipe' to confirm: ")? {
                println!("[satchel] Cancelled");
                return Ok(());
            }
            let (d, c) = db.clear_all(false)?;
            println!("[satchel] Cleared {d} documents, {c} chunks");
        }
    }

    Ok(())
}

fn confirm(prompt: &str) -> Result<bool> {
    use std::io::Write;
    print!("{prompt}");
    std::io::stdout().flush()?;
    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    let trimmed = input.trim().to_lowercase();
    Ok(trimmed == "y" || trimmed == "yes" || trimmed == "wipe")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn vault_next_to_raw_binary_is_found() {
        let tmp = TempDir::new().unwrap();
        let exe = tmp.path().join("satchel");
        std::fs::write(&exe, b"").unwrap();
        let vault = tmp.path().join("vault");
        std::fs::create_dir(&vault).unwrap();
        assert_eq!(vault_next_to_exe(&exe), Some(vault));
    }

    #[test]
    fn vault_missing_returns_none() {
        let tmp = TempDir::new().unwrap();
        let exe = tmp.path().join("satchel");
        std::fs::write(&exe, b"").unwrap();
        assert_eq!(vault_next_to_exe(&exe), None);
    }

    #[test]
    fn vault_sibling_to_app_bundle_is_found() {
        // /tmp/x/Satchel.app/Contents/MacOS/satchel + /tmp/x/vault
        let tmp = TempDir::new().unwrap();
        let macos_dir = tmp.path().join("Satchel.app/Contents/MacOS");
        std::fs::create_dir_all(&macos_dir).unwrap();
        let exe = macos_dir.join("satchel");
        std::fs::write(&exe, b"").unwrap();
        let vault = tmp.path().join("vault");
        std::fs::create_dir(&vault).unwrap();
        assert_eq!(vault_next_to_exe(&exe), Some(vault));
    }

    #[test]
    fn translocation_path_is_detected() {
        // macOS sandbox path shape: /private/var/folders/.../AppTranslocation/<UUID>/d/Satchel.app/Contents/MacOS/satchel
        let exe = PathBuf::from(
            "/private/var/folders/pw/abc/T/AppTranslocation/EB180DAE-BC9E/d/Satchel.app/Contents/MacOS/satchel",
        );
        assert!(is_translocated(&exe));
    }

    #[test]
    fn normal_path_is_not_translocated() {
        let exe = PathBuf::from("/Applications/Satchel.app/Contents/MacOS/satchel");
        assert!(!is_translocated(&exe));
        let exe2 = PathBuf::from("/usr/local/bin/satchel");
        assert!(!is_translocated(&exe2));
    }

    #[test]
    fn direct_vault_inside_bundle_takes_precedence() {
        // If somebody truly puts a `vault/` inside Contents/MacOS/, that
        // wins over the .app sibling — first match in the probe order.
        let tmp = TempDir::new().unwrap();
        let macos_dir = tmp.path().join("Satchel.app/Contents/MacOS");
        std::fs::create_dir_all(&macos_dir).unwrap();
        let exe = macos_dir.join("satchel");
        std::fs::write(&exe, b"").unwrap();
        let inner_vault = macos_dir.join("vault");
        std::fs::create_dir(&inner_vault).unwrap();
        let outer_vault = tmp.path().join("vault");
        std::fs::create_dir(&outer_vault).unwrap();
        assert_eq!(vault_next_to_exe(&exe), Some(inner_vault));
    }

    #[test]
    fn preferred_sibling_target_for_app_bundle() {
        // macOS: Satchel.app under a non-system path produces the
        // sibling vault target right next to the .app.
        let exe = PathBuf::from("/Volumes/USB/Satchel.app/Contents/MacOS/satchel");
        assert_eq!(
            preferred_sibling_vault_target(&exe),
            Some(PathBuf::from("/Volumes/USB/vault"))
        );
    }

    #[test]
    fn preferred_sibling_target_for_raw_binary() {
        // Linux/Windows: returns parent-of-binary + /vault.
        let exe = PathBuf::from("/home/user/satchel-portable/satchel");
        assert_eq!(
            preferred_sibling_vault_target(&exe),
            Some(PathBuf::from("/home/user/satchel-portable/vault"))
        );
    }

    #[test]
    fn preferred_sibling_target_skips_applications() {
        // /Applications is system-managed; we should not auto-drop a
        // `vault/` next to it.
        let exe = PathBuf::from("/Applications/Satchel.app/Contents/MacOS/satchel");
        assert_eq!(preferred_sibling_vault_target(&exe), None);
    }

    #[test]
    fn preferred_sibling_target_skips_usr_local_bin() {
        let exe = PathBuf::from("/usr/local/bin/satchel");
        assert_eq!(preferred_sibling_vault_target(&exe), None);
    }

    #[test]
    fn is_system_install_path_basic_cases() {
        assert!(is_system_install_path(&PathBuf::from("/Applications")));
        assert!(is_system_install_path(&PathBuf::from(
            "/Applications/Satchel.app"
        )));
        assert!(is_system_install_path(&PathBuf::from("/usr/local/bin")));
        assert!(is_system_install_path(&PathBuf::from("/usr")));
        assert!(is_system_install_path(&PathBuf::from("/System/Library")));
        assert!(is_system_install_path(&PathBuf::from("/opt/homebrew/bin")));

        // Real deployment locations (USB stick, project dir, Downloads) are
        // NOT system locations.
        assert!(!is_system_install_path(&PathBuf::from("/Volumes/USB")));
        assert!(!is_system_install_path(&PathBuf::from(
            "/Users/alice/Projects/satchel"
        )));
        assert!(!is_system_install_path(&PathBuf::from(
            "/Users/alice/Downloads"
        )));
    }
}
