use std::{
    path::Path,
    process::{Command, Output},
};
fn hash(path: &str) -> String {
    Command::new("/usr/bin/shasum")
        .args(["-a", "256", path])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| {
            String::from_utf8_lossy(&output.stdout)
                .split_whitespace()
                .next()
                .unwrap_or("unavailable")
                .to_owned()
        })
        .unwrap_or_else(|| "unavailable".into())
}
fn checkout_git_output(args: &[&str]) -> Option<Output> {
    // A source archive can sit inside another checkout. Only use Git if the
    // source root itself has repository metadata, including worktree pointers.
    if !Path::new("../.git").exists() {
        return None;
    }
    Command::new("git")
        .current_dir("..")
        .args(["-c", "core.fsmonitor=false"])
        .args(args)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .output()
        .ok()
        .filter(|output| output.status.success())
}

fn main() {
    let commit = checkout_git_output(&["rev-parse", "HEAD"])
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        .unwrap_or_else(|| "unavailable".into());
    let dirty = checkout_git_output(&["status", "--porcelain"])
        .is_some_and(|output| !output.stdout.is_empty());
    println!("cargo:rustc-env=CVEC_BUILD_COMMIT={commit}");
    println!("cargo:rustc-env=CVEC_BUILD_DIRTY={dirty}");
    println!(
        "cargo:rustc-env=CVEC_BUILD_PROFILE={}",
        std::env::var("PROFILE").unwrap_or_default()
    );
    for (name, path) in [
        ("CVEC_BUILD_CARGO_LOCK", "Cargo.lock"),
        ("CVEC_BUILD_NPM_LOCK", "../package-lock.json"),
    ] {
        println!("cargo:rerun-if-changed={path}");
        println!("cargo:rustc-env={name}={}", hash(path));
    }
    if Path::new("../.git/HEAD").is_file() {
        println!("cargo:rerun-if-changed=../.git/HEAD");
    }
    println!("cargo:rerun-if-env-changed=CVEC_BUILD_SOURCE_HASH");
    println!(
        "cargo:rustc-env=CVEC_BUILD_SOURCE_HASH={}",
        std::env::var("CVEC_BUILD_SOURCE_HASH").unwrap_or_else(|_| "unavailable".into())
    );
    tauri_build::build();
}
