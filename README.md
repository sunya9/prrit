# prrit

Gerrit-style stacked pull requests for GitHub: one commit, one PR, uploaded with `git push review`.
prrit manages branches, pushes, and PRs itself, and delegates only the stacking on GitHub to `gh stack link`.

## Install

Pick one. Every method installs both `prrit` and `git-remote-prrit` (the name git looks for), and both must end up on your PATH.

**Homebrew** (macOS / Linux)

```sh
brew trust sunya9/tap
brew install sunya9/tap/prrit
```

**Install script** (prebuilt binary into `~/.local/bin`)

```sh
curl -fsSL https://github.com/sunya9/prrit/releases/latest/download/install.sh | sh
```

**cargo-binstall** (prebuilt binary via cargo)

```sh
cargo binstall prrit
```

**cargo install** (build from source)

```sh
cargo install prrit
```

Prebuilt binaries cover macOS (arm64, x86_64) and Linux (x86_64, aarch64, static musl). Archives and checksums are on the [releases page](https://github.com/sunya9/prrit/releases).

## Requirements

- `git`, `gh` (logged in), and `gh extension install github/gh-stack`

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

### Teaching an AI assistant

[`skills/prrit/SKILL.md`](skills/prrit/SKILL.md) is an [Agent Skill](https://agentskills.io) that explains the workflow and its pitfalls to coding agents (Claude Code, Codex, Cursor, ...). Install it with your agent's skill tooling, e.g. `npx skills add sunya9/prrit`, or copy it into the agent's skills directory; `prrit skill` prints the same file from any installed prrit.

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

### Releasing

Commits follow Conventional Commits. On every push to `main`, [release-plz](https://release-plz.dev) keeps a release PR open with the version bump and CHANGELOG; merging it publishes the crate to crates.io, tags `vX.Y.Z`, creates the GitHub release, and the same workflow builds the binaries, uploads archives plus `SHA256SUMS` and `install.sh`, and pushes the formula to the Homebrew tap. Repository settings involved:

- repository setting "Allow GitHub Actions to create and approve pull requests" (Settings → Actions → General), otherwise release-plz cannot open the release PR
- crates.io Trusted Publishing: on the crate's settings page add a GitHub publisher for `sunya9/prrit` with workflow `release-plz.yml`; no `CARGO_REGISTRY_TOKEN` is needed (a brand-new crate has to be published by hand once before this works)
- secret `RELEASE_PLZ_TOKEN` (optional PAT): lets CI run on the release PR
- variable `HOMEBREW_TAP_REPO` (e.g. `sunya9/homebrew-tap`) and secret `HOMEBREW_TAP_TOKEN` (PAT with contents write on that repo): enable the tap update; without them that job is skipped

If a release run fails halfway, rerun its failed jobs (`gh run rerun <run-id> --failed`); the binaries are always built from the tagged commit the run belongs to.

## Limitations

- Linear history only (merge commits are rejected)
- `git fetch review` is not supported; fetch from the real remote
- PRs that belong to another stack of yours are never touched; orphans outside any stack show up in `status` and are closed by hand
- Remote branches of detached PRs are left behind under `prrit/<login>/`
- After a PR merges, a normal `git rebase origin/main` drops the merged commit (git matches it by patch-id), so there is no sync command
