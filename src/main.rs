use anyhow::Result;
use clap::{Parser, Subcommand};
use satchel_rag::{embed, ingest, mcp, rag, server, vault};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "satchel",
    about = "Portable RAG on a stick. Plug in, connect, augment.",
    version,
    long_about = None
)]
struct Cli {
    #[arg(short, long, default_value = "./vault")]
    vault: PathBuf,

    #[command(subcommand)]
    command: Option<Commands>,
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
fn ensure_default_vault(vault_path: &PathBuf) -> Result<()> {
    if vault::active_vault_path(vault_path).is_ok() {
        return Ok(());
    }
    eprintln!("[satchel] No vault found. Creating default vault...");
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
    let vault_path = cli.vault.clone();

    let command = cli.command.unwrap_or(Commands::Serve {
        transport: "http".to_string(),
        port: 7428,
    });

    match command {
        Commands::Serve { transport, port } => {
            ensure_default_vault(&vault_path)?;
            let vault_dir = vault::active_vault_path(&vault_path)?;
            let db = rag::Database::open(&vault_dir)?;
            let embedder = embed::Embedder::load(&vault_path)?;

            match transport.as_str() {
                "stdio" => mcp::stdio::serve(db, embedder).await?,
                "http" | "sse" => server::serve(db, embedder, port).await?,
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
                ingest::ingest_path(&path, &db, &embedder, &config).await?;
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
                _ => anyhow::bail!(
                    "Specify exactly one of: <path>, --prefix <p>, --type <ext>"
                ),
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
