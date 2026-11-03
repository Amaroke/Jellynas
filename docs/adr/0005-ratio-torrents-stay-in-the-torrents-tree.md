# Ratio torrents stay in the torrents tree

The Admin seeds on private trackers to keep a good ratio, with torrents grabbed only to seed. Those torrents must never reach the Library, and the long seeding times of private trackers must not fill the disk.

## Decision

- **Ratio folder**: ratio torrents go to qBittorrent in the `ratio` category, saved in `/data/torrents/ratio`. Radarr and Sonarr only import from their own category, so no tool imports a ratio torrent.
- **Grabbing by demand, not freeleech**: ratio-grab, a script of this repo, reads the newest movies and shows of the private indexers in Prowlarr and grabs a release at most 6 hours old, between 1 and 15 GB, with at least 10 leechers and 5 leechers per seeder, at most 2 downloading and 3 a day, and pauses while the ratio torrents downloaded more than `RATIO_GRAB_MAX_DEFICIT_GB` beyond what they uploaded. autobrr stays available for freeleech filters on trackers that announce on IRC.
- **Imports stay copies**: Radarr and Sonarr keep copying into the Library, as before. Tdarr rewrites every Library file, which would break a hardlink anyway, and the copy keeps the torrents tree byte for byte as the trackers announced it, which cross-seed needs to match it.
- **Cross-seed links within the torrents tree**: cross-seed hardlinks the data it matches into `/data/torrents/cross-seed`, in the `cross-seed-link` category. A cross-seeded torrent takes no extra space, and keeps seeding when Radarr or Sonarr remove the torrent it matched.
- **qbit_manage manages only the ratio and cross-seed categories**: it tags every torrent by tracker, but applies share limits and removes torrents only in `ratio` and `cross-seed-link`. It never stops them at a ratio or a seeding time, and removes one once it has uploaded nothing for 10 days, after the minimum seeding time of any tracker that asks for 10 days or less, C411 asks for 2. Radarr and Sonarr keep "Remove Completed" for their own torrents, with the seeding goals that Prowlarr syncs per indexer.
- **Size limit**: ratio-limit, a script of this repo, removes the ratio torrents that upload the least once the ratio folder holds more than `RATIO_MAX_SIZE_GB`, among those that seeded at least `RATIO_MIN_SEEDING_DAYS`. It counts every ratio torrent at its full size, still downloading or not, so a new grab makes room at once.

## Considered Options

- **autobrr with a freeleech filter only**: rejected as the main grabber. C411 announces on no IRC channel autobrr knows, and none of its last hundred movies and shows was freeleech, so the filter would grab nothing.
- **autobrr on any fresh release**: rejected. It sees a release the second it is published, before leechers arrive, so it cannot tell a hungry swarm from a dead one.
- **Hardlinks from the torrents tree into the Library**: rejected. Tdarr replaces the Library file, so the link would break at the first pass and the file would take twice its space anyway, only later.
- **qbit_manage replacing "Remove Completed"**: rejected. Radarr and Sonarr already apply the seeding goal of each indexer and remove their torrents at the right time, a second remover would race them.
- **qbit_manage for the size limit**: rejected, it has no limit on the size of a folder. Its standalone script watches the free space of a whole drive, which the Library shares.
- **cross-seed injecting without links**: rejected. The cross-seeded torrent would share the files of the original one, and lose them when Radarr or Sonarr remove it.

## Consequences

Every ratio-grab download counts against the ratio until it uploads more than it took, which a hungry swarm usually pays back, so the limits stay low until the Account has a buffer. A file downloaded by Radarr or Sonarr still takes twice its space while it seeds. The ratio folder can go over its limit while every torrent in it is still downloading or below `RATIO_MIN_SEEDING_DAYS`, ratio-limit says so in its logs. A torrent cross-seeded from a ratio torrent keeps its data after ratio-limit removes the original, until qbit_manage removes it at its own goal. The `cross-seed` folder holds hardlinks, so the torrents tree must stay on a single disk.
