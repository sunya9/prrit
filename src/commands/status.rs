use crate::context::{resolve_base, resolve_login, Context};
use crate::error::Result;
use crate::format::{format_entry, format_orphans};
use crate::plan::{build_plan, divergence_warning, prefix_for, remote_divergence, Action, Commit, PlanInput};

pub struct StatusOptions<'a> {
    pub remote: &'a str,
    pub base: Option<&'a str>,
}

pub fn status(ctx: &Context<'_>, opts: &StatusOptions<'_>) -> Result<()> {
    let base = resolve_base(ctx, opts.remote, opts.base)?;
    ctx.git.fetch(opts.remote, &base)?;
    let upstream = format!("{}/{base}", opts.remote);
    let raw = ctx.git.rev_list(&upstream, "HEAD")?;
    if raw.is_empty() {
        ctx.out(format!("Nothing on top of {upstream}."));
        return Ok(());
    }

    let login = resolve_login(ctx)?;
    let mut with_id = Vec::new();
    let mut without_id = Vec::new();
    for c in &raw {
        match &c.change_id {
            Some(id) => with_id.push(Commit { sha: c.sha.clone(), message: c.message.clone(), change_id: id.clone() }),
            None => without_id.push(c.sha[..7.min(c.sha.len())].to_string()),
        }
    }

    let remote_refs = ctx.git.ls_remote(opts.remote, &prefix_for(&login))?;
    let prs = ctx.gh.list_prs()?;
    let stacks = ctx.gh.list_stacks()?;
    let plan = build_plan(PlanInput {
        login: &login,
        base: &base,
        commits: &with_id,
        remote_refs: &remote_refs,
        prs: &prs,
        stacks: &stacks,
    });

    ctx.out(format!("Stack on {upstream} (bottom to top, * = not pushed):"));
    for entry in &plan.entries {
        ctx.out(format_entry(entry));
    }
    if !without_id.is_empty() {
        ctx.out(format!(
            "{} commit(s) without Change-Id (added on next push): {}",
            without_id.len(),
            without_id.join(" ")
        ));
    }
    for w in &plan.warnings {
        ctx.err(format!("warning: {w}"));
    }
    let diverged = remote_divergence(&plan.pushes, |sha| ctx.git.has_commit(sha));
    if let (false, Some(top)) = (diverged.is_empty(), plan.entries.iter().rev().find(|e| e.action != Action::Skip)) {
        ctx.err(format!("warning: {}", divergence_warning(&diverged, opts.remote, &top.branch, false)));
    }
    if !plan.superseded.is_empty() {
        ctx.out("PRs still in the stack but no longer in the series (detached on next push):");
        for p in &plan.superseded {
            ctx.out(format!("  #{} {}  {}", p.number, p.title, p.url));
        }
    }
    for line in format_orphans(&plan.orphans) {
        ctx.out(line);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::upload::fixture::{context, pr, runner, State};
    use crate::context::recorder::Recorder;

    #[test]
    fn shows_each_commit_read_only() {
        let a = "a".repeat(40);
        let branch_a = "prrit/octocat/11111111";
        let r = runner(State {
            commits: vec![
                (a.clone(), format!("feat: a\n\nChange-Id: I{}\n", "1".repeat(40))),
                ("b".repeat(40), "feat: b\n".into()),
            ],
            remote_refs: vec![(branch_a.into(), "0".repeat(40))],
            prs: vec![pr(10, branch_a, "main", "feat: a"), pr(12, "prrit/octocat/deadbeef", "main", "old")],
            ..Default::default()
        });
        let rec = Recorder::default();
        status(&context(&r, &rec), &StatusOptions { remote: "origin", base: None }).unwrap();

        let text = rec.out_text();
        assert!(text.contains("aaaaaaa  #10 OPEN     feat: a *"), "{text}");
        assert!(text.contains("1 commit(s) without Change-Id"));
        assert!(text.contains("#12 old"));
        assert!(r.args_of("git", "push").is_empty());
        assert!(r.args_of("git", "commit-tree").is_empty());
        assert!(!rec.err_text().contains("changed on GitHub"));
        let writes: Vec<String> = r
            .joined("gh")
            .into_iter()
            .filter(|c| !(c.starts_with("pr list") || c.starts_with("api --paginate")))
            .collect();
        assert!(writes.is_empty(), "{writes:?}");
    }

    #[test]
    fn points_at_the_github_version_when_the_remote_diverged() {
        let branch_a = "prrit/octocat/11111111";
        let r = runner(State {
            commits: vec![("a".repeat(40), format!("feat: a\n\nChange-Id: I{}\n", "1".repeat(40)))],
            remote_refs: vec![(branch_a.into(), "f".repeat(40))],
            prs: vec![pr(10, branch_a, "main", "feat: a")],
            ..Default::default()
        })
        .fail_when(|cmd, args| cmd == "git" && args[0] == "cat-file");
        let rec = Recorder::default();
        status(&context(&r, &rec), &StatusOptions { remote: "origin", base: None }).unwrap();
        let err = rec.err_text();
        assert!(err.contains("the next push overwrites it"), "{err}");
        assert!(err.contains(&format!("git reset --hard origin/{branch_a}")), "{err}");
    }
}
