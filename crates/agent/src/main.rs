mod auth;
mod config;
mod credentials;
mod name;
mod proxy;
mod refresh;
mod tunnel;
mod ws;

use anyhow::Context;
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::sync::Arc;

use crate::config::Config;
use crate::credentials::CredentialsStore;

pub struct AppState {
    pub name: String,
    pub config: Config,
    pub http: reqwest::Client,
    pub credentials: Arc<CredentialsStore>,
}

#[derive(Parser)]
#[command(
    name = "cloudcode-agent",
    about = "Cloudcode agent: dials out to a hub via WebSocket and serves its claude OAuth credentials"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Start the agent (dial the hub and serve requests until exit).
    Serve {
        #[arg(short, long, default_value = "agent.toml")]
        config: PathBuf,
    },
    /// Generate a new shared secret. Prints the plaintext for agent.toml
    /// and the argon2id hash for hub.toml.
    GenSecret,
    /// 后台管理 agent daemon（start/stop/restart/status）
    Daemon {
        #[command(subcommand)]
        cmd: cloudcode_daemon::DaemonCmd,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,cloudcode_agent=debug".into()),
        )
        .init();

    match Cli::parse().cmd {
        Cmd::Serve { config } => serve(config).await,
        Cmd::GenSecret => gen_secret(),
        Cmd::Daemon { cmd } => cloudcode_daemon::run("agent", "agent.toml", cmd),
    }
}

async fn serve(config_path: PathBuf) -> anyhow::Result<()> {
    let config =
        Config::load(&config_path).with_context(|| format!("loading {}", config_path.display()))?;
    let credentials = Arc::new(
        CredentialsStore::load(config.claude.credentials_path.clone()).with_context(|| {
            format!(
                "loading credentials from {}",
                config.claude.credentials_path.display()
            )
        })?,
    );
    let http = reqwest::Client::builder().build()?;

    let name = config
        .agent
        .name
        .clone()
        .unwrap_or_else(name::default_agent_name);
    tracing::info!(agent = %name, "starting cloudcode-agent");

    refresh::spawn(credentials.clone(), http.clone());

    let state = Arc::new(AppState {
        name,
        config,
        http,
        credentials,
    });

    ws::run(state).await
}

fn gen_secret() -> anyhow::Result<()> {
    let secret = auth::generate_secret();
    let hash = auth::hash_secret(&secret)?;
    println!("# Plaintext secret (give to the agent host, do not commit):");
    println!("# add to agent.toml under [auth]");
    println!("[auth]");
    println!("shared_secret = \"{}\"", secret);
    println!();
    println!("# argon2id hash; give to hub admin to add under [[agents]]");
    println!("[[agents]]");
    println!("# name = \"<auto-detected on agent start, or set explicitly>\"");
    println!("shared_secret_hash = \"{}\"", hash);
    Ok(())
}
