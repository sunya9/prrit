//! Shared helpers for tests that drive a real git.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

// Parallel tests can share a clock tick on macOS; a counter keeps dirs unique.
static DIR_SEQ: AtomicUsize = AtomicUsize::new(0);

pub const TEST_ENV: &[(&str, &str)] = &[
    ("GIT_CONFIG_GLOBAL", "/dev/null"),
    ("GIT_CONFIG_NOSYSTEM", "1"),
    ("GIT_AUTHOR_NAME", "Alice"),
    ("GIT_AUTHOR_EMAIL", "alice@example.com"),
    ("GIT_AUTHOR_DATE", "2024-01-02T03:04:05+0900"),
    ("GIT_COMMITTER_NAME", "Bob"),
    ("GIT_COMMITTER_EMAIL", "bob@example.com"),
    ("GIT_COMMITTER_DATE", "2024-01-02T03:04:05+0900"),
];

pub fn sh(cwd: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .envs(TEST_ENV.iter().copied())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

pub struct TempRepo {
    pub root: PathBuf,
    pub work: PathBuf,
}

impl TempRepo {
    /// Bare `remote.git` with a `main` commit, cloned into `work`, remote HEAD set.
    pub fn with_remote() -> Self {
        let root = fresh_dir();
        let remote = root.join("remote.git");
        let work = root.join("work");
        sh(
            &root,
            &[
                "init",
                "-q",
                "--bare",
                "-b",
                "main",
                remote.to_str().unwrap(),
            ],
        );
        sh(
            &root,
            &[
                "clone",
                "-q",
                remote.to_str().unwrap(),
                work.to_str().unwrap(),
            ],
        );
        sh(&work, &["checkout", "-q", "-b", "main"]);
        std::fs::write(work.join("base.txt"), "base\n").unwrap();
        sh(&work, &["add", "base.txt"]);
        sh(&work, &["commit", "-q", "-m", "chore: base"]);
        sh(&work, &["push", "-q", "-u", "origin", "main"]);
        sh(&work, &["remote", "set-head", "origin", "main"]);
        TempRepo { root, work }
    }

    /// A lone repository (no remote) for hook tests.
    pub fn bare_work() -> Self {
        let root = fresh_dir();
        let work = root.join("work");
        std::fs::create_dir_all(&work).unwrap();
        sh(&work, &["init", "-q", "-b", "main"]);
        TempRepo { root, work }
    }
}

impl Drop for TempRepo {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn fresh_dir() -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("prrit-test-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}
