---
name: publish-discord
description: Publish the Discord messages of discord-posts/ on the NAS now, the Patch notes included, through the API of the discord-posts service. Use when asked to publish, post, refresh or sync a Discord message, a tuto, the links or a Patch note.
---

# Publish Discord

The `discord-posts` service keeps each channel of the Discord server of the
Friends in sync with its folder of `discord-posts/` every night. This skill
syncs now. Reaching the NAS is in
[nas-access.md](../../../docs/agents/nas-access.md), how the sync works in the
Discord messages section of the README.

## 1. Check that main holds the message

The service publishes the files of `main` as doco-cd deployed them. A message
changed on a branch is not on the NAS until its pull request is merged and
deployed. If the Admin asks for a message that `origin/main` does not hold yet,
say so and stop.

## 2. Read the plan

```sh
ssh jellynas "sudo sh /mnt/tank/jellynas/scripts/check.sh \
  api discord-posts GET /api/posts"
```

If the call cannot reach the service, it was stopped from the Dashboard: say
so and stop, the switch is the Admin's.

Each channel lists its files with their `action` on the next run, `post`,
`edit` or `repost`, the messages it would `delete`, whether it `needs_force`,
and the `problems` that block it.

## 3. Agree on what to publish

Show the Admin a short table per channel of what will be posted, reposted and
deleted. Leave out the plain edits, they change nothing visible but the footer.
Say that a repost or a deletion loses the reactions and the links to the
message. Name each problem and its fix in the repo: a channel with a problem is
left alone.

Ask with buttons whether to publish every channel or only one. When a channel
`needs_force`, ask for it separately, naming each message it deletes. Never
publish before the Admin agrees: the channels are read by the Friends.

## 4. Publish

Every channel, or one with `?channel=<name>`, and with `force=true` only when
the Admin agreed to it:

```sh
ssh jellynas "sudo sh /mnt/tank/jellynas/scripts/check.sh \
  api discord-posts POST /api/publish"
ssh jellynas "sudo sh /mnt/tank/jellynas/scripts/check.sh \
  api discord-posts POST '/api/publish?channel=tutos&force=true'"
```

## 5. Report

The reply gives each message an `outcome`: `posted`, `edited`, `reposted`,
`deleted`, `skipped` (a channel without its webhook in `.env`) or `failed` with
its `error`. Show what changed as a table. For a failure, name the cause: a
webhook missing from `.env` is for the Admin to set, anything else is a fix in
the repo, which the `write-discord` skill can make.
