---
name: write-discord
description: Write the next Patch note for the Friends from the pull requests merged since the last one, or change any other Discord message of discord-posts/, then open a pull request for the Admin to review. Use when asked for a Patch note, to tell the Friends what changed, or to add, change, move or remove a Discord message or tuto.
---

# Write Discord

The Discord messages of the Friends are the files of `discord-posts/`, one
folder per channel, as the Discord messages section of the README sets out.
This skill changes those files through a pull request. Publishing is the
`publish-discord` skill, once the pull request is merged and deployed.

## 1. Start from main

Follow the Git section of `CLAUDE.md`: fetch, fast-forward `main`, and cut a
branch named after the change, such as `patch-note-2026-11-27`. Done when the
branch holds the latest `main`.

## 2a. A Patch note

1. Read the `covers: #<number>` header of the newest file of
   `discord-posts/patch/`. With no note yet, start from the first pull request
   of the repo.
2. List the pull requests merged on `main` after that one, by merge time, not
   by number:

   ```sh
   gh pr list --state merged --base main --limit 200 \
     --json number,title,body,author,mergedAt
   ```

   Keep the ones merged after the `mergedAt` of the covered pull request.
3. Leave out Renovate and Dependabot, and anything a Friend does not notice:
   CI, tests, docs for the Admin, internal tooling, services only the Admin
   uses. Keep new or removed services, new features, fixes of what the Friends
   use, and changes of addresses, apps or rules. Read the body of a pull
   request when its title does not say.
4. With nothing left, tell the Admin nothing changed for the Friends, and stop
   without writing a note.
5. Write `discord-posts/patch/<today>.md`: the header with the last merged pull
   request read in step 2, even one left out, then one list item per change, in
   French, in the tone of the other messages, for someone who does not know the
   NAS, with an emoji at the front like the other messages. No `#` heading: the
   title comes from the date.

## 2b. Another message

Edit the file of the message, or add one in the folder of its channel. The
number in front of the name sets its place. Before writing, tell the Admin what
the place costs on Discord:

- a new file at the end of its channel is only posted,
- a new file before others, or a renumbering that moves a file, deletes and
  posts again every message after it, which loses their reactions and links,
- removing a file deletes its message.

Ask with buttons when the place is the Admin's choice.

## 3. Check

Run `cargo test -p discord-posts`: it fails on a message too long for Discord,
a name without its number or date, a placeholder other than `${DOMAIN}` and
`${CADDY_HTTPS_PORT}`, or a Patch note that breaks its rules. Fix until it passes.

## 4. Review with the Admin

Show the Admin the message as Discord will show it: the title, then the text.
Apply their changes and show it again, until they approve it.

## 5. Open the pull request

Commit, push, and open a pull request to `main`, then turn on auto-fix, as the
Finishing a change section of `CLAUDE.md` sets out. A Patch note lists in the
body the pull requests it covers and the ones it leaves out. Then offer the
Admin, with buttons, to review the pull request now: on yes, walk them through
its diff. Never merge it: merging deploys.

## 6. Publish once deployed

When the Admin says the pull request is merged, or its merge reaches you, wait
for doco-cd to deploy it: `origin/main` holds the commit, then the CI builds
`discord-posts` in a few minutes and doco-cd pulls it within a minute. Then
offer, with buttons, to publish with the `publish-discord` skill.
