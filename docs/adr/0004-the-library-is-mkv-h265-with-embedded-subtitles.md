# The Library is MKV H265 1080p with its subtitles embedded

Every video of the Library is normalised by Tdarr to one compact format that most devices play directly, offline on a phone included, with its subtitles inside the file. Recyclarr makes most downloads compliant already ([ADR 0002](0002-recyclarr-prefers-french-h265-1080p.md)), Bazarr fetches the missing subtitles, and Tdarr fixes the rest.

## Decision

- **Format**: MKV, H265, the picture within 1920x1080 at its own aspect ratio, never scaled up. An H265 stream within 1920x1080 is copied as is, any other is encoded with libx265 at CRF 22, preset medium, on the CPU. A file with nothing to change is not rewritten.
- **Audio**: French and the original audio only. Tdarr cannot see the original language of a work, so it takes the language of the first audio track that is not French. A track without a language counts as one more language.
- **Subtitles**: only for watching in the original audio. French and English for movies and shows, French and Japanese for anime, none for a French original work, forced subtitles included. Bazarr picks the profile from the `anime` and `french-original` tags that Sonarr and Radarr set, and searches each language without a cutoff. Tdarr keeps the subtitle tracks of the release by the original audio: French and Japanese when it is Japanese, French and English otherwise.
- **Embedded, never beside the video**: Tdarr embeds every subtitle file that Bazarr saved next to the video, whatever its language, and deletes the files once the new video is in place. Bazarr requeues the video in Tdarr after each download, and never upgrades a subtitle, which would add a second track of the same language.
- **Rescan**: once the new file is in place, Tdarr asks Radarr or Sonarr to rescan its folder, so they and Bazarr follow a new extension at once.

## Considered Options

- **Only preferring the right format at download time**: kept as a complement, but files stay inconsistent when the ideal release does not exist.
- **MP4**: rejected. It does not hold every subtitle format nor several audio tracks cleanly.
- **H264**: rejected. Files are much larger.
- **Asking Radarr and Sonarr for the original language and the tags**: rejected. The tracks of the file give the same answer for nearly every release Recyclarr lets through, without a lookup per file.
- **Applying the language rules to the subtitles of Bazarr**: rejected. A Japanese live action movie gets English subtitles from Bazarr, Tdarr would remove them, and Bazarr would download them again at every search.
- **Letting Bazarr embed its subtitles**: rejected. Bazarr and Tdarr would rewrite the same video, possibly at the same time.
- **French subtitles only, or English only as a fallback**: rejected. The Admin wants both for every work in a foreign language.
- **Subtitles with French audio**: rejected. A French work has no other audio to subtitle.

## Consequences

Re-encoding is destructive: going back means downloading again. A release whose first foreign track is a dub keeps that dub, and Recyclarr refuses most of them. An anime movie gets its anime subtitles only once the Admin assigns it `Anime 1080p` in Radarr, which cannot tell it apart. A subtitle file written while Tdarr already processes its video stays next to it until Bazarr finds another one or the Admin requeues the video. A 4K source scaled down keeps HDR10 but loses Dolby Vision, which is why Recyclarr ranks 1080p above 4K.
