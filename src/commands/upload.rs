use crate::context::{resolve_login, Context};
use crate::error::{Error, Result};
use crate::format::{format_entry, format_orphans};
use crate::gh::{CreatePr, EditPr};
use crate::plan::{
    build_plan, divergence_warning, prefix_for, remote_divergence, Action, PlanInput, Pr,
};

pub struct UploadOptions<'a> {
    pub remote: &'a str,
    pub base: &'a str,
    pub draft: bool,
    /// Local ref whose commits are published; rewritten when Change-Ids are added.
    pub ref_name: &'a str,
}

pub fn upload(ctx: &Context<'_>, opts: &UploadOptions<'_>) -> Result<()> {
    let base = opts.base;
    ctx.git.fetch(opts.remote, base)?;
    let upstream = format!("{}/{base}", opts.remote);
    let raw = ctx.git.rev_list(&upstream, opts.ref_name)?;
    if raw.is_empty() {
        return Err(Error::msg(format!(
            "nothing to push: {} has no commits on top of {upstream}",
            opts.ref_name
        )));
    }

    let ensured = ctx.git.ensure_change_ids(&raw, opts.ref_name)?;
    if ensured.rewritten > 0 {
        ctx.out(format!(
            "Added Change-Id to {} commit(s).",
            ensured.rewritten
        ));
    }

    let login = resolve_login(ctx)?;
    let remote_refs = ctx.git.ls_remote(opts.remote, &prefix_for(&login))?;
    let prs = ctx.gh.list_prs()?;
    let stacks = ctx.gh.list_stacks()?;
    let mut plan = build_plan(PlanInput {
        login: &login,
        base,
        commits: &ensured.commits,
        remote_refs: &remote_refs,
        prs: &prs,
        stacks: &stacks,
    });

    let diverged = remote_divergence(&plan.pushes, |sha| ctx.git.has_commit(sha));
    if let (false, Some(top)) = (
        diverged.is_empty(),
        plan.entries.iter().rev().find(|e| e.action != Action::Skip),
    ) {
        ctx.err(format!(
            "warning: {}",
            divergence_warning(&diverged, opts.remote, &top.branch, true)
        ));
    }

    ctx.git.push(opts.remote, &plan.pushes)?;
    if !plan.pushes.is_empty() {
        ctx.out(format!(
            "Pushed {} branch(es) to {}.",
            plan.pushes.len(),
            opts.remote
        ));
    }

    let mut numbers = Vec::new();
    for entry in &mut plan.entries {
        match entry.action {
            Action::Skip => continue,
            Action::Create => {
                let created = ctx.gh.create_pr(&CreatePr {
                    head: &entry.branch,
                    base: &entry.base,
                    title: &entry.title,
                    body: &entry.body,
                    draft: opts.draft,
                })?;
                entry.pr = Some(Pr {
                    number: created.number,
                    state: "OPEN".into(),
                    head_ref_name: entry.branch.clone(),
                    base_ref_name: entry.base.clone(),
                    title: entry.title.clone(),
                    body: entry.body.clone(),
                    url: created.url,
                    is_draft: opts.draft,
                });
            }
            Action::Edit => {
                if let Some(pr) = &entry.pr {
                    let edit = EditPr {
                        title: (pr.title != entry.title).then_some(entry.title.as_str()),
                        body: (pr.body != entry.body).then_some(entry.body.as_str()),
                        base: (pr.base_ref_name != entry.base).then_some(entry.base.as_str()),
                    };
                    if !edit.is_empty() {
                        ctx.gh.edit_pr(pr.number, &edit)?;
                    }
                }
            }
            Action::Keep => {}
        }
        if let Some(pr) = &entry.pr {
            numbers.push(pr.number);
        }
        entry.pushed = true;
    }

    for stack in &plan.stale_stacks {
        ctx.gh.unstack(*stack)?;
    }
    if plan.needs_link {
        ctx.gh.stack_link(base, &numbers)?;
    }

    ctx.out(format!("Stack on {upstream}:"));
    for entry in &plan.entries {
        let url = entry
            .pr
            .as_ref()
            .map(|p| format!("  {}", p.url))
            .unwrap_or_default();
        ctx.out(format!("{}{url}", format_entry(entry)));
    }
    for w in &plan.warnings {
        ctx.err(format!("warning: {w}"));
    }
    for line in format_orphans(&plan.orphans) {
        ctx.out(line);
    }
    Ok(())
}

#[cfg(test)]
pub mod fixture {
    use crate::context::recorder::Recorder;
    use crate::context::Context;
    use crate::plan::{Pr, Stack};
    use crate::runner::fake::FakeRunner;
    use std::cell::Cell;
    use std::path::Path;
    use std::rc::Rc;

    #[derive(Default)]
    pub struct State {
        pub commits: Vec<(String, String)>,
        pub remote_refs: Vec<(String, String)>,
        pub prs: Vec<Pr>,
        pub stacks: Vec<Stack>,
    }

    pub fn pr(number: u64, head: &str, base: &str, title: &str) -> Pr {
        Pr {
            number,
            state: "OPEN".into(),
            head_ref_name: head.into(),
            base_ref_name: base.into(),
            title: title.into(),
            body: String::new(),
            url: format!("https://github.com/o/r/pull/{number}"),
            is_draft: false,
        }
    }

    pub fn runner(state: State) -> FakeRunner {
        let next_pr = Rc::new(Cell::new(500u64));
        let log: String = state
            .commits
            .iter()
            .map(|(sha, msg)| format!("{sha}\nparent\n{msg}\n\0"))
            .collect();
        let ls_remote: String = state
            .remote_refs
            .iter()
            .map(|(b, sha)| format!("{sha}\trefs/heads/{b}\n"))
            .collect();
        let prs = serde_json::to_string(&serde_json::json!(state
            .prs
            .iter()
            .map(|p| serde_json::json!({
                "number": p.number, "state": p.state, "headRefName": p.head_ref_name,
                "baseRefName": p.base_ref_name, "title": p.title, "body": p.body,
                "url": p.url, "isDraft": p.is_draft,
            }))
            .collect::<Vec<_>>()))
        .unwrap();
        let stacks = serde_json::to_string(&serde_json::json!([state
            .stacks
            .iter()
            .map(|s| serde_json::json!({
                "number": s.number, "open": s.open,
                "pull_requests": s.pull_requests.iter().map(|p| serde_json::json!({"number": p.number, "state": p.state})).collect::<Vec<_>>(),
            }))
            .collect::<Vec<_>>()]))
        .unwrap();

        FakeRunner::default()
            .on(move |cmd, args| {
                if cmd != "git" {
                    return None;
                }
                match args[0].as_str() {
                    "symbolic-ref" => Some(
                        if args[2] == "refs/remotes/origin/HEAD" {
                            "origin/main\n"
                        } else {
                            "feature\n"
                        }
                        .into(),
                    ),
                    "fetch" | "push" | "cat-file" => Some(String::new()),
                    "log" => Some(log.clone()),
                    "config" => Some(if args.get(2).map(String::as_str) == Some("--get") {
                        "octocat\n".into()
                    } else {
                        String::new()
                    }),
                    "ls-remote" => Some(ls_remote.clone()),
                    _ => None,
                }
            })
            .on(move |cmd, args| {
                if cmd != "gh" {
                    return None;
                }
                match (args[0].as_str(), args.get(1).map(String::as_str)) {
                    ("pr", Some("list")) => Some(prs.clone()),
                    ("pr", Some("create")) => {
                        let n = next_pr.get();
                        next_pr.set(n + 1);
                        Some(format!("https://github.com/o/r/pull/{n}\n"))
                    }
                    ("pr", Some("edit")) => Some(String::new()),
                    ("api", Some("--paginate")) => Some(stacks.clone()),
                    ("api", Some("--method")) => Some(String::new()),
                    ("stack", _) => Some(String::new()),
                    _ => None,
                }
            })
    }

    pub fn context<'a>(runner: &'a FakeRunner, rec: &'a Recorder) -> Context<'a> {
        Context::new(runner, Path::new("/repo"), rec)
    }
}

#[cfg(test)]
mod tests {
    use super::fixture::{context, pr, runner, State};
    use super::*;
    use crate::context::recorder::Recorder;
    use crate::plan::{Stack, StackPr};

    const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const ID_A: &str = "I1111111111111111111111111111111111111111";
    const ID_B: &str = "I2222222222222222222222222222222222222222";
    const BRANCH_A: &str = "prrit/octocat/11111111";
    const BRANCH_B: &str = "prrit/octocat/22222222";

    fn opts<'a>(
        remote: &'a str,
        base: &'a str,
        draft: bool,
        ref_name: &'a str,
    ) -> UploadOptions<'a> {
        UploadOptions {
            remote,
            base,
            draft,
            ref_name,
        }
    }

    #[test]
    fn first_push_pushes_creates_and_links() {
        let r = runner(State {
            commits: vec![
                (
                    A.into(),
                    format!("feat: a\n\nbody a\n\nChange-Id: {ID_A}\n"),
                ),
                (B.into(), format!("feat: b\n\nChange-Id: {ID_B}\n")),
            ],
            ..Default::default()
        });
        let rec = Recorder::default();
        upload(
            &context(&r, &rec),
            &opts("origin", "main", false, "refs/heads/feature"),
        )
        .unwrap();

        assert_eq!(
            r.args_of("git", "push"),
            vec![vec![
                "push".to_string(),
                "--quiet".into(),
                "--atomic".into(),
                format!("--force-with-lease=refs/heads/{BRANCH_A}:"),
                format!("--force-with-lease=refs/heads/{BRANCH_B}:"),
                "origin".into(),
                format!("{A}:refs/heads/{BRANCH_A}"),
                format!("{B}:refs/heads/{BRANCH_B}"),
            ]]
        );
        let gh = r.joined("gh");
        assert!(gh.contains(&format!(
            "pr create --head {BRANCH_A} --base main --title feat: a --body body a\n"
        )));
        assert!(gh.contains(&format!(
            "pr create --head {BRANCH_B} --base {BRANCH_A} --title feat: b --body "
        )));
        assert!(gh.contains(&"stack link --base main 500 501".to_string()));
        assert!(rec.out_text().contains("https://github.com/o/r/pull/500"));
    }

    #[test]
    fn honours_remote_base_draft_and_ref() {
        let r = runner(State {
            commits: vec![(A.into(), format!("feat: a\n\nChange-Id: {ID_A}\n"))],
            ..Default::default()
        });
        let rec = Recorder::default();
        upload(
            &context(&r, &rec),
            &opts("upstream", "develop", true, "refs/heads/x"),
        )
        .unwrap();
        assert_eq!(
            r.args_of("git", "fetch"),
            vec![vec!["fetch", "--quiet", "upstream", "develop"]]
        );
        assert!(r.args_of("git", "log")[0].contains(&"upstream/develop..refs/heads/x".to_string()));
        assert!(r
            .joined("gh")
            .iter()
            .any(|c| c.starts_with("pr create") && c.ends_with("--draft")));
        assert!(r.args_of("gh", "stack").is_empty());
    }

    #[test]
    fn edits_only_drifted_prs_and_skips_pushed_branches() {
        let r = runner(State {
            commits: vec![
                (A.into(), format!("feat: a\n\nChange-Id: {ID_A}\n")),
                (B.into(), format!("feat: b renamed\n\nChange-Id: {ID_B}\n")),
            ],
            remote_refs: vec![
                (BRANCH_A.into(), A.into()),
                (BRANCH_B.into(), "0".repeat(40)),
            ],
            prs: vec![
                pr(10, BRANCH_A, "main", "feat: a"),
                pr(11, BRANCH_B, BRANCH_A, "feat: b"),
            ],
            ..Default::default()
        });
        let rec = Recorder::default();
        upload(&context(&r, &rec), &opts("origin", "main", false, "HEAD")).unwrap();
        assert_eq!(
            r.args_of("git", "push"),
            vec![vec![
                "push".to_string(),
                "--quiet".into(),
                "--atomic".into(),
                format!(
                    "--force-with-lease=refs/heads/{BRANCH_B}:{}",
                    "0".repeat(40)
                ),
                "origin".into(),
                format!("{B}:refs/heads/{BRANCH_B}"),
            ]]
        );
        let gh = r.joined("gh");
        let edits: Vec<&String> = gh.iter().filter(|c| c.starts_with("pr edit")).collect();
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0], "pr edit 11 --title feat: b renamed");
        assert!(r
            .joined("gh")
            .contains(&"stack link --base main 10 11".to_string()));
    }

    #[test]
    fn dissolves_stale_stack_before_linking_without_closing() {
        let r = runner(State {
            commits: vec![
                (A.into(), format!("feat: a\n\nChange-Id: {ID_A}\n")),
                (
                    B.into(),
                    format!("feat: b (amended)\n\nChange-Id: {ID_B}\n"),
                ),
            ],
            remote_refs: vec![(BRANCH_A.into(), A.into())],
            prs: vec![
                pr(1, BRANCH_A, "main", "feat: a"),
                pr(2, "prrit/octocat/deadbeef", BRANCH_A, "feat: b"),
            ],
            stacks: vec![Stack {
                number: 3,
                open: true,
                pull_requests: vec![
                    StackPr {
                        number: 1,
                        state: "open".into(),
                    },
                    StackPr {
                        number: 2,
                        state: "open".into(),
                    },
                ],
            }],
        });
        let rec = Recorder::default();
        upload(&context(&r, &rec), &opts("origin", "main", false, "HEAD")).unwrap();
        let gh = r.joined("gh");
        let unstack = gh
            .iter()
            .position(|c| c == "api --method POST repos/{owner}/{repo}/stacks/3/unstack")
            .unwrap();
        let link = gh.iter().position(|c| c.starts_with("stack link")).unwrap();
        assert!(link > unstack);
        assert!(!gh.iter().any(|c| c.starts_with("pr close")));
        assert_eq!(gh[link], "stack link --base main 1 500");
        assert!(rec.err_text().contains("#2 no longer in the series"));
    }

    #[test]
    fn warns_when_the_remote_tip_was_changed_on_github_but_still_pushes() {
        let github_sha = "f".repeat(40);
        let gs = github_sha.clone();
        let r = runner(State {
            commits: vec![(A.into(), format!("feat: a\n\nChange-Id: {ID_A}\n"))],
            remote_refs: vec![(BRANCH_A.into(), github_sha.clone())],
            prs: vec![pr(1, BRANCH_A, "main", "feat: a")],
            ..Default::default()
        })
        .fail_when(move |cmd, args| {
            cmd == "git" && args[0] == "cat-file" && args[2].starts_with(&gs)
        });
        let rec = Recorder::default();
        upload(&context(&r, &rec), &opts("origin", "main", false, "HEAD")).unwrap();
        let err = rec.err_text();
        assert!(
            err.contains(&format!("{BRANCH_A} (was fffffff) changed on GitHub")),
            "{err}"
        );
        assert!(
            err.contains(&format!("git reset --hard origin/{BRANCH_A}")),
            "{err}"
        );
        assert_eq!(r.args_of("git", "push").len(), 1);
    }

    #[test]
    fn no_warning_for_our_own_previous_push() {
        let r = runner(State {
            commits: vec![(A.into(), format!("feat: a\n\nChange-Id: {ID_A}\n"))],
            remote_refs: vec![(BRANCH_A.into(), "0".repeat(40))],
            prs: vec![pr(1, BRANCH_A, "main", "feat: a")],
            ..Default::default()
        });
        let rec = Recorder::default();
        upload(&context(&r, &rec), &opts("origin", "main", false, "HEAD")).unwrap();
        assert!(!rec.err_text().contains("changed on GitHub"));
    }

    #[test]
    fn nothing_to_push_is_an_error_before_any_push() {
        let r = runner(State::default());
        let rec = Recorder::default();
        let err = upload(&context(&r, &rec), &opts("origin", "main", false, "HEAD")).unwrap_err();
        assert!(err.to_string().contains("nothing to push"));
        assert!(r.args_of("git", "push").is_empty());
    }
}
