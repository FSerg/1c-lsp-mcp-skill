mod api;
mod mcp;
mod runtime;
mod service;
mod web;

use anyhow::Result;
use clap::{Parser, Subcommand};

use lsp_skill_core::{logging, AppConfig, AppPaths};

const VERSION: &str = concat!(
    env!("CARGO_PKG_VERSION"),
    " (",
    env!("LSP_SKILL_GIT_SHA"),
    ")"
);

#[derive(Parser)]
#[command(
    name = "lsp-skill-server",
    about = "1C LSP Skill — менеджер BSL Language Server",
    version = VERSION
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Управление службой (systemd / launchd / Windows autostart)
    Service {
        #[command(subcommand)]
        action: ServiceAction,
    },
}

#[derive(Subcommand)]
enum ServiceAction {
    /// Установить и запустить службу
    Install,
    /// Остановить и удалить службу
    Uninstall,
    /// Показать статус службы
    Status,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Service { action }) => match action {
            ServiceAction::Install => service::install()?,
            ServiceAction::Uninstall => service::uninstall()?,
            ServiceAction::Status => service::status()?,
        },
        None => run_server()?,
    }

    Ok(())
}

fn run_server() -> Result<()> {
    let paths = AppPaths::discover()?;
    paths.ensure()?;

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    let config = runtime.block_on(AppConfig::load_or_create(&paths))?;
    logging::init_logging(&paths.logs_dir, &config.log_level)?;

    let mcp_diag = config.mcp_diagnostics_enabled;
    let mcp_diag_port = config.mcp_diagnostics_port;
    let mcp_nav = config.mcp_navigation_enabled;
    let mcp_nav_port = config.mcp_navigation_port;
    let listen_host = config.listen_host.clone();

    let server = runtime.block_on(runtime::StartedServer::start(paths.clone(), config))?;
    println!("1C LSP Skill: {}", server.connect_url());
    if mcp_diag {
        println!("MCP Diagnostics: http://{listen_host}:{mcp_diag_port}/mcp");
    }
    if mcp_nav {
        println!("MCP Navigation: http://{listen_host}:{mcp_nav_port}/mcp");
    }

    runtime.block_on(server.wait_for_signal())?;

    Ok(())
}
