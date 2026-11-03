# Tdarr picks the tracks to keep from the file alone

ADR 0001 keeps French and original audio, and ADR 0003 picks the subtitle languages from the tags of Sonarr and Radarr. Tdarr cannot see those tags nor the original language of a work, so its flow decides from the tracks of the file.

## Decision

- **Original audio**: the language of the first audio track that is not French. Every audio track in French or in that language is kept, the others are removed. A file with only French audio keeps only French. A track without a language counts as one more language, so a file whose only track has none keeps it.
- **Subtitles**: French and Japanese when the original audio is Japanese, French and English otherwise, forced subtitles included. A subtitle track without a language is removed.
- **Video**: an H265 stream within 1920x1080 is copied as is. Any other is encoded with libx265 at CRF 22, preset medium, on the CPU, scaled down to fit within 1920x1080 when larger. A file with nothing to change is not rewritten.
- **MKV conformity**: MP4 text subtitles are converted to SubRip, cover pictures and data streams are removed, extra video streams are removed.

## Considered Options

- **Asking Radarr and Sonarr for the original language and the tags**: rejected. The flow would need their API keys, which would end up in the committed export, and a network call per file.
- **One library per subtitle profile**: rejected. Anime movies live in the movies folder, so the folder does not tell an anime apart.
- **Keeping French, English and Japanese subtitles everywhere**: rejected. ADR 0003 keeps English subtitles out of anime and Japanese ones out of the rest.

## Consequences

A release whose first foreign track is a dub, not the original, keeps that dub instead. Recyclarr refuses most such releases, since it requires the original audio. A Japanese live action movie gets French and Japanese subtitles in the file, while Bazarr adds English ones next to it. A French original work keeps its embedded French and English subtitles even though Bazarr downloads none. A 4K HDR source scaled down keeps HDR10 but loses Dolby Vision, and a Dolby Vision only source plays with wrong colours, which is why Recyclarr ranks 1080p above 4K.
