# Tdarr normalises the whole Library to MKV H265 1080p

Every downloaded video is re-encoded by Tdarr to MKV H265 1080p, keeping only French and original audio and embedding French and English subtitles in the file. We want a single, compact format that most devices play directly, including offline on a phone with subtitles. Re-encoding is destructive: the original file is replaced and each pass loses quality, so going back means downloading again.

## Considered Options

- **Only prefer the right format at download time (Recyclarr)**: lossless, but files stay inconsistent when the ideal release does not exist. Kept as a complement to limit re-encoding.
- **MP4 instead of MKV**: better browser compatibility, but does not support every subtitle format nor several audio tracks cleanly.
- **H264 instead of H265**: plays everywhere without transcoding, but files are much larger.

## Consequences

The machine running Tdarr must encode H265 in hardware: the PC's RTX 5070 today, a recent Intel CPU with QuickSync on the future dedicated machine.
