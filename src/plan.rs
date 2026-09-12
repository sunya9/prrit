//! Pure planning: local commits + remote refs + PRs + stacks -> what to do.

use serde::Deserialize;

use crate::change_id::{branch_name, strip_change_id, subject_of};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Commit {
    pub sha: String,
    pub change_id: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Pr {
    pub number: u64,
    pub state: String,
    pub head_ref_name: String,
    pub base_ref_name: String,
    pub title: String,
    pub body: String,
    pub url: String,
    pub is_draft: bool,
}

/// Shape of GET /repos/{owner}/{repo}/stacks (the API gh-stack itself uses).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Stack {
    pub number: u64,
    pub open: bool,
    pub pull_requests: Vec<StackPr>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct StackPr {
    pub number: u64,
    pub state: String,
}

pub struct PlanInput<'a> {
    pub login: &'a str,
    pub base: &'a str,
    pub commits: &'a [Commit],
    pub remote_refs: &'a std::collections::HashMap<String, String>,
    pub prs: &'a [Pr],
    pub stacks: &'a [Stack],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Push {
    pub branch: String,
    pub sha: String,
    pub expected_remote_sha: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Create,
    Edit,
    Keep,
    Skip,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub sha: String,
    pub change_id: String,
    pub branch: String,
    pub base: String,
    pub title: String,
    pub body: String,
    pub action: Action,
    pub pr: Option<Pr>,
    pub pushed: bool,
}

#[derive(Debug, Default)]
pub struct Plan {
    pub pushes: Vec<Push>,
    pub entries: Vec<Entry>,
    /// Open PRs of ours that belong to no known stack of this series.
    pub orphans: Vec<Pr>,
    /// Members of this series' GitHub stack whose commit is gone locally
    /// (typically a Change-Id lost to `commit --amend -m`). Like Gerrit they
    /// stay open as unrelated changes, but gh stack link refuses to drop
    /// members, even closed ones, so those stacks are dissolved and re-created.
    pub superseded: Vec<Pr>,
    pub stale_stacks: Vec<u64>,
    pub warnings: Vec<String>,
    pub needs_link: bool,
}

/// Branches about to be overwritten whose remote tip is unknown locally, i.e.
/// changed on GitHub (a UI rebase, an applied suggestion) rather than by us.
pub fn remote_divergence(pushes: &[Push], is_local: impl Fn(&str) -> bool) -> Vec<&Push> {
    pushes
        .iter()
        .filter(|p| p.expected_remote_sha.as_deref().is_some_and(|sha| !is_local(sha)))
        .collect()
}

pub fn divergence_warning(diverged: &[&Push], remote: &str, top_branch: &str, overwriting: bool) -> String {
    let list: Vec<String> = diverged
        .iter()
        .map(|p| format!("{} (was {})", p.branch, &p.expected_remote_sha.as_deref().unwrap_or("")[..7]))
        .collect();
    let verb = if overwriting { "overwriting it" } else { "the next push overwrites it" };
    format!(
        "{} changed on GitHub (rebased there, or a suggestion was applied); {verb}.          To adopt the GitHub version instead: git fetch {remote} && git reset --hard {remote}/{top_branch}.          The previous tip stays fetchable by sha for a while.",
        list.join(", ")
    )
}

pub fn prefix_for(login: &str) -> String {
    format!("prrit/{login}/")
}

fn body_of(message: &str) -> String {
    let stripped = strip_change_id(message);
    match stripped.split_once('\n') {
        Some((_, rest)) => rest.trim_start_matches('\n').to_string(),
        None => String::new(),
    }
}

// Open PR wins; a merged PR means the commit is stale; a closed-only PR is
// treated as absent so the change can be re-proposed.
fn pr_for<'a>(branch: &str, prs: &'a [Pr]) -> Option<&'a Pr> {
    let candidates: Vec<&Pr> = prs.iter().filter(|p| p.head_ref_name == branch).collect();
    candidates
        .iter()
        .find(|p| p.state == "OPEN")
        .or_else(|| candidates.iter().find(|p| p.state == "MERGED"))
        .copied()
}

pub fn build_plan(input: PlanInput<'_>) -> Plan {
    let mut plan = Plan::default();
    let mut previous_branch = input.base.to_string();

    for commit in input.commits {
        let branch = branch_name(input.login, &commit.change_id);
        let title = subject_of(&commit.message).to_string();
        let body = body_of(&commit.message);
        let pr = pr_for(&branch, input.prs).cloned();
        let remote_sha = input.remote_refs.get(&branch);
        let pushed = remote_sha.is_some_and(|s| *s == commit.sha);

        if let Some(p) = &pr {
            if p.state == "MERGED" {
                plan.warnings.push(format!(
                    "{} is already merged as #{}; rebase onto {} to drop it",
                    &commit.sha[..7.min(commit.sha.len())],
                    p.number,
                    input.base
                ));
                plan.entries.push(Entry {
                    sha: commit.sha.clone(),
                    change_id: commit.change_id.clone(),
                    branch,
                    base: previous_branch.clone(),
                    title,
                    body,
                    action: Action::Skip,
                    pr,
                    pushed,
                });
                continue;
            }
        }

        if !pushed {
            plan.pushes.push(Push {
                branch: branch.clone(),
                sha: commit.sha.clone(),
                expected_remote_sha: remote_sha.cloned(),
            });
        }

        let action = match &pr {
            None => Action::Create,
            Some(p) if p.base_ref_name != previous_branch || p.title != title || p.body != body => Action::Edit,
            Some(_) => Action::Keep,
        };

        plan.entries.push(Entry {
            sha: commit.sha.clone(),
            change_id: commit.change_id.clone(),
            branch: branch.clone(),
            base: previous_branch.clone(),
            title,
            body,
            action,
            pr,
            pushed,
        });
        previous_branch = branch;
    }

    let in_series: std::collections::HashSet<u64> =
        plan.entries.iter().filter_map(|e| e.pr.as_ref().map(|p| p.number)).collect();
    let mut superseded_numbers = std::collections::HashSet::new();
    for stack in input.stacks {
        if !stack.pull_requests.iter().any(|p| in_series.contains(&p.number)) {
            continue;
        }
        // Merged members are simply history; anything else left behind blocks link.
        let left_behind: Vec<&StackPr> = stack
            .pull_requests
            .iter()
            .filter(|p| p.state != "merged" && !in_series.contains(&p.number))
            .collect();
        if left_behind.is_empty() {
            continue;
        }
        plan.stale_stacks.push(stack.number);
        superseded_numbers.extend(left_behind.iter().map(|p| p.number));
    }
    plan.superseded = input.prs.iter().filter(|p| superseded_numbers.contains(&p.number)).cloned().collect();
    if !plan.superseded.is_empty() {
        let list: Vec<String> = plan.superseded.iter().map(|p| format!("#{}", p.number)).collect();
        let tail = if plan.superseded.len() == 1 {
            "it (it stays open as an unrelated PR)"
        } else {
            "them (they stay open as unrelated PRs)"
        };
        plan.warnings.push(format!(
            "{} no longer in the series; re-creating the stack without {tail}. \
             A Change-Id is lost by `commit --amend -m`; amend with the editor or --no-edit to keep it.",
            list.join(", ")
        ));
    }

    let prefix = prefix_for(input.login);
    plan.orphans = input
        .prs
        .iter()
        .filter(|p| {
            p.state == "OPEN"
                && p.head_ref_name.starts_with(&prefix)
                && !in_series.contains(&p.number)
                && !superseded_numbers.contains(&p.number)
        })
        .cloned()
        .collect();

    plan.needs_link = plan.entries.iter().filter(|e| e.action != Action::Skip).count() >= 2;
    plan
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    const LOGIN: &str = "octocat";
    const BASE: &str = "main";

    fn commit(n: usize) -> Commit {
        commit_with(n, &format!("feat: change {n}\n\nbody {n}\n"))
    }

    fn commit_with(n: usize, message: &str) -> Commit {
        let change_id = format!("I{}", n.to_string().repeat(40));
        Commit {
            sha: n.to_string().repeat(40),
            change_id: change_id.clone(),
            message: format!("{message}\nChange-Id: {change_id}\n"),
        }
    }

    fn branch(n: usize) -> String {
        format!("prrit/{LOGIN}/{}", n.to_string().repeat(8))
    }

    fn pr(n: usize) -> Pr {
        Pr {
            number: 100 + n as u64,
            state: "OPEN".into(),
            head_ref_name: branch(n),
            base_ref_name: if n == 1 { BASE.into() } else { branch(n - 1) },
            title: format!("feat: change {n}"),
            body: format!("body {n}\n"),
            url: format!("https://github.com/o/r/pull/{}", 100 + n),
            is_draft: false,
        }
    }

    fn stack(number: u64, prs: &[(u64, &str)]) -> Stack {
        Stack {
            number,
            open: true,
            pull_requests: prs.iter().map(|(n, s)| StackPr { number: *n, state: s.to_string() }).collect(),
        }
    }

    fn plan(commits: &[Commit], remote: &[(String, String)], prs: &[Pr], stacks: &[Stack]) -> Plan {
        let remote_refs: HashMap<String, String> = remote.iter().cloned().collect();
        build_plan(PlanInput { login: LOGIN, base: BASE, commits, remote_refs: &remote_refs, prs, stacks })
    }

    fn actions(p: &Plan) -> Vec<Action> {
        p.entries.iter().map(|e| e.action).collect()
    }

    #[test]
    fn first_push_creates_everything() {
        let p = plan(&[commit(1), commit(2)], &[], &[], &[]);
        assert_eq!(
            p.pushes,
            vec![
                Push { branch: branch(1), sha: "1".repeat(40), expected_remote_sha: None },
                Push { branch: branch(2), sha: "2".repeat(40), expected_remote_sha: None },
            ]
        );
        assert_eq!(actions(&p), vec![Action::Create, Action::Create]);
        assert_eq!(p.entries[0].base, BASE);
        assert_eq!(p.entries[0].title, "feat: change 1");
        assert_eq!(p.entries[0].body, "body 1\n");
        assert_eq!(p.entries[1].base, branch(1));
        assert!(p.warnings.is_empty());
        assert!(p.needs_link);
    }

    #[test]
    fn pushes_only_changed_branches_with_lease() {
        let old = format!("{:0<40}", "old");
        let p = plan(
            &[commit(1), commit(2)],
            &[(branch(1), "1".repeat(40)), (branch(2), old.clone())],
            &[pr(1), pr(2)],
            &[],
        );
        assert_eq!(p.pushes, vec![Push { branch: branch(2), sha: "2".repeat(40), expected_remote_sha: Some(old) }]);
        assert_eq!(actions(&p), vec![Action::Keep, Action::Keep]);
    }

    #[test]
    fn edits_bases_after_reorder() {
        let p = plan(&[commit(2), commit(1)], &[], &[pr(1), pr(2)], &[]);
        let edits: Vec<(u64, &str)> = p
            .entries
            .iter()
            .filter(|e| e.action == Action::Edit)
            .map(|e| (e.pr.as_ref().unwrap().number, e.base.as_str()))
            .collect();
        assert_eq!(edits, vec![(102, BASE), (101, branch(2).as_str())]);
    }

    #[test]
    fn edits_title_and_body_when_message_changed() {
        let p = plan(&[commit_with(1, "feat: renamed\n\nnew body\n")], &[], &[pr(1)], &[]);
        assert_eq!(p.entries[0].action, Action::Edit);
        assert_eq!(p.entries[0].title, "feat: renamed");
        assert_eq!(p.entries[0].body, "new body\n");
    }

    #[test]
    fn skips_merged_and_warns() {
        let mut merged = pr(1);
        merged.state = "MERGED".into();
        let p = plan(&[commit(1), commit(2)], &[], &[merged], &[]);
        assert_eq!(actions(&p), vec![Action::Skip, Action::Create]);
        assert_eq!(p.pushes.iter().map(|x| x.branch.clone()).collect::<Vec<_>>(), vec![branch(2)]);
        assert_eq!(p.entries[1].base, BASE);
        assert!(p.warnings[0].contains("merged as #101"));
        assert!(!p.needs_link);
    }

    #[test]
    fn closed_only_pr_means_create_again() {
        let mut closed = pr(1);
        closed.state = "CLOSED".into();
        let p = plan(&[commit(1)], &[], &[closed], &[]);
        assert_eq!(p.entries[0].action, Action::Create);
    }

    #[test]
    fn single_commit_needs_no_link() {
        assert!(!plan(&[commit(1)], &[], &[], &[]).needs_link);
        assert!(plan(&[commit(1), commit(2)], &[], &[], &[]).needs_link);
    }

    #[test]
    fn orphans_are_open_prefixed_prs_outside_the_series() {
        let mut closed = pr(4);
        closed.state = "CLOSED".into();
        let mut other = pr(5);
        other.head_ref_name = "feature/other".into();
        let p = plan(&[commit(1)], &[], &[pr(1), pr(3), closed, other], &[]);
        assert_eq!(p.orphans.iter().map(|x| x.number).collect::<Vec<_>>(), vec![103]);
    }

    #[test]
    fn superseded_members_dissolve_only_their_stack() {
        let p = plan(
            &[commit(1), commit(3)],
            &[],
            &[pr(1), pr(2), pr(5)],
            &[stack(9, &[(101, "open"), (102, "open")]), stack(10, &[(105, "open"), (106, "open")])],
        );
        assert_eq!(p.superseded.iter().map(|x| x.number).collect::<Vec<_>>(), vec![102]);
        assert_eq!(p.stale_stacks, vec![9]);
        assert_eq!(p.orphans.iter().map(|x| x.number).collect::<Vec<_>>(), vec![105]);
        assert!(p.warnings[0].contains("#102 no longer in the series"));
    }

    #[test]
    fn divergence_is_a_remote_tip_we_never_had() {
        let pushes = vec![
            Push { branch: "b1".into(), sha: "1".repeat(40), expected_remote_sha: Some("a".repeat(40)) },
            Push { branch: "b2".into(), sha: "2".repeat(40), expected_remote_sha: Some("f".repeat(40)) },
            Push { branch: "b3".into(), sha: "3".repeat(40), expected_remote_sha: None },
        ];
        let diverged = remote_divergence(&pushes, |sha| sha.starts_with('a'));
        assert_eq!(diverged.iter().map(|p| p.branch.as_str()).collect::<Vec<_>>(), vec!["b2"]);
        let text = divergence_warning(&diverged, "origin", "prrit/octocat/top", true);
        assert!(text.contains("b2 (was fffffff)"));
        assert!(text.contains("git reset --hard origin/prrit/octocat/top"));
        assert!(text.contains("overwriting it"));
    }

    #[test]
    fn closed_members_count_as_superseded_but_merged_do_not() {
        let mut closed = pr(2);
        closed.state = "CLOSED".into();
        let p = plan(&[commit(1)], &[], &[pr(1), closed], &[stack(9, &[(101, "open"), (102, "closed")])]);
        assert_eq!(p.superseded.iter().map(|x| x.number).collect::<Vec<_>>(), vec![102]);
        assert_eq!(p.stale_stacks, vec![9]);

        let mut merged = pr(2);
        merged.state = "MERGED".into();
        let p = plan(&[commit(1)], &[], &[pr(1), merged], &[stack(9, &[(101, "open"), (102, "merged")])]);
        assert!(p.superseded.is_empty());
        assert!(p.stale_stacks.is_empty());
    }
}
