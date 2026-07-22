# CLAUDE.md

## Git workflow

Every change to this repo lands through a PR against the default branch
(`main`), never a direct push — small docs-only changes included, not
just code.

On green CI, merge with **"Create a merge commit"** ("merge and sync") —
never squash-merge or rebase-merge. Full commit history is preserved
deliberately.

If CI fails, diagnose and push a fix (or report the blocker) before
merging — don't merge red, and don't bypass the check.

See CONTRIBUTING.md for the full contribution workflow and code style, and
RELEASE_NOTES.md's own instructions for keeping it current after every
merged change.
