# Ratio torrents stay in the torrents tree

The Admin seeds on private trackers to keep a good ratio, with torrents grabbed only to seed. Those torrents must never reach the Library, and the long seeding times of private trackers must not fill the disk.

## Decision

- **Ratio folder**: ratio torrents go to qBittorrent in the `ratio` category, saved in `/data/torrents/ratio`. Radarr and Sonarr only import from their own category, so no tool imports a ratio torrent.
- **Grabbing by demand, not freeleech**: ratio, a script of this repo, reads the newest movies and shows of the private indexers in Prowlarr and grabs a release at most 1 hour old, between 1 and 5 GB, with at least 10 leechers and 5 leechers per seeder, at most 2 downloading and 3 a day, and pauses while the ratio torrents downloaded more than `RATIO_GRAB_MAX_DEFICIT_GB` beyond what they uploaded, counting the torrents removed since. autobrr stays available for freeleech filters on trackers that announce on IRC. [ADR 0009](0009-ratio-farms-from-twins-and-keeps-a-zen-reserve.md) adds Twins and the Zen reserve.
- **Imports stay copies**: Radarr and Sonarr keep copying into the Library, as before. Tdarr rewrites every Library file, which would break a hardlink anyway, and the copy keeps the torrents tree byte for byte as the trackers announced it, which cross-seed needs to match it.
- **Cross-seed links within the torrents tree**: cross-seed hardlinks the data it matches into `/data/torrents/cross-seed`, in the `cross-seed-link` category. A cross-seeded torrent takes no extra space, and keeps seeding when Radarr or Sonarr remove the torrent it matched.
- **qbit_manage tags, ratio removes**: qbit_manage tags every torrent by tracker. Since [ADR 0009](0009-ratio-farms-from-twins-and-keeps-a-zen-reserve.md), it removes nothing, and ratio removes the torrents of the `ratio`, `cross-seed-link` and `zen` categories. Radarr and Sonarr keep "Remove Completed" for their own torrents, with the seeding goals that Prowlarr syncs per indexer.
- **Size limit**: ratio removes the torrents of the `ratio` and `cross-seed-link` categories that upload the least once they hold more than `RATIO_MAX_SIZE_GB`, among those that seeded at least `RATIO_MIN_SEEDING_DAYS`. It counts every torrent at its full size, still downloading or not, so a new grab makes room at once.

## Considered Options

- **autobrr with a freeleech filter only**: rejected as the main grabber. C411 announces on no IRC channel autobrr knows, and none of its last hundred movies and shows was freeleech, so the filter would grab nothing.
- **autobrr on any fresh release**: rejected. It sees a release the second it is published, before leechers arrive, so it cannot tell a hungry swarm from a dead one.
- **Hardlinks from the torrents tree into the Library**: rejected. Tdarr replaces the Library file, so the link would break at the first pass and the file would take twice its space anyway, only later.
- **qbit_manage replacing "Remove Completed"**: rejected. Radarr and Sonarr already apply the seeding goal of each indexer and remove their torrents at the right time, a second remover would race them.
- **qbit_manage for the size limit**: rejected, it has no limit on the size of a folder. Its standalone script watches the free space of a whole drive, which the Library shares.
- **cross-seed injecting without links**: rejected. The cross-seeded torrent would share the files of the original one, and lose them when Radarr or Sonarr remove it.

## Consequences

Every grab paid to the tracker counts against the ratio until it uploads more than it took, so the limits stay low until the Account has a buffer. A file downloaded by Radarr or Sonarr still takes twice its space while it seeds. The ratio folder can go over its limit while every torrent in it is still downloading or below `RATIO_MIN_SEEDING_DAYS`, ratio says so in its logs. A torrent cross-seeded from a ratio torrent keeps its data after ratio removes the original, until ratio removes it in turn. The `cross-seed` folder holds hardlinks, so the torrents tree must stay on a single disk.

The first day on C411 grabbed 12 GB and uploaded 0.6 GB: releases up to 6 hours old were already full of seedboxes by the time they finished, so the age and size limits went down to 1 hour and 5 GB, and the deficit limit to 5 GB. The deficit first counted only the torrents still in qBittorrent, which forgot the loss of every removed torrent, so ratio-grab now keeps it in a file.
