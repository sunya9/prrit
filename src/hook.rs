pub const HOOK_MARKER: &str = "# prrit commit-msg hook";

/// Gerrit-style commit-msg hook. Kept dependency-free (POSIX sh + git) so the
/// hook keeps working even if the prrit binary moves or is uninstalled.
pub const COMMIT_MSG_HOOK: &str = r#"#!/bin/sh
# prrit commit-msg hook
# Adds a Change-Id trailer so prrit can map a commit to its PR across amends.
MSG="$1"

if git interpret-trailers --parse "$MSG" | grep -q '^Change-Id: '; then
  exit 0
fi

# Nothing to do for an empty or comment-only message; git aborts the commit.
if ! grep -qv '^#' "$MSG" || ! grep -q '[^[:space:]]' "$MSG"; then
  exit 0
fi

ID="I$( {
  git write-tree 2>/dev/null
  git rev-parse -q --verify HEAD 2>/dev/null
  date +%s
  echo "$$"
  head -n 1 "$MSG"
} | git hash-object --stdin)"

git interpret-trailers --in-place --trailer "Change-Id: $ID" "$MSG"
"#;
