//! Drives a real `git push review HEAD:refs/for/main` through git's
//! remote-helper machinery against the built binaries, with gh replaced by a
//! recording fake on PATH.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

// Tests run in parallel and the clock is too coarse on macOS to tell them
// apart, so a counter keeps sandbox directories unique.
static SANDBOX_SEQ: AtomicUsize = AtomicUsize::new(0);

const FAKE_GH: &str = r#"#!/bin/sh
echo "$@" >> "$PRRIT_GH_LOG"
case "$1 $2" in
  "api user") echo alice ;;
  "pr list") echo "[]" ;;
  "pr create") n=$(grep -c "^pr create" "$PRRIT_GH_LOG"); echo "https://github.com/o/r/pull/$((100 + n))" ;;
  "extension list") printf 'gh stack\tgithub/gh-stack\tv0.1.1\n' ;;
  "stack link") echo "linked: $*" ;;
  "api --paginate") echo "[[]]" ;;
  "api --method") ;;
  *) echo "fake gh: unexpected $*" >&2; exit 1 ;;
esac
"#;

struct Sandbox {
    root: PathBuf,
    work: PathBuf,
    gh_log: PathBuf,
    path: String,
}

impl Sandbox {
    fn new() -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let seq = SANDBOX_SEQ.fetch_add(1, Ordering::Relaxed);
        let root =
            std::env::temp_dir().join(format!("prrit-e2e-{}-{nanos}-{seq}", std::process::id()));
        let bin = root.join("bin");
        fs::create_dir_all(&bin).unwrap();

        // Fake gh and the real helper binary side by side on PATH.
        fs::write(bin.join("gh"), FAKE_GH).unwrap();
        set_exec(&bin.join("gh"));
        let helper = env!("CARGO_BIN_EXE_git-remote-prrit");
        std::os::unix::fs::symlink(helper, bin.join("git-remote-prrit")).unwrap();
        let prrit = env!("CARGO_BIN_EXE_prrit");
        std::os::unix::fs::symlink(prrit, bin.join("prrit")).unwrap();

        let gh_log = root.join("gh.log");
        fs::write(&gh_log, "").unwrap();
        let path = format!(
            "{}:{}",
            bin.display(),
            std::env::var("PATH").unwrap_or_default()
        );

        let remote = root.join("remote.git");
        let work = root.join("work");
        let sb = Sandbox {
            root,
            work,
            gh_log,
            path,
        };
        sb.git_in(
            &sb.root,
            &[
                "init",
                "-q",
                "--bare",
                "-b",
                "main",
                remote.to_str().unwrap(),
            ],
        );
        sb.git_in(
            &sb.root,
            &[
                "clone",
                "-q",
                remote.to_str().unwrap(),
                sb.work.to_str().unwrap(),
            ],
        );
        sb.git(&["checkout", "-q", "-b", "main"]);
        sb.commit("base.txt", "chore: base");
        sb.git(&["push", "-q", "-u", "origin", "main"]);
        sb.git(&["remote", "set-head", "origin", "main"]);
        sb.git(&["config", "prrit.user", "alice"]);
        sb
    }

    fn cmd(&self, program: &str, args: &[&str], cwd: &Path) -> Output {
        Command::new(program)
            .args(args)
            .current_dir(cwd)
            .env("PATH", &self.path)
            .env("PRRIT_GH_LOG", &self.gh_log)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_AUTHOR_NAME", "Alice")
            .env("GIT_AUTHOR_EMAIL", "alice@example.com")
            .env("GIT_COMMITTER_NAME", "Alice")
            .env("GIT_COMMITTER_EMAIL", "alice@example.com")
            .output()
            .unwrap()
    }

    fn git_in(&self, cwd: &Path, args: &[&str]) -> String {
        let out = self.cmd("git", args, cwd);
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    fn git(&self, args: &[&str]) -> String {
        self.git_in(&self.work, args)
    }

    fn commit(&self, name: &str, message: &str) {
        fs::write(self.work.join(name), format!("{name}\n")).unwrap();
        self.git(&["add", name]);
        self.git(&["commit", "-q", "-m", message]);
    }

    fn gh_calls(&self) -> Vec<String> {
        fs::read_to_string(&self.gh_log)
            .unwrap()
            .lines()
            .map(String::from)
            .collect()
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn set_exec(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

#[test]
fn init_then_push_review_creates_and_links_prs() {
    let sb = Sandbox::new();
    let out = sb.cmd("prrit", &["init"], &sb.work);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(sb.git(&["remote", "get-url", "review"]), "prrit::origin");

    sb.git(&["checkout", "-q", "-b", "feature"]);
    sb.commit("a.txt", "feat: a"); // hook adds the Change-Id
    sb.git(&["config", "core.hooksPath", "/nonexistent"]); // next commit lacks one on purpose
    sb.commit("b.txt", "feat: b");
    sb.git(&["config", "--unset", "core.hooksPath"]);

    let out = sb.cmd("git", &["push", "review", "HEAD:refs/for/main"], &sb.work);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{stderr}");
    assert!(stderr.contains("HEAD -> refs/for/main"), "{stderr}");
    assert!(
        stderr.contains("Added Change-Id to 1 commit(s)."),
        "{stderr}"
    );

    let log = sb.git(&["log", "--format=%B", "origin/main..HEAD"]);
    let ids: Vec<&str> = log
        .lines()
        .filter_map(|l| l.strip_prefix("Change-Id: "))
        .collect();
    assert_eq!(ids.len(), 2);
    let branches = sb.git(&["ls-remote", "--heads", "origin", "refs/heads/prrit/alice/*"]);
    for id in &ids {
        assert!(
            branches.contains(&format!("refs/heads/prrit/alice/{}", &id[1..9])),
            "{branches}"
        );
    }

    let calls = sb.gh_calls();
    let creates: Vec<&String> = calls
        .iter()
        .filter(|c| c.starts_with("pr create"))
        .collect();
    assert_eq!(creates.len(), 2);
    assert!(creates[0].contains("--base main"));
    assert!(creates[1].contains("--base prrit/alice/"));
    assert_eq!(calls.last().unwrap(), "stack link --base main 101 102");
}

#[test]
fn wip_option_creates_drafts_and_bare_push_works_with_push_default() {
    let sb = Sandbox::new();
    sb.git(&["remote", "add", "review", "prrit::origin"]);
    sb.git(&["config", "remote.review.push", "HEAD:refs/for/main%wip"]);
    sb.git(&["config", "remote.pushDefault", "review"]);
    sb.git(&["checkout", "-q", "-b", "feature"]);
    sb.commit("a.txt", "feat: a");

    let out = sb.cmd("git", &["push"], &sb.work);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let calls = sb.gh_calls();
    assert!(
        calls
            .iter()
            .any(|c| c.starts_with("pr create --head prrit/alice/") && c.ends_with("--draft")),
        "{calls:?}"
    );
}

#[test]
fn rejects_destinations_outside_refs_for() {
    let sb = Sandbox::new();
    sb.git(&["remote", "add", "review", "prrit::origin"]);
    sb.git(&["checkout", "-q", "-b", "feature"]);
    sb.commit("a.txt", "feat: a");

    let out = sb.cmd("git", &["push", "review", "HEAD:refs/heads/main"], &sb.work);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("refs/for/<base>"), "{stderr}");
    assert!(stderr.contains("[remote rejected]"), "{stderr}");
    assert!(sb.gh_calls().is_empty());
}

#[test]
fn warns_and_overwrites_when_the_branch_changed_on_the_server() {
    let sb = Sandbox::new();
    sb.git(&["remote", "add", "review", "prrit::origin"]);
    sb.git(&["checkout", "-q", "-b", "feature"]);
    sb.commit("a.txt", "feat: a");
    let first = sb.cmd("git", &["push", "review", "HEAD:refs/for/main"], &sb.work);
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let branch = sb.git(&["ls-remote", "--heads", "origin", "refs/heads/prrit/alice/*"]);
    let branch = branch
        .split("refs/heads/")
        .nth(1)
        .unwrap()
        .trim()
        .to_string();

    // Someone rewrites the branch server-side from another clone.
    let other = sb.root.join("other");
    sb.git_in(
        &sb.root,
        &[
            "clone",
            "-q",
            sb.root.join("remote.git").to_str().unwrap(),
            other.to_str().unwrap(),
        ],
    );
    sb.git_in(
        &other,
        &["checkout", "-q", "-b", "x", &format!("origin/{branch}")],
    );
    fs::write(other.join("suggestion.txt"), "applied\n").unwrap();
    sb.git_in(&other, &["add", "suggestion.txt"]);
    sb.git_in(&other, &["commit", "-q", "-m", "Apply suggestion"]);
    sb.git_in(
        &other,
        &["push", "-q", "origin", &format!("HEAD:refs/heads/{branch}")],
    );
    let server_tip = sb.git_in(&other, &["rev-parse", "HEAD"]);

    let second = sb.cmd("git", &["push", "review", "HEAD:refs/for/main"], &sb.work);
    let stderr = String::from_utf8_lossy(&second.stderr);
    assert!(second.status.success(), "{stderr}");
    assert!(
        stderr.contains(&format!(
            "{branch} (was {}) changed on GitHub",
            &server_tip[..7]
        )),
        "{stderr}"
    );
    assert!(
        stderr.contains(&format!("git reset --hard origin/{branch}")),
        "{stderr}"
    );
    let now = sb.git(&[
        "ls-remote",
        "--heads",
        "origin",
        &format!("refs/heads/{branch}"),
    ]);
    assert!(now.starts_with(&sb.git(&["rev-parse", "HEAD"])), "{now}");
}

#[test]
fn status_and_help_run_from_the_binary() {
    let sb = Sandbox::new();
    let out = sb.cmd("prrit", &["--help"], &sb.work);
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("git push review"));

    let out = sb.cmd("prrit", &["bogus"], &sb.work);
    assert_eq!(out.status.code(), Some(1));

    for args in [&["--version"][..], &["-V"]] {
        let out = sb.cmd("prrit", args, &sb.work);
        assert!(out.status.success());
        assert_eq!(
            String::from_utf8_lossy(&out.stdout).trim(),
            format!("prrit {}", env!("CARGO_PKG_VERSION"))
        );
    }

    let out = sb.cmd("prrit", &["skill"], &sb.work);
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).starts_with("---\nname: prrit"));

    let out = sb.cmd("prrit", &["status"], &sb.work);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains("Nothing on top of origin/main."));
}
