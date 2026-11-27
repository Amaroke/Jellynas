# Jellynas

## Language

Everything in this repo is written in English: code, comments, docs, CONTEXT.md,
commit messages, branch names, issues, pull requests, labels and workflow names.
This applies even when the conversation with the agent happens in another
language.

## Asking the Admin

When you need a choice or an approval from the Admin, offer buttons
(AskUserQuestion) rather than a question in text.

## Rust first

Write new code in Rust: services, scripts, CLIs and tests alike. Reach for
Python or shell only when Rust cannot do the job, and say why in the pull
request. When you change an existing Python or shell script substantially,
port it to Rust instead. Until the repo tests are ported (#476), new tests of
existing Python code go in the `unittest` suite under `tests/`.

## Finishing a change

Once a change is committed, push its branch, open a pull request that closes its
issue, and end your reply with the link to the pull request. Never work on
`main`: if the current branch is `main`, create a branch named after the issue
first.

Merging into `main` is deploying to production: doco-cd deploys `main` on the
NAS within a minute. Never merge a pull request unless a skill tells you to,
such as `merge-all-prs`, or the Admin asks for it explicitly.

## NAS access

To check or fix the running NAS, go through `scripts/check.sh` on the Clone as
`docs/agents/nas-access.md` describes. API keys stay on the NAS.

## Agent skills

### Issue tracker

Issues and specs live in the GitHub Issues of Amaroke/Jellynas. See
`docs/agents/issue-tracker.md`.

### Triage labels

The five default labels (needs-triage, needs-info, ready-for-agent,
ready-for-human, wontfix). See `docs/agents/triage-labels.md`.

### Domain docs

Single-context: CONTEXT.md at the repo root. See `docs/agents/domain.md`.
