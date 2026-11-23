use std::{
    path::Path,
    process::{Command, Output},
    time::Duration,
};

use crate::Error;

fn command(root: &Path) -> Command {
    let mut command = Command::new("git");
    for (name, _) in std::env::vars_os() {
        if name.to_string_lossy().starts_with("GIT_") {
            command.env_remove(name);
        }
    }
    command
        .args([
            "--literal-pathspecs",
            "-c",
            "core.fsmonitor=false",
            "-c",
            "core.hooksPath=/dev/null",
            "-C",
        ])
        .arg(root)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_TERMINAL_PROMPT", "0")
        // A missing object in a partial clone must fail locally. Blocking
        // transports also covers Git versions without GIT_NO_LAZY_FETCH.
        .env("GIT_NO_LAZY_FETCH", "1")
        .env("GIT_ALLOW_PROTOCOL", "");
    command
}

pub(super) async fn run(
    root: &Path,
    args: &[&str],
    timeout: Duration,
    stdout_limit: usize,
) -> Result<Output, Error> {
    // Git can invoke configured clean and smudge filters while inspecting or
    // checking out files. Neutralize filters from every applicable config.
    let mut inventory = command(root);
    inventory.args([
        "config",
        "--null",
        "--name-only",
        "--get-regexp",
        r"^filter\..*\.(clean|smudge|process|required)$",
    ]);
    let filters =
        super::process::output(inventory.into(), None, Duration::from_secs(5), 128 * 1024).await?;
    if !filters.status.success() && filters.status.code() != Some(1) {
        return Err(Error::App(
            "Git content-filter configuration could not be inspected safely.".into(),
        ));
    }
    let mut command = command(root);
    for key in filters
        .stdout
        .split(|byte| *byte == 0)
        .filter(|key| !key.is_empty())
    {
        let key = std::str::from_utf8(key)
            .map_err(|_| Error::App("Unsupported Git filter name.".into()))?;
        // Git's -c separator cannot address a subsection containing '='.
        if key.contains('=') {
            return Err(Error::App("Unsupported Git filter name.".into()));
        }
        let value = if key.ends_with(".required") {
            "false"
        } else {
            ""
        };
        command.arg("-c").arg(format!("{key}={value}"));
    }
    command.args(args);
    super::process::output(command.into(), None, timeout, stdout_limit).await
}

pub(super) fn run_sync(
    root: &Path,
    args: &[&str],
    timeout: Duration,
    stdout_limit: usize,
) -> Result<Output, Error> {
    std::thread::scope(|scope| {
        scope
            .spawn(|| {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()?
                    .block_on(run(root, args, timeout, stdout_limit))
            })
            .join()
    })
    .map_err(|_| Error::App("Git command worker failed".into()))?
}

#[cfg(all(test, unix))]
mod tests {
    use std::{fs, os::unix::fs::PermissionsExt};

    use super::*;

    #[tokio::test]
    async fn missing_git_object_does_not_invoke_promisor_remote() {
        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path();
        let helper = root.join("promisor-helper");
        fs::write(
            &helper,
            "#!/bin/sh\n: > \"$(dirname \"$0\")/promisor-invoked\"\nexit 1\n",
        )
        .unwrap();
        fs::set_permissions(&helper, fs::Permissions::from_mode(0o700)).unwrap();
        let remote = format!(
            "ext::{}",
            helper
                .to_str()
                .unwrap()
                .replace('%', "%%")
                .replace(' ', "% ")
        );
        for args in [
            vec!["init", "--quiet"],
            vec!["config", "core.repositoryformatversion", "1"],
            vec!["config", "extensions.partialclone", "origin"],
            vec!["config", "remote.origin.promisor", "true"],
            vec!["config", "remote.origin.url", &remote],
            vec!["config", "protocol.ext.allow", "always"],
        ] {
            assert!(command(root).args(args).status().unwrap().success());
        }

        let output = run(
            root,
            &["cat-file", "-e", "0123456789012345678901234567890123456789"],
            Duration::from_secs(5),
            4096,
        )
        .await
        .unwrap();
        assert!(!output.status.success());
        assert!(!root.join("promisor-invoked").exists());
    }
}
