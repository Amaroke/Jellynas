---
name: feedback-to-issues
description: Turn messages pasted from Discord into issues, one per request, each saying who asked for it. Use when the Admin pastes feedback, requests or bug reports from the Friends or from Discord, or asks to file them as issues.
---

# Feedback to issues

The Admin pastes messages from Discord. This skill files one issue per new
request, with the origin that the Origin of an issue section of
`docs/agents/issue-tracker.md` sets: the `Asked by` line and the `from:admin` or
`from:friend` label. That origin is what `discord-posts` shows and tags in the
issues channel.

## 1. Read the table of people

```sh
ssh jellynas 'sudo cat /mnt/tank/appdata/discord-posts/friends.toml'
```

Its `[admin]` section names the Admin on Discord, its `[friends]` section each
Friend, with their Discord id. Done when you hold both sections, or know the
file is missing.

## 2. Split the paste into requests

Discord prints the name of the author above their messages. Group the messages
by author, then cut them into requests: one thing to add, change or fix each.
A name in `[admin]` is the Admin. Any other name is a Friend, written exactly
as Discord shows it, case and accents included. Done when every message
belongs to a request or is plain chat with nothing to do.

## 3. Check each request against the repo

For each request, search the code, the README and the issues, open and closed:

```sh
gh issue list --state all --limit 200 --search "<words of the request>" \
  --json number,title,state,stateReason
```

Sort it into one of three cases:

- **done**: the code or a merged pull request already does it. Note where.
- **open**: an open issue already covers it. Note its number.
- **new**: nothing covers it.

Done when every request has its case and its evidence.

## 4. Ask the Admin, one request at a time

Show the request in one line, its author, its case and the evidence, then
offer buttons:

- **new**: "File it" or "Skip", with the title and body you would file.
- **open**: "Comment on" the issue, to add the words of the Friend to that issue
  with the same `Asked by` line, or "Skip".
- **done**: "Skip", or "File it anyway" when the Admin sees a gap.

## 5. File what the Admin kept

Write in English, in the house style of the open issues: a title that states
the outcome, such as "A Friend sees their watch history", and a body that opens
with the `Asked by` line, then says what is asked and what to check.

```sh
gh issue create --title "<title>" \
  --label needs-triage --label from:friend --label enhancement \
  --body "Asked by Niixe. <what is asked and what to check>"
```

Use `from:admin` and `Asked by the Admin.` for a request of the Admin, and
`bug` instead of `enhancement` for something that broke. Done when every
request the Admin kept has its issue or its comment.

## 6. Add a new Friend to the table

A Friend whose name is missing from `[friends]` shows on Discord as plain text,
without a tag. For each one, ask the Admin for their Discord id: in Discord,
with the developer mode on, right click the Friend, then "Copy User ID". Show
the exact line, then add it after the Admin presses "Add it":

```sh
echo '<name> = "<id>"' \
  | ssh jellynas 'sudo tee -a /mnt/tank/appdata/discord-posts/friends.toml'
```

The line lands at the end of the file, so `[friends]` stays the last section.
When the file is missing, write it whole, `[admin]` then `[friends]`, after the
same button. Done when every Friend of the issues filed has their line, or the
Admin chose to leave them untagged.

## 7. Report

List the issues filed and the comments added, each with its link. The issues
channel shows them and tags the Friends within the hour.
