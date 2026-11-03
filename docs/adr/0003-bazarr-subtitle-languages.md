# Bazarr subtitles serve the original audio

Releases already carry French audio, as ADR 0002 requires, so subtitles are only for watching in the original audio. Bazarr picks a language profile for each work from the tags that Sonarr and Radarr set on their own.

## Decision

- **Movies and shows**: French and English subtitles, with the `French and English` profile, the default one.
- **Anime**: French and Japanese subtitles, with the `French and Japanese` profile, picked by the `anime` tag. Sonarr sets it on every show of the Anime series type. Radarr sets it on every movie with the `Anime 1080p` quality profile, since it cannot tell an anime movie apart.
- **French original works**: no subtitles. The `french-original` tag, set on every work whose original language is French, removes the profile.
- **Embedded tracks**: a subtitle track already inside the file counts as found and is not downloaded again.
- **No cutoff**: Bazarr keeps searching each language of the profile until it finds it.

## Considered Options

- **French subtitles only**: rejected. The Admin wants English ones too for movies and shows.
- **English as a fallback, with a cutoff on French**: rejected. Both languages are wanted for every work in a foreign language.
- **Subtitles even with French audio**: rejected. Subtitles only serve the original audio, and a French work has no other one.
- **Forced subtitles only**: rejected. They cover foreign passages of a French dub, not watching in the original audio.

## Consequences

A French work never gets subtitles, not even English ones. An anime movie gets its anime profile only once the Admin assigns it `Anime 1080p` in Radarr. Japanese subtitles come mostly from Jimaku.cc, which needs an AniDB API client.
