---
name: merge-all-prs
description: Push every open pull request of the repo through to a squash merge, one at a time.
disable-model-invocation: true
---

# Merge all PRs

Drive every open, non-draft pull request of `Amaroke/Jellynas` to a squash
merge. The `main` ruleset gates each merge: squash only, linear history, branch
up to date with `main`, every required check green, every review thread
resolved. Satisfy the ruleset, never bypass it: merge with `--squash` alone,
never `--admin`, and push only to pull request branches.

Each merge makes every other branch stale, so work the pull requests
sequentially, oldest first, and finish one before touching the next.

## 1. List

```sh
gh pr list --state open --json number,title,headRefName,isDraft,author \
  --jq 'sort_by(.number)[] | select(.isDraft | not)'
```

Drafts stay open with the reason `draft`.

## 2. Drive each pull request

Loop on one pull request until it is merged or left open with a reason. Post a
one-line progress update to the user as each pull request starts, at each
stage below, and as it ends merged or left open.

**Update.** Bring the branch up to date with `main`:

```sh
gh pr update-branch <n>
```

On a conflict, rebase in a throwaway worktree, resolve with the
`resolving-merge-conflicts` skill, push, and remove the worktree:

```sh
git fetch origin main <branch>
git worktree add ../pr-<n> origin/<branch>
git -C ../pr-<n> rebase origin/main
git -C ../pr-<n> push --force-with-lease origin HEAD:<branch>
git worktree remove ../pr-<n>
```

**Checks.** Wait for the required checks:

```sh
gh pr checks <n> --required --watch --fail-fast
```

Renovate's `minimumReleaseAge` shows as a pending `renovate/stability-days`
status. It is not a required check: merge a Renovate pull request without
waiting for it.

On a red check, read the failing job with `gh run view <run-id> --log-failed`,
push a fix to the branch from a worktree as above, and wait again. After three
red rounds, leave the pull request open with the failing check as the reason.

**Threads.** List the unresolved review threads:

```sh
gh api graphql -F owner=Amaroke -F repo=Jellynas -F n=<n> -f query='
query($owner: String!, $repo: String!, $n: Int!) {
  repository(owner: $owner, name: $repo) {
    pullRequest(number: $n) {
      reviewThreads(first: 100) {
        nodes { id isResolved path comments(first: 20) { nodes { body } } }
      }
    }
  }
}' --jq '.data.repository.pullRequest.reviewThreads.nodes[]
  | select(.isResolved | not)'
```

Settle each one: push the fix it asks for, or reply on it when no change is
due. Then resolve it:

```sh
gh api graphql -F id=<thread-id> -f query='
mutation($id: ID!) { resolveReviewThread(input: {threadId: $id}) {
  thread { isResolved } } }'
```

A pushed fix reruns the checks: go back to **Checks**.

**Merge.** Once the branch is up to date, the required checks are green and no
thread is open:

```sh
gh pr merge <n> --squash
```

A merge refused because the branch went stale sends the pull request back to
**Update**.

## 3. Report

The run is done when every pull request from step 1 is merged or left open
with a reason. Show one table:

| PR | Title | Result |
| -- | ----- | ------ |
| #n | title | merged, or left open: reason |
