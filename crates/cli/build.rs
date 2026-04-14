use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let workspace_root = PathBuf::from("../..");

    let sha = Command::new("git")
        .args(["rev-parse", "--short=7", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    let dirty = Command::new("git")
        .args(["status", "--porcelain"])
        .output()
        .ok()
        .map(|o| !o.stdout.is_empty())
        .unwrap_or(false);

    let suffix = if dirty { "-dirty" } else { "" };
    println!("cargo:rustc-env=LSP_SKILL_GIT_SHA={sha}{suffix}");

    if let Some(git_dir) = resolve_git_dir(&workspace_root) {
        let head_path = git_dir.join("HEAD");
        println!("cargo:rerun-if-changed={}", head_path.display());
        if let Ok(head) = fs::read_to_string(&head_path) {
            if let Some(rest) = head.lines().next().and_then(|l| l.strip_prefix("ref: ")) {
                let ref_file = git_dir.join(rest.trim());
                println!("cargo:rerun-if-changed={}", ref_file.display());
            }
        }
    }
}

fn resolve_git_dir(workspace_root: &Path) -> Option<PathBuf> {
    let dot_git = workspace_root.join(".git");
    let meta = fs::metadata(&dot_git).ok()?;
    if meta.is_dir() {
        return Some(dot_git);
    }
    if meta.is_file() {
        let content = fs::read_to_string(&dot_git).ok()?;
        let gitdir = content
            .lines()
            .find_map(|l| l.strip_prefix("gitdir: "))?
            .trim();
        let path = PathBuf::from(gitdir);
        if path.is_absolute() {
            return Some(path);
        }
        return Some(workspace_root.join(path));
    }
    None
}
