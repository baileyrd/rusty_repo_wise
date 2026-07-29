# CI and branch protection

CONTRIBUTING.md states two rules that the repo cannot enforce on its own:

> Every change lands through a PR — no direct pushes to the default branch.
> CI must be green before merge.

Both are settings on GitHub, not facts about the code. This document says what
the CI workflow is, how to make it the gate those rules assume, and how to check
whether the gate is actually on.

## The workflow

`.github/workflows/ci-rust.yml` defines a single job, **`check`**. That job name
is the *status check context* — the string branch protection matches on — so
renaming the job silently detaches any protection rule that names it. If you
rename it, update the protection rule in the same change.

It runs on every pull request and on every push to `main`, with
`cancel-in-progress` concurrency keyed on the ref.

Steps, in order:

| Step | Covers |
| --- | --- |
| Format | `cargo fmt --all -- --check` |
| Clippy | `cargo clippy --all-targets --all-features -- -D warnings` |
| Test | `cargo test --all-features` |
| Format / Test / Clippy (WASM web crate) | `crates/repowise-web` |

The steps are sequential and a failure stops the job, which has a consequence
worth knowing: **a formatting violation means nothing else ran.** A red run
whose only visible failure is Format is not evidence that the tests pass — they
were skipped. This is not hypothetical; commit `894d0a6` left `main` red on two
`cargo fmt` violations for two days, and Clippy, Test and the WASM steps never
executed in that window.

`crates/repowise-web` is deliberately outside the root workspace (it only ever
targets `wasm32-unknown-unknown`), so the workspace-wide steps skip it. It gets
its own three steps for that reason. Its tests run on the host target: the
crate's pure logic — the treemap layout in particular — is testable without a
browser, and the alternative is a frontend with no test coverage at all.

## Making it the gate

Branch protection is what turns a green run from information into a
precondition. Without it, both CONTRIBUTING.md rules are convention: anyone with
write access can push straight to `main`, and a PR can be merged red.

Applying it requires admin on the repository:

```sh
gh api -X PUT repos/baileyrd/rusty_repo_wise/branches/main/protection \
  -H "Accept: application/vnd.github+json" \
  -f 'required_status_checks[strict]=true' \
  -f 'required_status_checks[contexts][]=check' \
  -F 'enforce_admins=true' \
  -F 'required_pull_request_reviews=null' \
  -F 'restrictions=null'
```

What each field buys:

- `contexts[] = check` — the job name above. A misspelling here does not error;
  it creates a rule waiting on a check that will never report, which blocks
  every PR instead of gating them.
- `strict = true` — a PR must be up to date with `main` before merging, so the
  green run reflects the merged state rather than a stale base.
- `enforce_admins = true` — admins included. Omitting this leaves the exact hole
  the rule exists to close, since admins are who would push directly.
- `required_pull_request_reviews = null` — no review requirement at this layer.
  CONTRIBUTING.md asks for an approval; on a single-maintainer repo, encoding it
  here would block every PR. Set it when there is a second maintainer.
- `restrictions = null` — no push allowlist; the PR requirement is doing the
  work.

Protection does **not** dictate merge method. CONTRIBUTING.md requires a merge
commit — never squash or rebase — and that is enforced by repository settings
(Settings → General → Pull Requests: allow merge commits, disable the other
two), not by the API call above.

## Checking whether it is on

```sh
gh api repos/baileyrd/rusty_repo_wise/branches/main --jq '.protected'
```

`false` means everything in this document is aspirational for that branch. As of
this commit it returns `false` — the rule has not been applied, because doing so
needs admin credentials that CI and automation in this repo do not have. Treat
the command, not this paragraph, as the source of truth.
