use clap::{Parser, Subcommand};
use std::path::PathBuf;
use anyhow::Result;

mod embed;
mod ingest;
mod mcp;
mod rag;
mod server;
mod vault;

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
    command: Commands,
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

    match cli.command {
        Commands::Serve { transport, port } => {
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
    }

    Ok(())
}
