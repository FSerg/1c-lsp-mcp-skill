use std::env;
use std::fs;
use std::path::{Path, PathBuf};

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
