# the Farm earns Zen first under a Ratio floor

Supersedes ADR 0009 (ratio farms C411 from Twins and keeps a Zen reserve).

ADR 0009 farmed upload on C411 with paid grabs of fresh releases. Over its first weeks, paid grabs downloaded 18.2 GB and uploaded 3.9 GB: a home connection behind a VPN loses the race against seedboxes on fresh releases. Meanwhile C411 pays Zen, worth 1 GB of upload credit each, every night for every C411 torrent seeded, whatever it uploads, and pays its largest rewards as one-off Quests on the Seed count: how many torrents are seeded at once, whatever their size. The Account ratio sits at 2.09, so the account is not at the edge ADR 0009 assumed.

## Decision

- **Zen before upload**: the NAS works first for Zen, nightly and from Quests, then for the Library, and takes upload as a bonus. Never a Hit and run stays above everything.
- **A Ratio floor of 1.2**: under an Account ratio of 1.2, read from the profile API of C411, the NAS spends no download on C411, not even for the Library: ratio disables C411 in Prowlarr until the ratio is back above the floor. Freeleech and Twins stay allowed, since they cost no download.
- **No paid grabs**: grabbing fresh C411 releases for upload is dropped.
- **The Volume hunt**: ratio grabs small Rare C411 torrents, up to 1 GB, cheapest first, to raise the Seed count, with at least 25 torrents in each of five C411 categories (Films & Vidéos, Ebook, Audio, Emulation, Imprimante 3D) for the diversity Quests. Films & Vidéos concentrates on anime, where Twins from Nyaa cost no download and have no size cap.
- **One Farm**: the Farm and the Zen reserve merge into one Farm in one qBittorrent category, under one disk budget of 100 GB. Nothing leaves it while the budget has room. Once full, the torrents that pay the least Zen for their size leave first, never before their Seeding goal.
- **Five categories**: movies, shows, music and books belong to the Automation stack, farm to ratio. Tags tell the tracker kind and the Source.
- **Public torrents**: a torrent of a public tracker seeds two weeks whatever its ratio, and leaves sooner once its C411 copy seeds the same files. Public trackers are added by hand in Prowlarr and recognised by its privacy field.
- **Swarm Keeper until November 2027**: up to 300 torrents its Quests pay the most. At the class window of November 2027, if the Seed count passed 500, the Admin switches to Tanker, whose Quests already filled are validated at once.

## Considered Options

- **Keeping paid grabs with stricter rules**: rejected, the figures of ADR 0009 show they lose upload whatever the filter.
- **Tanker now**: rejected, its volume Quests pay less than Swarm Keeper below 500 torrents.
- **A Farm and a Zen reserve with separate budgets**: rejected, every C411 torrent pays Zen, so one budget ordered by Zen per GB of disk is simpler and loses nothing.
- **A floor that spares the Library**: rejected by the Admin, the floor holds for every C411 download.

## Consequences

The profile API of C411 becomes a dependency of ratio: without it, ratio cannot read the Account ratio and spends no download on C411. Zen is spent by hand in the shop of C411, no API key can buy. The Zen of each Source is an estimate from the formula of C411, only the nightly total is exact. Hundreds of small torrents make qBittorrent hold many more torrents than before.
