---
name: prrit
description: Publish a series of local commits as one GitHub pull request per commit, stacked with GitHub's native stacked PRs, using Gerrit-style `git push review HEAD:refs/for/<base>`. Use when asked for stacked PRs, one PR per commit, a review stack, refs/for, or when a branch with several independent commits should become separate PRs.
---

# prrit: one commit, one PR

prrit turns every commit on top of the base branch into its own PR and links
them into a GitHub stack. Local commits are the source of truth: amend or
reorder locally, push again, and the same PRs update.

## Check the setup first

```sh
prrit --help                          # both prrit and git-remote-prrit must be on PATH
gh extension list | grep stack        # needs github/gh-stack
git remote get-url review             # prrit::<remote> means the repo is initialised
```

If the `review` remote is missing, run `prrit init` (add `--push-default` only if
the user wants a bare `git push` to upload; it changes where every push goes).
`init` also installs a `commit-msg` hook that adds a `Change-Id` trailer.

## Workflow

```sh
git fetch origin
git checkout -b feature origin/main
# one logical change per commit; the hook adds Change-Id trailers
git commit ...
git commit ...
git push review                       # = git push review HEAD:refs/for/main
prrit status                          # PR number/state per commit, * = not pushed
```

Fixing things after review is plain git followed by the same push:

```sh
git rebase -i origin/main             # reword / edit / reorder / squash / drop
git commit --amend                    # or --amend --no-edit
git push review
```

Destination ref options (Gerrit syntax):

```sh
git push review HEAD:refs/for/develop        # stack onto another base
git push review HEAD:refs/for/main%wip       # create new PRs as drafts
git push review feature:refs/for/main        # upload a branch that is not checked out
```

## Rules that keep the stack healthy

- Identity is the `Change-Id` trailer. Never run `git commit --amend -m "..."`
  or otherwise rewrite the message without the trailer; the commit becomes a
  new PR and the old one is detached from the stack. Use `--amend` with the
  editor, `--amend --no-edit`, or `rebase -i` `reword`.
- Do not `git push origin feature` for a stack. Only `git push review` (or the
  explicit `HEAD:refs/for/<base>`) creates and updates the PRs.
- History must be linear; merge commits are rejected.
- Keep commits self-contained: each PR is reviewed and can be merged on its own,
  bottom first.
- After a PR merges: `git fetch origin && git rebase origin/main`. git drops the
  merged commit by patch-id; there is no sync command.
- If `prrit status` or a push warns that a branch "changed on GitHub", someone
  rebased or applied a suggestion on GitHub. Either adopt it first
  (`git fetch origin && git reset --hard origin/prrit/<login>/<top branch>`)
  or accept that the push overwrites it.
- PRs listed under "not in the current stack" are earlier changes that vanished
  locally; close them by hand if unwanted (`gh pr close <n>`).

## Reading `prrit status`

```
Stack on origin/main (bottom to top, * = not pushed):
  3f2a1c9  #41 OPEN     feat: add token parser
  9b8d7e6  (new)        feat: wire parser into CLI *
```

`(new)` means no PR yet; `*` means the local commit differs from what is on
GitHub. Warnings name merged commits still in the series and PRs the next push
will detach.

## Things prrit does not do

- No `sync`, `land`, or `abandon` commands: use `git rebase`, GitHub's merge
  button (or `gh stack merge`), and `gh pr close`.
- It never closes PRs or deletes remote branches.
