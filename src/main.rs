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
/// 1. `<binary-dir>/vault/` if it exists — preserves the USB-stick deployment
///    pattern where the binary and vault travel together.
/// 2. Platform data directory: `~/Library/Application Support/satchel` (macOS),
///    `$XDG_DATA_HOME/satchel` or `~/.local/share/satchel` (Linux/BSD),
///    `%APPDATA%/satchel` (Windows).
/// 3. `./vault` as a final fallback if no env vars are set.
fn default_vault_path() -> PathBuf {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            let candidate = parent.join("vault");
            if candidate.exists() {
                return candidate;
            }
        }
    }
    platform_data_dir().unwrap_or_else(|| PathBuf::from("vault"))
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
