---
name: publish-discord
description: Publish the Discord messages of discord-posts/ on the NAS now, the Patch notes included, through the API of the discord-posts service. Use when asked to publish, post or refresh a Discord message, a tuto, the links or a Patch note.
---

# Publish Discord

The `discord-posts` service publishes the files of `discord-posts/` every night.
This skill publishes them now. Reaching the NAS is in
[nas-access.md](../../../docs/agents/nas-access.md).

## 1. Check that main holds the message

The service publishes the files of `main` as doco-cd deployed them. A message
changed on a branch is not on the NAS until its pull request is merged. If the
Admin asks for a message that `origin/main` does not hold yet, say so and stop.

## 2. List the messages

```sh
ssh jellynas "sudo sh /mnt/tank/jellynas/scripts/check.sh \
  api discord-posts GET /api/posts"
```

Each entry gives the `channel`, the `file`, the `message` id once posted (null
before), and for a Patch note the last pull request it `covers`.

## 3. Agree on what to publish

Show the Admin, in a short table, the messages that will be posted for the
first time and the ones that will be edited. Ask with buttons whether to
publish all of them or only the one they named. Publishing writes to a channel
the Friends read, so never publish before the Admin agrees.

## 4. Publish

All the messages:

```sh
ssh jellynas "sudo sh /mnt/tank/jellynas/scripts/check.sh \
  api discord-posts POST /api/publish"
```

A single one, by its `file`:

```sh
ssh jellynas "sudo sh /mnt/tank/jellynas/scripts/check.sh \
  api discord-posts POST '/api/publish?file=patch-notes/2026-11-27.md'"
```

## 5. Report

The reply gives each message an `outcome`: `posted`, `edited`, `skipped` (a
Patch note without its webhook in `.env`) or `failed` with its `error`. Show
the outcomes as a table. For a failure, name the cause: a webhook missing from
`.env` is for the Admin to set, a placeholder missing from `.env` or a message
too long is a fix in the repo. A Patch note refused for its order stays
unposted: say which note blocks it, and never work around the order, since
Discord would show the notes out of order for good.
