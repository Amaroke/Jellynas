# Tdarr picks the tracks to keep and embeds the subtitles of Bazarr

ADR 0001 keeps French and original audio, and ADR 0003 picks the subtitle languages from the tags of Sonarr and Radarr. Tdarr cannot see those tags nor the original language of a work, so its flow decides from the tracks of the file. The Library holds no subtitle file, so a video downloaded by a Friend always carries its subtitles.

## Decision

- **Original audio**: the language of the first audio track that is not French. Every audio track in French or in that language is kept, the others are removed. A file with only French audio keeps only French. A track without a language counts as one more language, so a file whose only track has none keeps it.
- **Subtitles of the release**: French and Japanese when the original audio is Japanese, French and English otherwise, forced subtitles included. A subtitle track without a language is removed.
- **Subtitles of Bazarr**: every subtitle file named after the video with a language code, such as `.fr.srt` or `.ja.forced.ass`, is embedded with its language and its forced or hearing impaired flag, whatever its language, since Bazarr already picked it from the tags. Tdarr deletes the files once the new video is in place, never before.
- **Late subtitles**: Bazarr puts the video back in the Tdarr queue after each download, so a subtitle found after the first pass gets embedded too. Bazarr does not upgrade its subtitles, an upgrade would add a second track of the same language.
- **Video**: an H265 stream within 1920x1080 is copied as is. Any other is encoded with libx265 at CRF 22, preset medium, on the CPU, scaled down to fit within 1920x1080 when larger. A file with nothing to change is not rewritten.
- **MKV conformity**: MP4 text subtitles are converted to SubRip, cover pictures and data streams are removed, extra video streams are removed.
- **Radarr and Sonarr**: once the new file is in place, Tdarr asks Radarr for a movie, or Sonarr for a show or an anime, to rescan its folder, so they and Bazarr follow a new extension right away.

## Considered Options

- **Asking Radarr and Sonarr for the original language and the tags**: rejected. The tracks of the file give the same answer for nearly every release Recyclarr lets through, without a lookup per file.
- **Applying the language rules to the subtitles of Bazarr**: rejected. A Japanese live action movie gets English subtitles from Bazarr, Tdarr would remove them, and Bazarr would download them again at every search.
- **Letting Bazarr embed its subtitles itself**: rejected. Bazarr and Tdarr would rewrite the same video, possibly at the same time.
- **One library per subtitle profile**: rejected. Anime movies live in the movies folder, so the folder does not tell an anime apart.

## Consequences

A release whose first foreign track is a dub, not the original, keeps that dub instead. Recyclarr refuses most such releases, since it requires the original audio. A French original work keeps its embedded French and English subtitles even though Bazarr downloads none. A subtitle file that Bazarr writes while Tdarr is already processing the same video stays next to it until Bazarr finds another subtitle for it, or until the Admin requeues the video in Tdarr. A 4K HDR source scaled down keeps HDR10 but loses Dolby Vision, and a Dolby Vision only source plays with wrong colours, which is why Recyclarr ranks 1080p above 4K.
