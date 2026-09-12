use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use crate::plan::{Pr, Stack};
use crate::runner::{RunOptions, Runner};

pub struct CreatePr<'a> {
    pub head: &'a str,
    pub base: &'a str,
    pub title: &'a str,
    pub body: &'a str,
    pub draft: bool,
}

#[derive(Default)]
pub struct EditPr<'a> {
    pub title: Option<&'a str>,
    pub body: Option<&'a str>,
    pub base: Option<&'a str>,
}

impl EditPr<'_> {
    pub fn is_empty(&self) -> bool {
        self.title.is_none() && self.body.is_none() && self.base.is_none()
    }
}

pub struct CreatedPr {
    pub number: u64,
    pub url: String,
}

const PR_FIELDS: &str = "number,state,headRefName,baseRefName,title,body,url,isDraft";

pub struct Gh<'a> {
    runner: &'a dyn Runner,
    cwd: PathBuf,
}

impl<'a> Gh<'a> {
    pub fn new(runner: &'a dyn Runner, cwd: &Path) -> Self {
        Gh {
            runner,
            cwd: cwd.to_path_buf(),
        }
    }

    fn gh(&self, args: &[&str]) -> Result<String> {
        self.runner.run(
            "gh",
            args,
            &RunOptions {
                cwd: Some(&self.cwd),
                ..Default::default()
            },
        )
    }

    pub fn login(&self) -> Result<String> {
        Ok(self
            .gh(&["api", "user", "--jq", ".login"])?
            .trim()
            .to_string())
    }

    pub fn default_branch(&self) -> Result<String> {
        Ok(self
            .gh(&[
                "repo",
                "view",
                "--json",
                "defaultBranchRef",
                "--jq",
                ".defaultBranchRef.name",
            ])?
            .trim()
            .to_string())
    }

    pub fn list_prs(&self) -> Result<Vec<Pr>> {
        let out = self.gh(&[
            "pr", "list", "--author", "@me", "--state", "all", "--limit", "300", "--json",
            PR_FIELDS,
        ])?;
        Ok(serde_json::from_str(&out)?)
    }

    pub fn create_pr(&self, input: &CreatePr<'_>) -> Result<CreatedPr> {
        let mut args = vec![
            "pr",
            "create",
            "--head",
            input.head,
            "--base",
            input.base,
            "--title",
            input.title,
            "--body",
            input.body,
        ];
        if input.draft {
            args.push("--draft");
        }
        let url = self.gh(&args)?.trim().to_string();
        let number = url
            .rsplit('/')
            .next()
            .and_then(|n| n.parse().ok())
            .ok_or_else(|| Error::msg(format!("could not parse PR number from: {url}")))?;
        Ok(CreatedPr { number, url })
    }

    pub fn edit_pr(&self, number: u64, input: &EditPr<'_>) -> Result<()> {
        let n = number.to_string();
        let mut args = vec!["pr", "edit", n.as_str()];
        if let Some(t) = input.title {
            args.extend(["--title", t]);
        }
        if let Some(b) = input.body {
            args.extend(["--body", b]);
        }
        if let Some(b) = input.base {
            args.extend(["--base", b]);
        }
        self.gh(&args).map(drop)
    }

    // gh expands {owner}/{repo} from the current repository.
    pub fn list_stacks(&self) -> Result<Vec<Stack>> {
        let out = self.gh(&[
            "api",
            "--paginate",
            "--slurp",
            "repos/{owner}/{repo}/stacks",
        ])?;
        let pages: Vec<Vec<Stack>> = serde_json::from_str(&out)?;
        Ok(pages.into_iter().flatten().collect())
    }

    /// Dissolves a stack on GitHub; the PRs themselves stay as they are.
    pub fn unstack(&self, stack_number: u64) -> Result<()> {
        let path = format!("repos/{{owner}}/{{repo}}/stacks/{stack_number}/unstack");
        self.gh(&["api", "--method", "POST", &path]).map(drop)
    }

    pub fn stack_link(&self, base: &str, pr_numbers: &[u64]) -> Result<()> {
        let numbers: Vec<String> = pr_numbers.iter().map(|n| n.to_string()).collect();
        let mut args = vec!["stack", "link", "--base", base];
        args.extend(numbers.iter().map(String::as_str));
        self.runner
            .run(
                "gh",
                &args,
                &RunOptions {
                    cwd: Some(&self.cwd),
                    inherit: true,
                    ..Default::default()
                },
            )
            .map(drop)
    }

    pub fn has_stack_extension(&self) -> bool {
        self.gh(&["extension", "list"])
            .map(|out| out.lines().any(|l| l.starts_with("gh stack\t")))
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::fake::FakeRunner;

    fn gh(runner: &FakeRunner) -> Gh<'_> {
        Gh::new(runner, Path::new("/repo"))
    }

    #[test]
    fn login_uses_the_api() {
        let runner =
            FakeRunner::default().on(|_, args| (args[0] == "api").then(|| "octocat\n".to_string()));
        assert_eq!(gh(&runner).login().unwrap(), "octocat");
        assert_eq!(runner.joined("gh"), vec!["api user --jq .login"]);
    }

    #[test]
    fn list_prs_parses_json() {
        let json = r#"[{"number":7,"state":"OPEN","headRefName":"prrit/octocat/abcdef01","baseRefName":"main","title":"t","body":"b","url":"https://github.com/o/r/pull/7","isDraft":true}]"#;
        let runner = FakeRunner::default().on(move |_, _| Some(json.to_string()));
        let prs = gh(&runner).list_prs().unwrap();
        assert_eq!(prs.len(), 1);
        assert_eq!(prs[0].number, 7);
        assert_eq!(prs[0].head_ref_name, "prrit/octocat/abcdef01");
        assert!(prs[0].is_draft);
        assert!(runner.joined("gh")[0].contains("--author @me"));
    }

    #[test]
    fn create_pr_returns_number_and_url() {
        let runner =
            FakeRunner::default().on(|_, _| Some("https://github.com/o/r/pull/42\n".to_string()));
        let created = gh(&runner)
            .create_pr(&CreatePr {
                head: "h",
                base: "main",
                title: "feat: x",
                body: "",
                draft: true,
            })
            .unwrap();
        assert_eq!(created.number, 42);
        assert_eq!(created.url, "https://github.com/o/r/pull/42");
        assert_eq!(
            runner.args_of("gh", "pr")[0],
            vec![
                "pr", "create", "--head", "h", "--base", "main", "--title", "feat: x", "--body",
                "", "--draft"
            ]
        );
    }

    #[test]
    fn edit_pr_passes_only_changed_fields() {
        let runner = FakeRunner::default().on(|_, _| Some(String::new()));
        gh(&runner)
            .edit_pr(
                42,
                &EditPr {
                    base: Some("main"),
                    ..Default::default()
                },
            )
            .unwrap();
        gh(&runner)
            .edit_pr(
                42,
                &EditPr {
                    title: Some("t"),
                    body: Some("b"),
                    base: None,
                },
            )
            .unwrap();
        assert_eq!(
            runner.joined("gh"),
            vec!["pr edit 42 --base main", "pr edit 42 --title t --body b"]
        );
    }

    #[test]
    fn stacks_and_unstack() {
        let runner = FakeRunner::default().on(|_, args| {
            if args.contains(&"--paginate".to_string()) {
                Some(r#"[[{"number":1,"open":true,"pull_requests":[{"number":5,"state":"open"}]}],[{"number":2,"open":true,"pull_requests":[]}]]"#.to_string())
            } else {
                Some(String::new())
            }
        });
        let stacks = gh(&runner).list_stacks().unwrap();
        assert_eq!(
            stacks.iter().map(|s| s.number).collect::<Vec<_>>(),
            vec![1, 2]
        );
        assert_eq!(stacks[0].pull_requests[0].number, 5);
        gh(&runner).unstack(7).unwrap();
        assert_eq!(
            runner.joined("gh")[1],
            "api --method POST repos/{owner}/{repo}/stacks/7/unstack"
        );
    }

    #[test]
    fn stack_link_streams_output() {
        let runner = FakeRunner::default().on(|_, _| Some(String::new()));
        gh(&runner).stack_link("main", &[11, 12]).unwrap();
        let calls = runner.calls.borrow();
        assert_eq!(
            calls[0].args,
            vec!["stack", "link", "--base", "main", "11", "12"]
        );
        assert!(calls[0].inherit);
    }

    #[test]
    fn detects_stack_extension() {
        let with = FakeRunner::default()
            .on(|_, _| Some("gh stack\tgithub/gh-stack\tv0.1.1\n".to_string()));
        assert!(gh(&with).has_stack_extension());
        let without = FakeRunner::default().on(|_, _| Some(String::new()));
        assert!(!gh(&without).has_stack_extension());
    }
}
