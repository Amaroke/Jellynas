---
name: check-aurral
description: Check the music of Aurral on the NAS, repair the files a browser cannot play, relaunch the failed, missing and lossy tracks, and clean slskd. Use when asked for the state, a check, a repair or a search of Aurral or of the music, to clean slskd, or when a track shows "Track unavailable".
---

# Check Aurral

Follow [the automation check](../../../docs/agents/automation-check.md) with the
app `aurral`, with one change: the Admin set this skill to fix and relaunch on
its own, so before writing the report run the steps below in order, without
asking, and let the report tell what each did.

Aurral downloads through slskd, which shares the network of Gluetun, so a
download problem often starts in slskd or Gluetun, whose state and logs the
report carries.

## 1. Repair the broken files

`broken` lists the files of the Library a browser cannot play: Aurral shows
them as "Track unavailable. Restore the file or refresh the Library." even
though the file is there. The audio is intact, only the header is wrong:

- **ID3 tag before the FLAC stream**: an MP3 tag written in front of the FLAC.
- **Embedded picture without a type**: a cover whose MIME type is empty.

When a file is `repairable`, run `check.sh repair-aurral`. It fixes those files
in place, audio untouched, and prints the files it repaired and the ones it
left. A file left, such as one that is not FLAC at all, is replaced from
Soulseek with a manual search in Aurral: name it to the Admin.

## 2. Clean slskd

`slskdLeftovers` counts the files slskd left in `downloads` and `incomplete`:
Aurral moves every album it keeps into its Library, so what stays there is a
duplicate, a file Aurral refused or an abandoned transfer. When it counts any,
run `check.sh clean-slskd`. It deletes the files untouched for an hour and the
empty folders, so a transfer in progress stays.

## 3. Relaunch the tracks

Each is one `check.sh api aurral POST <path>` call:

| Path                          | What it does                                 |
| ----------------------------- | -------------------------------------------- |
| `/playlists/research-missing` | Searches the failed and missing tracks again |
| `/playlists/quality-upgrades` | Searches FLAC for the lossy tracks, max 500  |

The reply of `research-missing` counts the tracks it queued. `quality-upgrades`
can outlast the call: when the call times out, the search still runs.

## 4. Read back

Run the check again: `broken` holds only the files left, `slskdLeftovers` is
empty, and `status` shows the queued tracks under `pending`.
