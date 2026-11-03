# Recyclarr prefers French H265 1080p releases

Radarr and Sonarr pick the release to download from the profiles that Recyclarr syncs. The profiles favour releases that already match the Library format of ADR 0001, so that Tdarr rarely re-encodes, and they refuse what Tdarr cannot fix: a missing French audio track.

## Decision

There are two profiles in each tool. **MULTi 1080p** is for movies and shows. **Anime 1080p** is for anime: Seerr assigns it to anime shows in Sonarr, and the Admin assigns it by hand to anime movies in Radarr, since Radarr cannot tell them apart.

- **Quality order**: 1080p WEB or Bluray, then 720p WEB or Bluray, then 4K WEB or Bluray, then Remux 1080p, then Remux 4K. HDTV is refused for movies and shows. The anime profile accepts it within each resolution, since Sonarr reads many anime web releases without a source tag as HDTV. Radarr and Sonarr always rank quality before score, so the points below only break ties within the same quality.
- **Language, movies and shows**: French audio is required, and so is the original audio when the work is not French. A French work in French only is accepted. A title that says `MULTi` counts as French and original audio, whatever language Radarr guesses from it.
- **Language, anime**: a `MULTi` that announces French subtitles scores 8000, a VOSTFR 4000, a `MULTi` without that mention 0. An original audio without French audio or subtitles is refused, and so is a French audio without the original.
- **Canadian French**: a release marked VFQ, or `MULTi.FRENCH` by scene naming, is refused. A release with both dubs (VF2) is accepted.
- **Scores, at equal quality**: H265 +2000, release group tier 0 to +1800 from the TRaSH Guides, VFF or VFI announced +40, Bluray +20, streaming service +5, Repack or Proper +1 to +7. Each level outweighs the sum of the levels below it.
- **AV1**: -3000, with a minimum score of -5000, so an AV1 release is only taken when nothing else is.
- **Unwanted formats**: low quality groups, upscales, 3D and the other unwanted formats of the TRaSH Guides are refused.
- **Size**: the minimum Bluray sizes match the WEB ones, since the guide minimums are sized for H264 and refuse most H265 encodes. At equal score, the smallest release wins.
- **Upgrades**: always, as soon as a release scores higher.
- **Neutral**: HDR and Dolby Vision, movie editions, season packs.

## Considered Options

- **The French MULTi.VO profiles of the TRaSH Guides as they are**: kept as the base for group tiers, refused as is. They accept the original audio without French, penalise H265 at 1080p, and refuse a `MULTi` title as soon as Radarr reads its language as French only.
- **Accepting the original audio without French as a fallback**: rejected. French audio is the one thing Tdarr cannot add, and Friends watch in French.
- **VOSTFR before MULTi for anime**: rejected. A `MULTi` usually holds the same Japanese audio and subtitles plus a French dub.
- **Requiring French subtitles in every anime MULTi**: rejected. Only about one anime `MULTi` title in fifteen announces its subtitles, so most anime would only arrive in VOSTFR. Bazarr fills in the missing subtitles instead.
- **Group tier before codec**: rejected. An H265 release avoids a lossy Tdarr pass, which matters more than the gap between two good groups.

## Consequences

A show or movie that only exists in original audio without French is never downloaded, nor is an anime without French audio or subtitles. A `MULTi` title is trusted to hold French audio, which a mislabelled release can break. Upgrades replace files whenever a better release appears, which costs downloads, seeding and sometimes a Tdarr pass.
