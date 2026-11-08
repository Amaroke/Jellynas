# ratio farms C411 from Twins and keeps a Zen reserve

Superseded by ADR 0011 (the Farm earns Zen first under a Ratio floor).

Supersedes ADR 0005 (ratio torrents stay in the torrents tree).

The Admin seeds on C411, the only private tracker of the Admin, to keep the ratio of its C411 account, with torrents grabbed only to seed. Those torrents must never reach the Library, the long seeding times must not fill the disk, and every paid grab costs ratio on an account that sits at its edge. C411 also pays Zen, worth 1 GB of upload each, for long seeding, with a class bonus for rare torrents. One script of the repo, `ratio/ratio.py`, with the standard library of Python only, runs the Farm and the Zen reserve every 10 minutes.

## Decision

- **The torrents tree only**: the Farm and the Zen reserve live in their own qBittorrent categories under `/data/torrents`. Radarr and Sonarr only import from their own category, so no tool imports them.
- **Imports stay copies**: Radarr and Sonarr copy into the Library. Tdarr rewrites every Library file, which would break a hardlink anyway, and the copy keeps the torrents tree byte for byte as the trackers announced it, which cross-seed needs.
- **cross-seed links within the torrents tree**: cross-seed hardlinks what it matches into `/data/torrents/cross-seed`, so a cross-seeded torrent takes no space and keeps seeding when Radarr or Sonarr remove the torrent it matched.
- **ratio alone removes from the Farm and the Zen reserve**: qbit_manage only tags torrents by tracker. Radarr and Sonarr keep "Remove Completed" for their own torrents, with the seeding goals Prowlarr syncs per indexer. ratio never removes before `RATIO_MIN_SEEDING_DAYS`.
- **Grabbing by demand, not freeleech**: C411 announces on no IRC channel autobrr knows, and almost none of its releases is freeleech, so ratio reads its newest releases in Prowlarr and grabs a fresh one with a hungry swarm, under a daily quota and a pause while the deficit is too high.
- **Twins**: for a fresh C411 episode with a leecher, ratio looks on Nyaa for the same release group within 5 % of its size, then compares the length of every file of both torrents. A Twin downloads from Nyaa for free, outside any quota or pause, and cross-seed seeds its C411 copy. When cross-seed misses a Twin listed under a French title, ratio adds the C411 torrent itself on the Nyaa files after an hour.
- **Nyaa first in Radarr and Sonarr**: Prowlarr gives Nyaa a better priority than C411, so an anime Request downloads from Nyaa at equal quality and score, and cross-seed seeds it on C411.
- **The Zen reserve**: rare C411 torrents kept for Zen whatever they upload, for the Swarm Keeper class, first until it holds 15 torrents, its best bonus. It fills from torrents leaving the Farm, from Twins, from Library torrents of Radarr and Sonarr once imported, and from C411 itself only above a surplus.
- **Freeleech is free**: a C411 release with a `downloadvolumefactor` of 0 is grabbed outside quota and pause, and never counts in the ledger.
- **C411 peers**: C411 counts each seeder twice in the peers it gives Prowlarr, so ratio reads its leechers as the peers minus twice the seeders.
- **One ledger for the tracker**: the deficit counts every torrent whose announce URL holds `RATIO_TRACKER`, Library ones included, and keeps the torrents removed since, so it follows the real ratio of the C411 account.

## Considered Options

- **autobrr on freeleech or on any fresh release**: rejected as the grabber. C411 announces on no IRC channel, and autobrr sees a release before leechers arrive, so it cannot tell a hungry swarm from a dead one. It stays for a tracker that announces on IRC.
- **Hardlinks from the torrents tree into the Library**: rejected. Tdarr replaces the Library file, so the link would break at the first pass.
- **cross-seed injecting without links**: rejected. The cross-seeded torrent would lose its files when Radarr or Sonarr remove the original.
- **qbit_manage removing**: rejected. Two removers in the same categories would race, its share limits would throw away the rare torrents the Zen reserve keeps, and it has no limit on the size of a folder.
- **The Tanker class**: rejected. Its best bonus needs 750 torrents seeding at once.
- **ratio checking and linking the C411 copy of every Twin itself**: rejected. cross-seed already does it, ratio only steps in when cross-seed misses the title.
- **Hunting rare torrents on C411 whatever the ratio**: rejected while the C411 account sits at the edge of its ratio.

## Consequences

The first day grabbed 12 GB and uploaded 0.6 GB: releases a few hours old were already full of seedboxes, so a Farm grab is at most 1 hour old and 5 GB. A downloaded Library file and an adopted Library torrent take their space twice. The ledger counts the Requests that Radarr and Sonarr download from C411, so a large Request can pause paid grabs. A Twin needs the same files on both trackers, which mostly happens for anime episodes of French groups that post on both. Twins depend on C411 accepting cross-seeds from a public tracker, and the cross-seed hook on Gluetun letting qBittorrent reach cross-seed. The hardlinks of cross-seed need the torrents tree on a single file system. To tell which Source earns its disk, ratio keeps the Source of each C411 torrent and reports its figures per Source once a day.
