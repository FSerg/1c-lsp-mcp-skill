use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use reqwest::StatusCode;

use lsp_skill_core::{compute_connect_url, AppConfig, AppPaths, RuntimeMetadata};

#[derive(Parser)]
#[command(name = "lsp-skill", about = "CLI клиент для 1C LSP Skill")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Статус проекта
    Status,
    /// Диагностика файла
    Diagnostics { file_path: String },
    /// Символы файла
    Symbols { file_path: String },
    /// Поиск ссылок
    References {
        file_path: String,
        #[arg(long)]
        line: u32,
        #[arg(long = "col")]
        character: u32,
    },
    /// Переход к определению символа
    Definition {
        file_path: String,
        #[arg(long)]
        line: u32,
        #[arg(long = "col")]
        character: u32,
    },
    /// Поиск символов по всему проекту
    #[command(name = "workspace-symbols")]
    WorkspaceSymbols {
        /// Строка поиска (регулярное выражение)
        query: String,
    },
    /// Добавить lsp-skill и lsp-skill-server в PATH
    InstallPath,
}

#[tokio::main]
async fn main() {
    if let Err(err) = run().await {
        eprintln!("{err}");
        std::process::exit(1);
    }
}

async fn run() -> Result<()> {
    let cli = Cli::parse();

    if let Commands::InstallPath = &cli.command {
        return install_path();
    }

    let project_id = load_project_id()?;
    let connect_url = load_connect_url().await?;
    let client = reqwest::Client::new();

    match cli.command {
        Commands::Status => {
            let response = client
                .get(format!("{connect_url}/api/projects/{project_id}/status"))
                .send()
                .await
                .map_err(|_| connection_error(&connect_url))?;
            print_json_response(response).await?;
        }
        Commands::Diagnostics { file_path } => {
            let response = client
                .post(format!(
                    "{connect_url}/api/projects/{project_id}/diagnostics"
                ))
                .json(&serde_json::json!({ "file_path": normalize_cli_file_path(&file_path) }))
                .send()
                .await
                .map_err(|_| connection_error(&connect_url))?;
            print_json_response(response).await?;
        }
        Commands::Symbols { file_path } => {
            let response = client
                .post(format!("{connect_url}/api/projects/{project_id}/symbols"))
                .json(&serde_json::json!({ "file_path": normalize_cli_file_path(&file_path) }))
                .send()
                .await
                .map_err(|_| connection_error(&connect_url))?;
            print_json_response(response).await?;
        }
        Commands::References {
            file_path,
            line,
            character,
        } => {
            let response = client
                .post(format!(
                    "{connect_url}/api/projects/{project_id}/references"
                ))
                .json(&serde_json::json!({
                    "file_path": normalize_cli_file_path(&file_path),
                    "line": line,
                    "character": character
                }))
                .send()
                .await
                .map_err(|_| connection_error(&connect_url))?;
            print_json_response(response).await?;
        }
        Commands::Definition {
            file_path,
            line,
            character,
        } => {
            let response = client
                .post(format!(
                    "{connect_url}/api/projects/{project_id}/definition"
                ))
                .json(&serde_json::json!({
                    "file_path": normalize_cli_file_path(&file_path),
                    "line": line,
                    "character": character
                }))
                .send()
                .await
                .map_err(|_| connection_error(&connect_url))?;
            print_json_response(response).await?;
        }
        Commands::WorkspaceSymbols { query } => {
            let response = client
                .post(format!(
                    "{connect_url}/api/projects/{project_id}/workspace-symbols"
                ))
                .json(&serde_json::json!({ "query": query }))
                .send()
                .await
                .map_err(|_| connection_error(&connect_url))?;
            print_json_response(response).await?;
        }
        Commands::InstallPath => unreachable!(),
    }

    Ok(())
}

// ── install-path ─────────────────────────────────────────────────────────────

fn install_path() -> Result<()> {
    let exe = std::env::current_exe().context("не удалось определить путь к исполняемому файлу")?;
    let bin_dir = exe
        .parent()
        .context("не удалось определить директорию исполняемого файла")?;

    #[cfg(unix)]
    install_path_unix(bin_dir)?;

    #[cfg(windows)]
    install_path_windows(bin_dir)?;

    Ok(())
}

#[cfg(unix)]
fn install_path_unix(bin_dir: &Path) -> Result<()> {
    use std::fs;
    use std::os::unix::fs as unix_fs;

    let home = std::env::var("HOME").context("не задана переменная HOME")?;
    let local_bin = PathBuf::from(&home).join(".local/bin");
    fs::create_dir_all(&local_bin)
        .with_context(|| format!("не удалось создать {}", local_bin.display()))?;

    // Создаём симлинки для всех бинарников в директории
    for name in ["lsp-skill", "lsp-skill-server"] {
        let source = bin_dir.join(name);
        let target = local_bin.join(name);

        if !source.exists() {
            continue;
        }

        if target.exists() || target.is_symlink() {
            fs::remove_file(&target).ok();
        }

        unix_fs::symlink(&source, &target).with_context(|| {
            format!(
                "не удалось создать симлинк {} -> {}",
                target.display(),
                source.display()
            )
        })?;
        println!(
            "Создан симлинк: {} -> {}",
            target.display(),
            source.display()
        );
    }

    // Проверяем, что ~/.local/bin в PATH
    let path_var = std::env::var("PATH").unwrap_or_default();
    let local_bin_str = local_bin.display().to_string();
    if !path_var.split(':').any(|p| p == local_bin_str) {
        println!();
        println!("Директория {} не найдена в PATH.", local_bin.display());
        println!("Добавьте в ~/.bashrc или ~/.zshrc:");
        println!();
        println!("  export PATH=\"$HOME/.local/bin:$PATH\"");
        println!();
        println!("Затем перезапустите терминал или выполните: source ~/.bashrc");
    } else {
        println!();
        println!("Готово! Команды lsp-skill и lsp-skill-server доступны в PATH.");
    }

    Ok(())
}

#[cfg(windows)]
fn install_path_windows(bin_dir: &Path) -> Result<()> {
    use std::process::Command;

    let bin_dir_str = bin_dir.display().to_string();

    // Проверяем, не в PATH ли уже
    let path_var = std::env::var("PATH").unwrap_or_default();
    if path_var
        .split(';')
        .any(|p| p.eq_ignore_ascii_case(&bin_dir_str))
    {
        println!("Директория {} уже в PATH.", bin_dir_str);
        return Ok(());
    }

    // Получаем текущий пользовательский PATH из реестра
    let output = Command::new("reg")
        .args(["query", "HKCU\\Environment", "/v", "Path"])
        .output()
        .context("не удалось прочитать PATH из реестра")?;

    let current_path = if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        // Формат: "    Path    REG_EXPAND_SZ    value"
        stdout
            .lines()
            .find(|line| line.contains("Path") || line.contains("PATH"))
            .and_then(|line| {
                line.split("REG_EXPAND_SZ")
                    .nth(1)
                    .or(line.split("REG_SZ").nth(1))
            })
            .map(|v| v.trim().to_string())
            .unwrap_or_default()
    } else {
        String::new()
    };

    let new_path = if current_path.is_empty() {
        bin_dir_str.clone()
    } else {
        format!("{};{}", current_path.trim_end_matches(';'), bin_dir_str)
    };

    let status = Command::new("setx")
        .args(["Path", &new_path])
        .status()
        .context("не удалось выполнить setx")?;

    if !status.success() {
        bail!("setx завершился с ошибкой");
    }

    println!("Директория {} добавлена в PATH.", bin_dir_str);
    println!("Перезапустите терминал, чтобы изменения вступили в силу.");
    Ok(())
}

// ── Existing functionality ───────────────────────────────────────────────────

fn load_project_id() -> Result<String> {
    let env_path = find_env_file(std::env::current_dir()?)
        .context("Не найден .env. Проверьте текущую директорию и родителей.")?;
    let content = std::fs::read_to_string(&env_path)
        .with_context(|| format!("Не удалось прочитать {}", env_path.display()))?;

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(value) = line.strip_prefix("PROJECT_ID=") {
            let value = value.trim().trim_matches('"');
            if !value.is_empty() {
                return Ok(value.to_string());
            }
        }
    }

    bail!("В файле {} отсутствует PROJECT_ID.", env_path.display())
}

fn find_env_file(mut dir: PathBuf) -> Option<PathBuf> {
    loop {
        let candidate = dir.join(".env");
        if candidate.exists() {
            return Some(candidate);
        }
        if !dir.pop() {
            return None;
        }
    }
}

async fn load_connect_url() -> Result<String> {
    let paths = AppPaths::discover()?;
    if let Some(runtime) = RuntimeMetadata::read(&paths).await? {
        return Ok(runtime.connect_url);
    }

    let config = AppConfig::load_or_create(&paths).await?;
    let connect_url = compute_connect_url(&config.listen_host, config.http_port);
    Err(connection_error(&connect_url))
}

async fn print_json_response(response: reqwest::Response) -> Result<()> {
    if response.status().is_success() {
        let json: serde_json::Value = response.json().await?;
        println!("{}", serde_json::to_string_pretty(&json)?);
        return Ok(());
    }

    if response.status() == StatusCode::NOT_FOUND {
        let value: serde_json::Value = response.json().await.unwrap_or_default();
        bail!(extract_error_message(value, "Запрос завершился с ошибкой."));
    }

    let value: serde_json::Value = response.json().await.unwrap_or_default();
    bail!(extract_error_message(value, "Запрос завершился с ошибкой."));
}

fn extract_error_message(value: serde_json::Value, fallback: &str) -> String {
    value
        .get("error")
        .and_then(|value| value.as_str())
        .map(ToString::to_string)
        .unwrap_or_else(|| fallback.to_string())
}

fn normalize_cli_file_path(path: &str) -> String {
    let path = Path::new(path);
    if path.is_absolute() {
        if let Ok(current_dir) = std::env::current_dir() {
            if let Ok(relative) = path.strip_prefix(&current_dir) {
                return relative.to_string_lossy().to_string();
            }
        }
    }
    path.to_string_lossy().trim_start_matches("./").to_string()
}

fn connection_error(connect_url: &str) -> anyhow::Error {
    anyhow::anyhow!(
        "Не удалось подключиться к серверу {connect_url}. Проверьте, что сервер запущен и доступен по сети."
    )
}
