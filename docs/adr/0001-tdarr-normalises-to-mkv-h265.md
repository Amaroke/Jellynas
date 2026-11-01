# Tdarr normalises the whole Library to MKV H265 1080p

Every downloaded video is normalised by Tdarr to MKV H265 1080p. We want a single, compact format that most devices play directly, including offline on a phone with subtitles.

## Decision

- **Container and codec**: MKV, H265.
- **Resolution**: the picture fits within 1920x1080 and keeps its aspect ratio, without black bars. A 4K source is scaled down. A square source becomes 1080x1080, an ultrawide one 1920x804. A source already within 1920x1080 keeps its resolution and is never scaled up.
- **Audio**: French and original audio only, a single track when the original is French.
- **Subtitles**: French and English only, forced subtitles included, embedded in the file.
- **Already compliant files**: a file already in MKV H265 within 1920x1080 is not re-encoded. Tdarr only removes the extra audio and subtitle tracks, without any quality loss.

## Considered Options

- **Only prefer the right format at download time (Recyclarr)**: kept as a complement. Lossless, it makes most downloads already compliant, but files stay inconsistent when the ideal release does not exist.
- **MP4 instead of MKV**: rejected. Better browser compatibility, but does not support every subtitle format nor several audio tracks cleanly.
- **H264 instead of H265**: rejected. Plays everywhere without transcoding, but files are much larger.

## Consequences

Re-encoding is destructive: the original file is replaced and each pass loses quality, so going back means downloading again. Skipping compliant files limits this loss to the files that need it.
