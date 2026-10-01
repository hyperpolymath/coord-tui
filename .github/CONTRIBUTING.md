<!--
SPDX-License-Identifier: CC-BY-SA-4.0
Copyright (c) Jonathan D.A. Jewell <j.d.a.jewell@open.ac.uk>
SPDX-FileCopyrightText: 2026 Jonathan D.A. Jewell (hyperpolymath) <6759885+hyperpolymath@users.noreply.github.com>
-->

# Contributing — coord-tui

## Audience

Developers working **on** `coord-tui`. For consumers (people calling or
depending on it) see <a href="./usage.adoc" class="adoc">usage</a>.

## Local-dev setup

Prerequisites — the minimum versions and where to get them:

- `<tool` `1>` v\`\<version\>\` — `<install` `instruction>`.

- `<tool` `2>` v\`\<version\>\` — `<install` `instruction>`.

- SSH signing key configured (estate policy — all commits must be
  signed). See
  [standards/docs/secure-coding-training.md](https://github.com/hyperpolymath/standards/blob/main/docs/secure-coding-training.md).

One-shot setup:

``` bash
git clone git@github.com:hyperpolymath/coord-tui.git
cd coord-tui
just setup       # installs deps, sets up hooks
just test        # runs the full test suite
```

## Running tests

- **Unit**: `just` `test-unit` — fast, no I/O.

- **Integration**: `just` `test-int` — uses real services (database,
  HTTP, etc.). Estate policy: prefer real over mocked (see
  `feedback_integration_tests_real_db` in maintainer’s memory).

- **Property**: `just` `test-prop` — randomised, slower; budget
  documented in `docs/proof-debt.md` if applicable.

- **Full**: `just` `test` — runs all of the above.

## Code style

We enforce style via CI (governance-reusable.yml from
hyperpolymath/standards). Locally:

``` bash
just fmt         # auto-format
just lint        # static checks
```

- All commits must be **signed**, with SSH for people (CI enforces; see
  [standards](https://github.com/hyperpolymath/standards)).

- All source files must carry an **SPDX-License-Identifier** header (CI
  enforces).

- Conventional commits — `feat`, `fix`, `chore`, `refactor`, `docs`,
  `test`, `ci`, `revert` (CHANGELOG is auto-generated from these via
  [`changelog-reusable.yml`](https://github.com/hyperpolymath/standards/blob/main/.github/workflows/changelog-reusable.yml)).

## Branching & PR workflow

1.  Branch off `main` as `claude/<topic>` (for AI agents) or
    `<initials>/<topic>` (for humans).

2.  Make focused, narrow commits — one logical change per commit.

3.  Open a PR against `main`.

4.  **Enable auto-merge immediately** on every PR you open (`gh` `pr`
    `merge` `<num>` `--auto` `--squash`) — estate standing policy (see
    standards#196 audit and policies).

5.  CI must be green. The PR auto-merges when checks pass + reviews
    land.

## Adding a new dependency

1.  State the **why** in the PR body — what does this dependency unlock?

2.  Check provenance (maintained, audited, no malicious history).

3.  Pin to a SHA, not a tag.

4.  Update `docs/architecture.adoc#Dependencies`.

## Adding an ADR

When you make a non-obvious design decision, write it down:

1.  Copy `docs/decisions/0001-template.adoc` → `0002-<slug>.adoc`.

2.  Fill in: Context, Decision, Consequences, Alternatives.

3.  Link the ADR from the README or relevant code as a comment.

## Reporting issues

- Bugs in `coord-tui`: file at `hyperpolymath/coord-tui/issues`.

- Estate-wide concerns (policy, conventions, CI): file at
  `hyperpolymath/standards/issues`.

## Signed commits

Every commit that reaches the default branch must be signed; a ruleset refuses
unsigned pushes. Estate policy:
[SIGNING-POLICY](https://github.com/hyperpolymath/standards/blob/main/docs/SIGNING-POLICY.adoc).

- **People and interactive agents** sign with an SSH key registered on GitHub
  as a *signing* key (`gpg.format=ssh`, `user.signingkey=<key>.pub`,
  `commit.gpgsign=true`). The committer email must be verified on that account.
- **Apps, bots and workflows** never `git push` local commits. They write
  through the API (`createCommitOnBranch` or the estate `signed-push` action)
  so that GitHub signs each commit.
- Merge PRs with **squash**. The ruleset checks every commit on the PR branch,
  not just the result, so one unsigned commit blocks the merge. Re-create such a
  branch with signed commits (`git cherry-pick -S`) and open a new PR.
  Rebase-merge replays commits unsigned and is disabled.
