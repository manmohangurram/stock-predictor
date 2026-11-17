# GitHub stacked PRs — reference

Source: <https://docs.github.com/en/pull-requests/get-started/stacked-prs-quickstart>
(read 2026-11-17). This is the workflow this repo follows for a feature that needs more than one PR.

## What a stack is

Each PR targets the branch of the PR below it; the bottom targets `main`. Reviewers see only that
layer's diff, and each layer merges independently, bottom-up.

## Setup

- `gh` ≥ 2.90 and Git ≥ 2.20, authenticated with `gh auth login`.
- Install the extension: `gh extension install github/gh-stack`
- Optional for agents: `gh skill install github/gh-stack`

## Create a stack

```sh
gh stack init                  # creates the tracking entry and the first branch
gh stack init a b c            # adopt existing branches (bottom to top)
# work, git add, git commit
gh stack add BRANCH-NAME       # next layer on top
# work, git add, git commit
gh stack add -Am "message"     # stage + commit + next branch in one step
gh stack push                  # push every branch
gh stack submit                # open the PRs, bases chained automatically
gh stack link a b c            # existing branches/PRs -> a stack on GitHub
gh stack checkout <stack#>     # import a remote stack for local tracking
gh stack view                  # branches, PR links, statuses, latest commits
```

`gh stack add -Am` commits on the current branch when it has no commits yet; when it already has
commits it creates the next branch.

## Why it matters here

- A large feature is reviewable layer by layer, and a red layer blocks the ones above it.
- Every PR is small enough to read in one sitting; the stack keeps the order visible on GitHub.
- Refactors, behaviour changes and dependency bumps still go in separate layers.

## Links

- Managing: <https://docs.github.com/en/pull-requests/how-tos/create-pull-requests/managing-stacked-pull-requests>
- Merging: <https://docs.github.com/en/pull-requests/how-tos/merge-and-close-pull-requests/merging-stacked-pull-requests>
