use std::path::PathBuf;

use anyhow::{bail, Context, Result};

const SERVICE_NAME: &str = "lsp-skill";

fn current_exe_path() -> Result<PathBuf> {
    let path = std::env::current_exe()
        .context("не удалось определить путь к исполняемому файлу")?
        .canonicalize()
        .context("не удалось получить абсолютный путь к исполняемому файлу")?;
    Ok(clean_path(path))
}

/// Убирает Windows extended-length prefix `\\?\` из пути.
fn clean_path(path: PathBuf) -> PathBuf {
    let s = path.to_string_lossy();
    match s.strip_prefix(r"\\?\") {
        Some(stripped) => PathBuf::from(stripped),
        None => path,
    }
}

// ── Linux: systemd user service ──────────────────────────────────────────────

#[cfg(target_os = "linux")]
pub fn install() -> Result<()> {
    use std::fs;

    let exe = current_exe_path()?;
    let unit_dir = dirs_unit_dir()?;
    fs::create_dir_all(&unit_dir)?;

    let unit_path = unit_dir.join(format!("{SERVICE_NAME}.service"));
    let unit_content = format!(
        "\
[Unit]
Description=LSP Skill — менеджер BSL Language Server
After=network.target

[Service]
Type=simple
ExecStart={exe}
Restart=on-failure
RestartSec=5

[Install]
WantedBy=default.target
",
        exe = exe.display(),
    );

    fs::write(&unit_path, unit_content)
        .with_context(|| format!("не удалось записать {}", unit_path.display()))?;
    println!("Создан unit-файл: {}", unit_path.display());

    run_systemctl(&["daemon-reload"])?;
    run_systemctl(&["enable", SERVICE_NAME])?;
    run_systemctl(&["start", SERVICE_NAME])?;

    println!("Служба {SERVICE_NAME} установлена и запущена.");
    println!("Управление: systemctl --user start|stop|restart|status {SERVICE_NAME}");
    Ok(())
}

#[cfg(target_os = "linux")]
pub fn uninstall() -> Result<()> {
    use std::fs;

    let _ = run_systemctl(&["stop", SERVICE_NAME]);
    let _ = run_systemctl(&["disable", SERVICE_NAME]);

    let unit_path = dirs_unit_dir()?.join(format!("{SERVICE_NAME}.service"));
    if unit_path.exists() {
        fs::remove_file(&unit_path)
            .with_context(|| format!("не удалось удалить {}", unit_path.display()))?;
        println!("Удалён unit-файл: {}", unit_path.display());
    }

    run_systemctl(&["daemon-reload"])?;
    println!("Служба {SERVICE_NAME} удалена.");
    Ok(())
}

#[cfg(target_os = "linux")]
pub fn status() -> Result<()> {
    let output = std::process::Command::new("systemctl")
        .args(["--user", "status", SERVICE_NAME])
        .status();

    match output {
        Ok(s) if s.success() => {}
        Ok(_) => println!("Служба {SERVICE_NAME} не запущена или не установлена."),
        Err(e) => bail!("не удалось выполнить systemctl: {e}"),
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn dirs_unit_dir() -> Result<PathBuf> {
    let home = std::env::var("HOME").context("не задана переменная HOME")?;
    Ok(PathBuf::from(home).join(".config/systemd/user"))
}

#[cfg(target_os = "linux")]
fn run_systemctl(args: &[&str]) -> Result<()> {
    let mut cmd_args = vec!["--user"];
    cmd_args.extend_from_slice(args);
    let status = std::process::Command::new("systemctl")
        .args(&cmd_args)
        .status()
        .with_context(|| format!("не удалось выполнить systemctl {}", args.join(" ")))?;
    if !status.success() {
        bail!("systemctl {} завершился с ошибкой", args.join(" "));
    }
    Ok(())
}

// ── macOS: launchd user agent ────────────────────────────────────────────────

#[cfg(target_os = "macos")]
const PLIST_LABEL: &str = "ru.infostart.lsp-skill";

#[cfg(target_os = "macos")]
pub fn install() -> Result<()> {
    use std::fs;
    use std::process::Command;

    let exe = current_exe_path()?;
    let plist_dir = dirs_launch_agents()?;
    fs::create_dir_all(&plist_dir)?;

    let plist_path = plist_dir.join(format!("{PLIST_LABEL}.plist"));
    let plist_content = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "https://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{label}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{exe}</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>StandardOutPath</key>
    <string>/tmp/{label}.out.log</string>
    <key>StandardErrorPath</key>
    <string>/tmp/{label}.err.log</string>
</dict>
</plist>
"#,
        label = PLIST_LABEL,
        exe = exe.display(),
    );

    fs::write(&plist_path, plist_content)
        .with_context(|| format!("не удалось записать {}", plist_path.display()))?;
    println!("Создан plist: {}", plist_path.display());

    let status = Command::new("launchctl")
        .args(["load", "-w"])
        .arg(&plist_path)
        .status()
        .context("не удалось выполнить launchctl load")?;

    if !status.success() {
        bail!("launchctl load завершился с ошибкой");
    }

    println!("Служба {PLIST_LABEL} установлена и запущена.");
    println!("Управление: launchctl start|stop {PLIST_LABEL}");
    Ok(())
}

#[cfg(target_os = "macos")]
pub fn uninstall() -> Result<()> {
    use std::fs;
    use std::process::Command;

    let plist_path = dirs_launch_agents()?.join(format!("{PLIST_LABEL}.plist"));

    if plist_path.exists() {
        let _ = Command::new("launchctl")
            .args(["unload", "-w"])
            .arg(&plist_path)
            .status();

        fs::remove_file(&plist_path)
            .with_context(|| format!("не удалось удалить {}", plist_path.display()))?;
        println!("Удалён plist: {}", plist_path.display());
    }

    println!("Служба {PLIST_LABEL} удалена.");
    Ok(())
}

#[cfg(target_os = "macos")]
pub fn status() -> Result<()> {
    let output = std::process::Command::new("launchctl")
        .args(["list", PLIST_LABEL])
        .output()
        .context("не удалось выполнить launchctl list")?;

    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        println!("{stdout}");
    } else {
        println!("Служба {PLIST_LABEL} не установлена или не запущена.");
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn dirs_launch_agents() -> Result<PathBuf> {
    let home = std::env::var("HOME").context("не задана переменная HOME")?;
    Ok(PathBuf::from(home).join("Library/LaunchAgents"))
}

// ── Windows: Windows Service via sc.exe ──────────────────────────────────────

#[cfg(target_os = "windows")]
const WIN_SERVICE_NAME: &str = "LspSkill";

#[cfg(target_os = "windows")]
pub fn install() -> Result<()> {
    use std::process::Command;

    let exe = current_exe_path()?;
    let exe_str = exe.display().to_string();

    // Создаём Windows Service
    let status = Command::new("sc.exe")
        .args([
            "create",
            WIN_SERVICE_NAME,
            &format!("binPath={exe_str}"),
            "start=auto",
            "DisplayName=1C LSP Skill Server",
        ])
        .status()
        .context("не удалось выполнить sc.exe create")?;

    if !status.success() {
        bail!(
            "sc.exe create завершился с ошибкой.\n\
             Убедитесь, что команда запущена от имени администратора."
        );
    }

    // Запускаем службу
    let status = Command::new("sc.exe")
        .args(["start", WIN_SERVICE_NAME])
        .status()
        .context("не удалось выполнить sc.exe start")?;

    if !status.success() {
        println!(
            "Предупреждение: служба создана, но не удалось запустить.\n\
             Запустите вручную: sc.exe start {WIN_SERVICE_NAME}"
        );
    }

    println!("Служба {WIN_SERVICE_NAME} установлена.");
    println!("Управление: sc.exe start|stop|query {WIN_SERVICE_NAME}");
    println!();
    println!(
        "Примечание: служба работает от имени LocalSystem.\n\
         Чтобы использовать конфигурацию текущего пользователя,\n\
         измените учётную запись службы в services.msc."
    );
    Ok(())
}

#[cfg(target_os = "windows")]
pub fn uninstall() -> Result<()> {
    use std::process::Command;

    let _ = Command::new("sc.exe")
        .args(["stop", WIN_SERVICE_NAME])
        .status();

    let status = Command::new("sc.exe")
        .args(["delete", WIN_SERVICE_NAME])
        .status()
        .context("не удалось выполнить sc.exe delete")?;

    if !status.success() {
        bail!(
            "sc.exe delete завершился с ошибкой.\n\
             Убедитесь, что команда запущена от имени администратора."
        );
    }

    println!("Служба {WIN_SERVICE_NAME} удалена.");
    Ok(())
}

#[cfg(target_os = "windows")]
pub fn status() -> Result<()> {
    let output = std::process::Command::new("sc.exe")
        .args(["query", WIN_SERVICE_NAME])
        .output()
        .context("не удалось выполнить sc.exe query")?;

    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        println!("{stdout}");
    } else {
        println!("Служба {WIN_SERVICE_NAME} не установлена.");
    }
    Ok(())
}

// ── Unsupported platform ─────────────────────────────────────────────────────

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
pub fn install() -> Result<()> {
    bail!("установка службы не поддерживается на этой платформе");
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
pub fn uninstall() -> Result<()> {
    bail!("удаление службы не поддерживается на этой платформе");
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
pub fn status() -> Result<()> {
    bail!("статус службы не поддерживается на этой платформе");
}
