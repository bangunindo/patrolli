# Contributing to Patrolli

Thank you for your interest in Patrolli.

## Pull requests are not accepted yet

Patrolli was just opened up, and we are still settling its design and the
way we work on it in public. Until then, pull requests are open only to the
core team. We will update this file when that changes.

## What you can do now

- **Report a bug or request a feature** by opening an issue. Include the
  Patrolli version, how you run it (single node or cluster), the relevant part
  of your configuration with secrets removed, and the logs.
- **Report a security vulnerability** privately, as described in
  [SECURITY.md](SECURITY.md). Don't open a public issue for it.
- **Fork the project.** Patrolli is licensed under the
  [GNU Affero General Public License, version 3](LICENSE), so you are free to
  study, change, and redistribute it under its terms.

## How we work

These are the rules for working on Patrolli. Until pull requests are open to
everyone, only the core team follows them. After that, they apply to every
contributor.

### Write in English

Use English for issues, pull request titles and descriptions, review
comments, code, and code comments.

### Create and name branches

- Create every branch from `main`.
- Name each branch `<name>/<topic>`, for example `aan/opl-validation`. Keep
  the topic short and lowercase, with hyphens between words.
- To get new changes from `main`, rebase your branch or merge `main` into it.
  Both are fine.
- GitHub deletes the branch automatically after the merge.

### Write commit messages

Commit messages have no required format. You don't need prefixes such as
`feat:` or `fix:`. Keep each message short and say what the commit does.

### Open a pull request

#### Write the title

Write a plain title that says what the pull request does:

- Start with a capitalized verb in the imperative, such as `Add`, `Fix`, or
  `Remove`.
- Do not end the title with a period.
- Keep the title to about 72 characters.
- Do not add area tags such as `[FE]` or `[BE]`.

Example: `Add saved changes warning on pipeline canvas processes`

#### Add a label

Add one of these labels to every pull request:

- `enhancement`
- `bug`
- `documentation`
- `ci`
- `misc`, for a change that fits none of the other labels, such as a license
  change

#### Write the description

- Describe what the pull request changes. This part is required.
- You can also explain why the change is needed.
- If you change the UI and a reviewer needs to see the result, attach a
  screenshot or a video. A UI bug fix needs one. A typo fix or a rename does
  not.
- If an issue exists, link it with `Closes #12`.

#### Keep each pull request small

- Make one change per pull request. Do not mix a feature with an unrelated
  refactor or fix.
- Split large work into a chain of smaller pull requests with
  [GitHub stacked pull requests](https://docs.github.com/en/pull-requests/how-tos/stacked-pull-requests).

#### Open it as a draft until it is ready

If the pull request is not ready for review, open it as a draft. When it is
ready, click **Ready for review**.

### Check the license of new dependencies

Patrolli is licensed under `AGPL-3.0-only`. A new dependency must have a
license that is compatible with `AGPL-3.0-only`.

- CI checks the licenses of Rust crates in `src/core` against the `allow`
  list in `src/core/deny.toml`.
- CI does not check the licenses of web dependencies in `src/web`. Before you
  add one, check its license against the `allow` list in
  `src/core/deny.toml`.

### Get a review

- The author picks the reviewer.
- The pull request needs one approval.
- There is no target response time.
- The reviewer, not the author, resolves review comments.
- The reviewer decides which comments must be resolved before the merge.
- Avoid pushing to a branch after it is approved. If you must push, for
  example to add a small fix you missed, the reviewer must review the new
  changes before the merge.

### Merge

The reviewer who approved the pull request merges it.
