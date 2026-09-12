use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::change_id::{append_change_id, extract_change_id, generate_change_id};
use crate::error::{Error, Result};
use crate::plan::{Commit, Push};
use crate::runner::{RunOptions, Runner};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawCommit {
    pub sha: String,
    pub message: String,
    pub change_id: Option<String>,
}

pub struct EnsureResult {
    pub commits: Vec<Commit>,
    pub rewritten: usize,
}

pub struct Git<'a> {
    runner: &'a dyn Runner,
    cwd: PathBuf,
}

impl<'a> Git<'a> {
    pub fn new(runner: &'a dyn Runner, cwd: &Path) -> Self {
        Git {
            runner,
            cwd: cwd.to_path_buf(),
        }
    }

    fn git(&self, args: &[&str]) -> Result<String> {
        self.runner.run(
            "git",
            args,
            &RunOptions {
                cwd: Some(&self.cwd),
                ..Default::default()
            },
        )
    }

    fn git_with(&self, args: &[&str], input: &str, env: Vec<(&str, &str)>) -> Result<String> {
        self.runner.run(
            "git",
            args,
            &RunOptions {
                cwd: Some(&self.cwd),
                input: Some(input),
                env,
                inherit: false,
            },
        )
    }

    pub fn default_branch(&self, remote: &str) -> Option<String> {
        let full = format!("refs/remotes/{remote}/HEAD");
        let out = self.git(&["symbolic-ref", "--short", &full]).ok()?;
        let short = out.trim();
        Some(
            short
                .strip_prefix(&format!("{remote}/"))
                .unwrap_or(short)
                .to_string(),
        )
    }

    pub fn fetch(&self, remote: &str, branch: &str) -> Result<()> {
        self.git(&["fetch", "--quiet", remote, branch]).map(drop)
    }

    pub fn rev_list(&self, base: &str, head: &str) -> Result<Vec<RawCommit>> {
        let range = format!("{base}..{head}");
        let out = self.git(&["log", "--reverse", "-z", "--format=%H%n%P%n%B", &range])?;
        let mut commits = Vec::new();
        for record in out.split('\0') {
            if record.trim().is_empty() {
                continue;
            }
            let mut lines = record.splitn(3, '\n');
            let sha = lines.next().unwrap_or("").to_string();
            let parents = lines.next().unwrap_or("");
            if parents.split_whitespace().count() > 1 {
                return Err(Error::msg(format!(
                    "{} is a merge commit; prrit only handles linear history",
                    &sha[..7.min(sha.len())]
                )));
            }
            let message = format!("{}\n", lines.next().unwrap_or("").trim_end());
            let change_id = extract_change_id(&message);
            commits.push(RawCommit {
                sha,
                message,
                change_id,
            });
        }
        Ok(commits)
    }

    pub fn ensure_change_ids(&self, commits: &[RawCommit], ref_name: &str) -> Result<EnsureResult> {
        if commits.iter().all(|c| c.change_id.is_some()) {
            let commits = commits
                .iter()
                .map(|c| Commit {
                    sha: c.sha.clone(),
                    message: c.message.clone(),
                    change_id: c.change_id.clone().unwrap(),
                })
                .collect();
            return Ok(EnsureResult {
                commits,
                rewritten: 0,
            });
        }

        let old_tip = self.git(&["rev-parse", ref_name])?.trim().to_string();
        let first_parent = format!("{}^", commits[0].sha);
        let mut parent = self.git(&["rev-parse", &first_parent])?.trim().to_string();
        let mut parent_changed = false;
        let mut rewritten = 0;
        let mut result = Vec::new();

        for commit in commits {
            if let (Some(id), false) = (&commit.change_id, parent_changed) {
                result.push(Commit {
                    sha: commit.sha.clone(),
                    message: commit.message.clone(),
                    change_id: id.clone(),
                });
                parent = commit.sha.clone();
                continue;
            }
            let (change_id, message) = match &commit.change_id {
                Some(id) => (id.clone(), commit.message.clone()),
                None => {
                    rewritten += 1;
                    let id = generate_change_id();
                    let message = append_change_id(&commit.message, &id);
                    (id, message)
                }
            };
            let sha = self.rewrite_commit(&commit.sha, &parent, &message)?;
            result.push(Commit {
                sha: sha.clone(),
                message,
                change_id,
            });
            parent = sha;
            parent_changed = true;
        }

        if parent_changed {
            // Every rewritten commit keeps its tree, so moving the ref (HEAD
            // follows the symbolic ref; a branch is updated even if checked out)
            // leaves the worktree untouched.
            self.git(&[
                "update-ref",
                "-m",
                "prrit: add Change-Id",
                ref_name,
                &parent,
                &old_tip,
            ])?;
        }
        Ok(EnsureResult {
            commits: result,
            rewritten,
        })
    }

    fn rewrite_commit(&self, sha: &str, parent: &str, message: &str) -> Result<String> {
        let tree = self
            .git(&["rev-parse", &format!("{sha}^{{tree}}")])?
            .trim()
            .to_string();
        let author = self.git(&["log", "-1", "--format=%an%n%ae%n%aD", sha])?;
        let mut fields = author.lines();
        let name = fields.next().unwrap_or("");
        let email = fields.next().unwrap_or("");
        let date = fields.next().unwrap_or("");
        let out = self.git_with(
            &["commit-tree", &tree, "-p", parent],
            message,
            vec![
                ("GIT_AUTHOR_NAME", name),
                ("GIT_AUTHOR_EMAIL", email),
                ("GIT_AUTHOR_DATE", date),
            ],
        )?;
        Ok(out.trim().to_string())
    }

    /// Whether the commit object exists locally. Anything prrit pushed does, so
    /// a remote tip that is missing here was written by something else.
    pub fn has_commit(&self, sha: &str) -> bool {
        self.git(&["cat-file", "-e", &format!("{sha}^{{commit}}")])
            .is_ok()
    }

    pub fn ls_remote(&self, remote: &str, prefix: &str) -> Result<HashMap<String, String>> {
        let pattern = format!("refs/heads/{prefix}*");
        let out = self.git(&["ls-remote", "--heads", remote, &pattern])?;
        let mut refs = HashMap::new();
        for line in out.lines() {
            if let Some((sha, r)) = line.split_once('\t') {
                if let Some(branch) = r.strip_prefix("refs/heads/") {
                    refs.insert(branch.to_string(), sha.to_string());
                }
            }
        }
        Ok(refs)
    }

    pub fn push(&self, remote: &str, pushes: &[Push]) -> Result<()> {
        if pushes.is_empty() {
            return Ok(());
        }
        let mut args: Vec<String> = vec!["push".into(), "--quiet".into(), "--atomic".into()];
        for p in pushes {
            args.push(format!(
                "--force-with-lease=refs/heads/{}:{}",
                p.branch,
                p.expected_remote_sha.as_deref().unwrap_or("")
            ));
        }
        args.push(remote.to_string());
        for p in pushes {
            args.push(format!("{}:refs/heads/{}", p.sha, p.branch));
        }
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        self.git(&refs).map(drop)
    }

    pub fn get_config(&self, key: &str) -> Option<String> {
        let out = self.git(&["config", "--local", "--get", key]).ok()?;
        let value = out.trim();
        (!value.is_empty()).then(|| value.to_string())
    }

    pub fn set_config(&self, key: &str, value: &str) -> Result<()> {
        self.git(&["config", "--local", key, value]).map(drop)
    }

    pub fn remote_url(&self, remote: &str) -> Option<String> {
        self.git(&["remote", "get-url", remote])
            .ok()
            .map(|s| s.trim().to_string())
    }

    pub fn add_remote(&self, name: &str, url: &str) -> Result<()> {
        self.git(&["remote", "add", name, url]).map(drop)
    }

    pub fn hooks_dir(&self) -> Result<PathBuf> {
        let out = self.git(&["rev-parse", "--path-format=absolute", "--git-path", "hooks"])?;
        Ok(PathBuf::from(out.trim()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::{ExecRunner, InheritTarget};
    use crate::testutil::{sh, TempRepo};

    fn commit_file(repo: &TempRepo, name: &str, message: &str) -> String {
        std::fs::write(repo.work.join(name), format!("{name}\n")).unwrap();
        sh(&repo.work, &["add", name]);
        sh(&repo.work, &["commit", "-q", "-m", message]);
        sh(&repo.work, &["rev-parse", "HEAD"])
    }

    const RUNNER: ExecRunner = ExecRunner {
        inherit_target: InheritTarget::Stdout,
    };

    #[test]
    fn default_branch_reads_remote_head() {
        let repo = TempRepo::with_remote();
        let git = Git::new(&RUNNER, &repo.work);
        assert_eq!(git.default_branch("origin").as_deref(), Some("main"));
        assert_eq!(git.default_branch("nope"), None);
    }

    #[test]
    fn rev_list_bottom_to_top_with_messages() {
        let repo = TempRepo::with_remote();
        sh(&repo.work, &["checkout", "-q", "-b", "feature"]);
        let a = commit_file(&repo, "a.txt", "feat: a\n\nChange-Id: Iaaaa");
        let b = commit_file(&repo, "b.txt", "feat: b");
        let git = Git::new(&RUNNER, &repo.work);
        let commits = git.rev_list("origin/main", "HEAD").unwrap();
        assert_eq!(
            commits.iter().map(|c| c.sha.clone()).collect::<Vec<_>>(),
            vec![a, b]
        );
        assert_eq!(commits[0].message, "feat: a\n\nChange-Id: Iaaaa\n");
        assert_eq!(commits[0].change_id.as_deref(), Some("Iaaaa"));
        assert_eq!(commits[1].message, "feat: b\n");
        assert_eq!(commits[1].change_id, None);
    }

    #[test]
    fn rev_list_rejects_merges() {
        let repo = TempRepo::with_remote();
        sh(&repo.work, &["checkout", "-q", "-b", "side"]);
        commit_file(&repo, "s.txt", "feat: side");
        sh(&repo.work, &["checkout", "-q", "main"]);
        commit_file(&repo, "m.txt", "feat: main");
        sh(
            &repo.work,
            &["merge", "-q", "--no-ff", "-m", "merge side", "side"],
        );
        let git = Git::new(&RUNNER, &repo.work);
        assert!(git
            .rev_list("origin/main", "HEAD")
            .unwrap_err()
            .to_string()
            .contains("merge commit"));
    }

    #[test]
    fn ensure_change_ids_rewrites_only_missing_keeping_tree_and_author() {
        let repo = TempRepo::with_remote();
        sh(&repo.work, &["checkout", "-q", "-b", "feature"]);
        let a = commit_file(&repo, "a.txt", "feat: a\n\nChange-Id: Iaaaa");
        let b = commit_file(&repo, "b.txt", "feat: b");
        let tree_b = sh(&repo.work, &["rev-parse", &format!("{b}^{{tree}}")]);
        let git = Git::new(&RUNNER, &repo.work);

        let result = git
            .ensure_change_ids(&git.rev_list("origin/main", "HEAD").unwrap(), "HEAD")
            .unwrap();

        assert_eq!(result.rewritten, 1);
        assert_eq!(result.commits[0].sha, a);
        assert_eq!(result.commits[0].change_id, "Iaaaa");
        let new_b = &result.commits[1];
        assert_ne!(new_b.sha, b);
        assert_eq!(
            extract_change_id(&new_b.message).as_deref(),
            Some(new_b.change_id.as_str())
        );
        assert_eq!(sh(&repo.work, &["rev-parse", "HEAD"]), new_b.sha);
        assert_eq!(sh(&repo.work, &["rev-parse", "feature"]), new_b.sha);
        assert_eq!(
            sh(
                &repo.work,
                &["rev-parse", &format!("{}^{{tree}}", new_b.sha)]
            ),
            tree_b
        );
        assert_eq!(
            sh(
                &repo.work,
                &["log", "-1", "--format=%an <%ae> %ai", &new_b.sha]
            ),
            "Alice <alice@example.com> 2024-01-02 03:04:05 +0900"
        );
    }

    #[test]
    fn ensure_change_ids_rewrites_descendants() {
        let repo = TempRepo::with_remote();
        sh(&repo.work, &["checkout", "-q", "-b", "feature"]);
        commit_file(&repo, "a.txt", "feat: a");
        let b = commit_file(&repo, "b.txt", "feat: b\n\nChange-Id: Ibbbb");
        let git = Git::new(&RUNNER, &repo.work);
        let result = git
            .ensure_change_ids(&git.rev_list("origin/main", "HEAD").unwrap(), "HEAD")
            .unwrap();
        assert_eq!(result.rewritten, 1);
        assert_ne!(result.commits[1].sha, b);
        assert_eq!(result.commits[1].change_id, "Ibbbb");
        assert_eq!(
            sh(
                &repo.work,
                &["rev-parse", &format!("{}^", result.commits[1].sha)]
            ),
            result.commits[0].sha
        );
    }

    #[test]
    fn ensure_change_ids_moves_a_branch_that_is_not_checked_out() {
        let repo = TempRepo::with_remote();
        sh(&repo.work, &["checkout", "-q", "-b", "feature"]);
        commit_file(&repo, "a.txt", "feat: a");
        sh(&repo.work, &["checkout", "-q", "main"]);
        let git = Git::new(&RUNNER, &repo.work);
        let raw = git.rev_list("origin/main", "refs/heads/feature").unwrap();
        let result = git.ensure_change_ids(&raw, "refs/heads/feature").unwrap();
        assert_eq!(
            sh(&repo.work, &["rev-parse", "feature"]),
            result.commits[0].sha
        );
        assert_eq!(
            sh(&repo.work, &["rev-parse", "HEAD"]),
            sh(&repo.work, &["rev-parse", "origin/main"])
        );
    }

    #[test]
    fn ensure_change_ids_on_detached_head() {
        let repo = TempRepo::with_remote();
        sh(&repo.work, &["checkout", "-q", "--detach"]);
        commit_file(&repo, "a.txt", "feat: a");
        let git = Git::new(&RUNNER, &repo.work);
        let result = git
            .ensure_change_ids(&git.rev_list("origin/main", "HEAD").unwrap(), "HEAD")
            .unwrap();
        assert_eq!(
            sh(&repo.work, &["rev-parse", "HEAD"]),
            result.commits[0].sha
        );
    }

    #[test]
    fn push_is_atomic_and_leased() {
        let repo = TempRepo::with_remote();
        sh(&repo.work, &["checkout", "-q", "-b", "feature"]);
        let a = commit_file(&repo, "a.txt", "feat: a");
        let b = commit_file(&repo, "b.txt", "feat: b");
        let git = Git::new(&RUNNER, &repo.work);

        git.push(
            "origin",
            &[
                Push {
                    branch: "prrit/alice/aaaaaaaa".into(),
                    sha: a.clone(),
                    expected_remote_sha: None,
                },
                Push {
                    branch: "prrit/alice/bbbbbbbb".into(),
                    sha: b.clone(),
                    expected_remote_sha: None,
                },
            ],
        )
        .unwrap();
        let refs = git.ls_remote("origin", "prrit/alice/").unwrap();
        assert_eq!(refs.get("prrit/alice/aaaaaaaa"), Some(&a));
        assert_eq!(refs.get("prrit/alice/bbbbbbbb"), Some(&b));

        // Remote moved under us: the stale lease must be rejected.
        sh(
            &repo.work,
            &[
                "push",
                "-q",
                "-f",
                "origin",
                &format!("{b}:refs/heads/prrit/alice/aaaaaaaa"),
            ],
        );
        let stale = Push {
            branch: "prrit/alice/aaaaaaaa".into(),
            sha: a,
            expected_remote_sha: Some("0".repeat(40)),
        };
        assert!(git.push("origin", &[stale]).is_err());
        assert_eq!(
            git.ls_remote("origin", "prrit/alice/").unwrap()["prrit/alice/aaaaaaaa"],
            b
        );

        git.push("origin", &[]).unwrap();
    }

    #[test]
    fn config_and_remotes() {
        let repo = TempRepo::with_remote();
        let git = Git::new(&RUNNER, &repo.work);
        assert_eq!(git.get_config("prrit.user"), None);
        git.set_config("prrit.user", "alice").unwrap();
        assert_eq!(git.get_config("prrit.user").as_deref(), Some("alice"));
        assert_eq!(git.remote_url("review"), None);
        git.add_remote("review", "prrit::origin").unwrap();
        assert_eq!(git.remote_url("review").as_deref(), Some("prrit::origin"));
        assert!(git.hooks_dir().unwrap().ends_with(".git/hooks"));
    }
}
