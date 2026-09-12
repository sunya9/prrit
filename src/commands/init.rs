use std::path::PathBuf;

use crate::context::{resolve_base, Context};
use crate::error::Result;
use crate::hook::{COMMIT_MSG_HOOK, HOOK_MARKER};

pub const DEFAULT_REVIEW_REMOTE: &str = "review";
const HELPER_BINARY: &str = "git-remote-prrit";

pub struct InitOptions<'a> {
    pub remote: &'a str,
    pub base: Option<&'a str>,
    /// Name of the remote that carries prrit::<remote>; only the helper
    /// protocol cares about it, so any name works.
    pub review_remote: &'a str,
    /// Opt-in: makes a bare `git push` upload the stack. Off by default because
    /// it changes where every plain push in the repository goes.
    pub push_default: bool,
    /// Directories searched for git-remote-prrit (PATH by default).
    pub path_dirs: Option<Vec<PathBuf>>,
}

fn install_hook(ctx: &Context<'_>) -> Result<bool> {
    let hooks_dir = ctx.git.hooks_dir()?;
    let hook_path = hooks_dir.join("commit-msg");
    if hook_path.exists() && !std::fs::read_to_string(&hook_path)?.contains(HOOK_MARKER) {
        ctx.err(format!(
            "{} already exists and was not written by prrit; merge it by hand.",
            hook_path.display()
        ));
        return Ok(false);
    }
    std::fs::create_dir_all(&hooks_dir)?;
    std::fs::write(&hook_path, COMMIT_MSG_HOOK)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&hook_path, std::fs::Permissions::from_mode(0o755))?;
    }
    ctx.out(format!(
        "Installed commit-msg hook at {}",
        hook_path.display()
    ));
    Ok(true)
}

fn configure_review_remote(ctx: &Context<'_>, opts: &InitOptions<'_>) -> Result<bool> {
    let name = opts.review_remote;
    let url = format!("prrit::{}", opts.remote);
    if name == opts.remote {
        ctx.err(format!(
            "the review remote cannot be \"{name}\" itself; pick another name with --review-remote"
        ));
        return Ok(false);
    }
    match ctx.git.remote_url(name) {
        None => {
            ctx.git.add_remote(name, &url)?;
            ctx.out(format!("Added remote \"{name}\" -> {url}"));
        }
        Some(existing) if existing != url => {
            ctx.err(format!(
                "remote \"{name}\" already exists with url {existing}; expected {url} (or choose another name with --review-remote)"
            ));
            return Ok(false);
        }
        Some(_) => {}
    }

    let base = resolve_base(ctx, opts.remote, opts.base)?;
    ctx.git.set_config(
        &format!("remote.{name}.push"),
        &format!("HEAD:refs/for/{base}"),
    )?;
    if opts.push_default {
        ctx.git.set_config("remote.pushDefault", name)?;
        ctx.out(format!(
            "Configured \"git push\" to upload HEAD to refs/for/{base} via {}",
            opts.remote
        ));
    } else {
        ctx.out(format!(
            "Upload with \"git push {name}\" (HEAD -> refs/for/{base} via {}); pass --push-default to make a bare \"git push\" do it",
            opts.remote
        ));
    }
    Ok(true)
}

fn helper_on_path(dirs: &[PathBuf]) -> bool {
    dirs.iter()
        .any(|d| !d.as_os_str().is_empty() && d.join(HELPER_BINARY).exists())
}

pub fn init(ctx: &Context<'_>, opts: &InitOptions<'_>) -> Result<bool> {
    let mut ok = install_hook(ctx)?;
    ok &= configure_review_remote(ctx, opts)?;

    let path_dirs = match &opts.path_dirs {
        Some(dirs) => dirs.clone(),
        None => std::env::var_os("PATH")
            .map(|p| std::env::split_paths(&p).collect())
            .unwrap_or_default(),
    };
    if !helper_on_path(&path_dirs) {
        ctx.err(format!(
            "{HELPER_BINARY} is not on PATH; put it next to prrit so git can find it."
        ));
        ok = false;
    }
    if !ctx.gh.has_stack_extension() {
        ctx.err(
            "gh-stack extension not found; install it with: gh extension install github/gh-stack",
        );
        ok = false;
    }
    Ok(ok)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::change_id::extract_change_id;
    use crate::context::recorder::Recorder;
    use crate::runner::{ExecRunner, InheritTarget, RunOptions, Runner};
    use crate::testutil::{sh, TempRepo};

    const STACK_INSTALLED: &str = "gh stack\tgithub/gh-stack\tv0.1.1\n";

    /// Real git, but `gh extension list` is answered from `extensions`.
    struct GhStub {
        extensions: String,
        real: ExecRunner,
    }

    impl Runner for GhStub {
        fn run(
            &self,
            cmd: &str,
            args: &[&str],
            opts: &RunOptions<'_>,
        ) -> crate::error::Result<String> {
            if cmd == "gh" && args.first() == Some(&"extension") {
                return Ok(self.extensions.clone());
            }
            self.real.run(cmd, args, opts)
        }
    }

    struct Fixture {
        repo: TempRepo,
        helper_dir: PathBuf,
    }

    fn fixture() -> Fixture {
        let repo = TempRepo::bare_work();
        sh(
            &repo.work,
            &["remote", "add", "origin", "https://example.invalid/o/r.git"],
        );
        let helper_dir = repo.root.join("bin");
        std::fs::create_dir_all(&helper_dir).unwrap();
        std::fs::write(helper_dir.join(HELPER_BINARY), "").unwrap();
        Fixture { repo, helper_dir }
    }

    fn run_init(
        f: &Fixture,
        extensions: &str,
        rec: &Recorder,
        tweak: impl FnOnce(&mut InitOptions<'_>),
    ) -> bool {
        let runner = GhStub {
            extensions: extensions.into(),
            real: ExecRunner {
                inherit_target: InheritTarget::Stdout,
            },
        };
        let ctx = Context::new(&runner, &f.repo.work, rec);
        let mut opts = InitOptions {
            remote: "origin",
            base: Some("main"),
            review_remote: "review",
            push_default: false,
            path_dirs: Some(vec![f.helper_dir.clone()]),
        };
        tweak(&mut opts);
        init(&ctx, &opts).unwrap()
    }

    fn commit_with_hook(f: &Fixture, message: &str) -> String {
        std::fs::write(f.repo.work.join("f.txt"), "x\n").unwrap();
        sh(&f.repo.work, &["add", "f.txt"]);
        sh(&f.repo.work, &["commit", "-q", "-m", message]);
        sh(&f.repo.work, &["log", "-1", "--format=%B"])
    }

    #[test]
    fn installs_hook_that_adds_change_id() {
        let f = fixture();
        let rec = Recorder::default();
        assert!(run_init(&f, STACK_INSTALLED, &rec, |_| {}));
        assert!(f.repo.work.join(".git/hooks/commit-msg").exists());
        let message = commit_with_hook(&f, "feat: x\n\nbody");
        assert!(extract_change_id(&message).is_some());
        assert!(message.starts_with("feat: x\n\nbody\n\nChange-Id: I"));
    }

    #[test]
    fn hook_handles_editor_comment_block() {
        let f = fixture();
        let rec = Recorder::default();
        run_init(&f, STACK_INSTALLED, &rec, |_| {});
        let editor = f.repo.root.join("editor.sh");
        std::fs::write(&editor, "#!/bin/sh\nprintf 'feat: edited\\n\\n' | cat - \"$1\" > \"$1.tmp\" && mv \"$1.tmp\" \"$1\"\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&editor, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        std::fs::write(f.repo.work.join("f.txt"), "x\n").unwrap();
        sh(&f.repo.work, &["add", "f.txt"]);
        let out = std::process::Command::new("git")
            .args(["commit", "-q"])
            .current_dir(&f.repo.work)
            .envs(crate::testutil::TEST_ENV.iter().copied())
            .env("GIT_EDITOR", &editor)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let message = sh(&f.repo.work, &["log", "-1", "--format=%B"]);
        assert!(
            message.starts_with("feat: edited\n\nChange-Id: I"),
            "{message}"
        );
        assert_eq!(message.lines().count(), 3);
    }

    #[test]
    fn keeps_existing_change_id() {
        let f = fixture();
        let rec = Recorder::default();
        run_init(&f, STACK_INSTALLED, &rec, |_| {});
        let id = format!("I{}", "a".repeat(40));
        let message = commit_with_hook(&f, &format!("feat: x\n\nChange-Id: {id}"));
        assert_eq!(extract_change_id(&message).as_deref(), Some(id.as_str()));
    }

    #[test]
    fn idempotent_but_never_clobbers_foreign_hook() {
        let f = fixture();
        let rec = Recorder::default();
        assert!(run_init(&f, STACK_INSTALLED, &rec, |_| {}));
        assert!(run_init(&f, STACK_INSTALLED, &rec, |_| {}));
        let hook = f.repo.work.join(".git/hooks/commit-msg");
        std::fs::write(&hook, "#!/bin/sh\nexit 0\n").unwrap();
        assert!(!run_init(&f, STACK_INSTALLED, &rec, |_| {}));
        assert_eq!(
            std::fs::read_to_string(&hook).unwrap(),
            "#!/bin/sh\nexit 0\n"
        );
        assert!(rec.err_text().contains("not written by prrit"));
    }

    #[test]
    fn configures_review_remote_without_push_default() {
        let f = fixture();
        let rec = Recorder::default();
        assert!(run_init(&f, STACK_INSTALLED, &rec, |_| {}));
        assert_eq!(
            sh(&f.repo.work, &["remote", "get-url", "review"]),
            "prrit::origin"
        );
        assert_eq!(
            sh(&f.repo.work, &["config", "remote.review.push"]),
            "HEAD:refs/for/main"
        );
        assert!(std::process::Command::new("git")
            .args(["config", "remote.pushDefault"])
            .current_dir(&f.repo.work)
            .output()
            .map(|o| !o.status.success())
            .unwrap());
        assert!(rec.out_text().contains("git push review"));
    }

    #[test]
    fn push_default_and_custom_name() {
        let f = fixture();
        let rec = Recorder::default();
        assert!(run_init(&f, STACK_INSTALLED, &rec, |o| {
            o.review_remote = "gerrit";
            o.push_default = true;
        }));
        assert_eq!(
            sh(&f.repo.work, &["remote", "get-url", "gerrit"]),
            "prrit::origin"
        );
        assert_eq!(
            sh(&f.repo.work, &["config", "remote.gerrit.push"]),
            "HEAD:refs/for/main"
        );
        assert_eq!(
            sh(&f.repo.work, &["config", "remote.pushDefault"]),
            "gerrit"
        );
    }

    #[test]
    fn refuses_bad_remote_names() {
        let f = fixture();
        let rec = Recorder::default();
        assert!(!run_init(&f, STACK_INSTALLED, &rec, |o| o.review_remote = "origin"));
        assert!(rec.err_text().contains("--review-remote"));

        sh(
            &f.repo.work,
            &[
                "remote",
                "add",
                "review",
                "https://example.invalid/other.git",
            ],
        );
        assert!(!run_init(&f, STACK_INSTALLED, &rec, |_| {}));
        assert_eq!(
            sh(&f.repo.work, &["remote", "get-url", "review"]),
            "https://example.invalid/other.git"
        );
    }

    #[test]
    fn reports_missing_helper_and_extension() {
        let f = fixture();
        let rec = Recorder::default();
        assert!(!run_init(&f, STACK_INSTALLED, &rec, |o| o.path_dirs =
            Some(vec![f.repo.root.join("nowhere")])));
        assert!(rec.err_text().contains("git-remote-prrit is not on PATH"));
        let rec = Recorder::default();
        assert!(!run_init(&f, "", &rec, |_| {}));
        assert!(rec
            .err_text()
            .contains("gh extension install github/gh-stack"));
    }
}
