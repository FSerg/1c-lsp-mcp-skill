use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR is missing"));
    let dest = out_dir.join("frontend-dist");
    let source = PathBuf::from("../../target/frontend-dist");
    let fallback = PathBuf::from("static");

    let _ = fs::remove_dir_all(&dest);
    fs::create_dir_all(&dest).expect("failed to create frontend-dist dir");

    if source.exists() {
        copy_dir(&source, &dest);
    } else {
        copy_dir(&fallback, &dest);
    }

    println!("cargo:rerun-if-changed=static");
    println!("cargo:rerun-if-changed=../../target/frontend-dist");

    emit_git_sha();
}

fn emit_git_sha() {
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
    println!("cargo:rerun-if-changed=../../.git/HEAD");
    println!("cargo:rerun-if-changed=../../.git/index");
}

fn copy_dir(source: &Path, dest: &Path) {
    for entry in fs::read_dir(source).expect("failed to read source dir") {
        let entry = entry.expect("failed to read dir entry");
        let path = entry.path();
        let target = dest.join(entry.file_name());
        if path.is_dir() {
            fs::create_dir_all(&target).expect("failed to create target subdir");
            copy_dir(&path, &target);
        } else {
            fs::copy(&path, &target).expect("failed to copy file");
        }
    }
}
