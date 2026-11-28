# Issue tracker: GitHub

Issues and specs for this repo live as GitHub issues. Use the `gh` CLI for all
operations, from inside the clone.

## Conventions

- **Create an issue**: `gh issue create --title "..." --body "..."`. Use a
  heredoc for multi-line bodies.
- **Read an issue**: `gh issue view <number> --comments`.
- **List issues**: `gh issue list --state open --json
  number,title,body,labels,comments` with `--label` and `--state` filters.
- **Comment on an issue**: `gh issue comment <number> --body "..."`
- **Apply / remove labels**: `gh issue edit <number> --add-label "..."` /
  `--remove-label "..."`
- **Close**: `gh issue close <number> --comment "..."`

Pull requests are not a request surface: only issues are triaged.

## Origin of an issue

Every issue says who asked for it, twice:

- its body opens with the line `Asked by the Admin.` or `Asked by <name>.`,
  where `<name>` is the Discord name of the Friend, exactly as Discord shows it,
- it carries the label `from:admin` or `from:friend`.

An issue an agent proposes and the Admin keeps opens with `Proposed by <issue>,
kept by the Admin.` and carries `from:admin`. The `discord-posts` service lists
the open issues with one of these labels in the issues channel of Discord, and
tags each Friend from the table `${APPDATA_ROOT}/discord-posts/friends.toml` on
the NAS, as the Issues on Discord section of the README sets out. An issue
without either label stays off Discord.

## When a skill says "publish to the issue tracker"

Create a GitHub issue.

## When a skill says "fetch the relevant ticket"

Run `gh issue view <number> --comments`.
