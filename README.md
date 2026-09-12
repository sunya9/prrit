# prrit

Gerrit-style stacked pull requests for GitHub: one commit, one PR, uploaded with `git push review`.
prrit manages branches, pushes, and PRs itself, and delegates only the stacking on GitHub to `gh stack link`.

## Requirements

- `git`, `gh` (logged in), and `gh extension install github/gh-stack`
- `prrit` and `git-remote-prrit` on your PATH; `cargo install --path .` installs both

## Usage

```sh
prrit init                    # commit-msg hook + "review" remote
git checkout -b feature origin/main
git commit ...                # stack up as many commits as you like
git push review               # = git push review HEAD:refs/for/main
git rebase -i origin/main     # amend / reorder, then
git push review               # the same PRs are updated, bases corrected
prrit status                  # PR for each commit, * marks unpushed commits
```

Like Gerrit, the destination ref picks the base branch and options:

```sh
git push review HEAD:refs/for/develop       # stack onto develop
git push review HEAD:refs/for/main%wip      # create new PRs as drafts
git push review feature:refs/for/main       # upload a branch that is not checked out
```

### What `prrit init [remote] [base]` sets up

- A `commit-msg` hook that adds a `Change-Id` trailer (the stable identity of a commit across amends and rebases)
- A `review` remote whose URL is `prrit::<remote>`, so git hands pushes to `git-remote-prrit`
- `remote.review.push = HEAD:refs/for/<base>`, so `git push review` needs no refspec; nothing else about pushing changes
- With `--push-default`, also `remote.pushDefault = review`, so a bare `git push` uploads the stack while fetch and pull keep using `<remote>`
- `--review-remote <name>` picks another name than `review` (say `gerrit`, or one helper remote per upstream in a fork setup); the rest of prrit does not depend on the name

### What a push does

- Fetches `<remote>/<base>` and takes every commit in `<remote>/<base>..<src>`
- Adds a `Change-Id` to commits that lack one (tree and author are preserved, the ref is moved)
- Pushes each commit as branch `prrit/<login>/<first 8 chars of Change-Id>` with `--force-with-lease --atomic`, skipping branches that are already up to date
- Creates a PR per commit (base = previous commit's branch, bottom = `<base>`); runs `gh pr edit` when title, body, or base drifted
- If the stack on GitHub still contains a PR whose commit is gone locally, dissolves that stack and re-creates it without the PR; the PR itself stays open as an unrelated change, as it would in Gerrit (`gh stack link` cannot drop members on its own, not even closed ones)
- With two or more PRs, runs `gh stack link --base <base> <PR#...>` to stack them on GitHub

### If you rebased or applied suggestions on GitHub

Local commits are the source of truth: a push force-updates every branch, so changes made on GitHub (the stack's Rebase button, an applied review suggestion) would be overwritten. prrit notices a remote tip it never pushed and tells you, in `status` and again right before pushing, how to adopt the GitHub version instead:

```sh
git fetch origin && git reset --hard origin/prrit/<login>/<top branch>
```

The push still goes through; the previous tip's sha is in the message and stays fetchable for a while.

### Keep the Change-Id when amending

The `Change-Id` trailer is the identity of a commit. `git commit --amend -m "..."` replaces the whole message and drops it, so the commit becomes a new PR and the old one is detached from the stack on the next push (it stays open; close it by hand if it is not wanted). Amend with the editor (`git commit --amend`) or `--no-edit`, or use `git rebase -i` with `reword`, and the trailer is kept.

### `prrit status [remote] [base]`

Read-only. Lists each commit on top of `<remote>/<base>` with its PR number and state, marks commits whose branch is not pushed yet, warns about commits whose PR is already merged, shows PRs the next push will detach from the stack, and lists open PRs of yours under `prrit/<login>/` that belong to no stack of this series (candidates to close).

## Development

```sh
cargo test                 # unit tests, real-git integration tests, and an end-to-end test with a fake gh
cargo clippy --all-targets
cargo build --release      # target/release/prrit and target/release/git-remote-prrit
cargo install --path .     # puts both binaries in ~/.cargo/bin
```

Runtime dependencies are `serde` and `serde_json` only; everything else is std plus the `git` and `gh` executables.

## Limitations

- Linear history only (merge commits are rejected)
- `git fetch review` is not supported; fetch from the real remote
- PRs that belong to another stack of yours are never touched; orphans outside any stack show up in `status` and are closed by hand
- Remote branches of detached PRs are left behind under `prrit/<login>/`
- After a PR merges, a normal `git rebase origin/main` drops the merged commit (git matches it by patch-id), so there is no sync command
