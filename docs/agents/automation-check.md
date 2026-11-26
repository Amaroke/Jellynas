# Automation check

The procedure the `check-radarr`, `check-sonarr` and `check-lazylibrarian`
skills follow. Reaching the NAS is in [nas-access.md](nas-access.md).

The Admin wants to see the state of their libraries first and act on it, not a
technical diagnosis. The report leads with tables of the library and offers
searches, and it names only the problems that stop downloads or lose files.

## 1. Run the check

Run the check of the app as [nas-access.md](nas-access.md) shows. Done when you
hold its JSON report, or the reason it could not run.

## 2. Read the library

The `facts` of the report hold the library, row by row:

- Radarr: `movies` (id, title, year, monitored, status, isAvailable, hasFile,
  quality, sizeGB, rootFolder) and `missing` (the monitored movies without a
  file).
- Sonarr: `series` (id, title, year, monitored, status, seriesType,
  episodeFiles, episodesWanted, episodesTotal, sizeGB, rootFolder) and
  `missing` (aired episodes missing per series). `episodesWanted` counts the
  aired monitored episodes, so a series is complete when `episodeFiles`
  reaches it.
- LazyLibrarian: `authors` (AuthorID, AuthorName, Status, HaveBooks,
  TotalBooks, UnignoredBooks), `wanted` (BookID, BookName, Status), `snatched`
  and `stats`.

A root folder under `/data/media/private/<user>` belongs to the Private library
of that user.

## 3. Trace the problems

The report holds `findings` (most severe first), `errors` (endpoints that did
not answer) and the rest of the `facts`. Trace each finding and error to a cause
you can name, upstream when needed: a symptom in the app often starts in
Prowlarr, ratio, Gluetun or qBittorrent, whose state the report carries too.
Read further with `check.sh api` or the logs as [nas-access.md](nas-access.md)
shows, and with the README of this repo for how each service is meant to work.

These are how the stack is meant to run. Leave them out of the problems, and
mention them in one line only when they explain why something is not
downloading:

- **C411 on the Paused sync profile.** ratio keeps C411 out of Radarr, Sonarr
  and LazyLibrarian while the Account ratio is under the Ratio floor. The check
  raises no finding for it, the `prowlarr` facts show the profile.
- **ratio could not read the C411 profile once.** A low finding, it tries again
  on its next run.
- **qBittorrent answering 403.** It is reachable, it only refuses the request
  without login.
- **Errors 521 or 522 from servarr.com.** The update and time servers of Radarr
  and Sonarr are down, nothing on the NAS.
- **Missing movies with the status announced.** Not released yet.
- **Failures "Manually marked as failed".** The Admin marked them.
- **A few Gluetun restarts.** The VPN came back on its own when the containers
  run healthy.

## 4. Write the report

In the language of the conversation, with these sections:

1. **Verdict**: one line, whether new releases come in and why not.
2. **Library**: tables built from the facts of step 2.
   - Radarr: the movies missing (title, year, status, folder), then the movies
     below the cutoff when the Admin asks, then a count by root folder.
   - Sonarr: one row per incomplete series (title, files over wanted, status,
     folder), then a count of the complete ones by root folder.
   - LazyLibrarian: one row per author (have over total), then the wanted
     books.
   Keep the ids in the rows: they are what a search needs.
3. **Problems**: only what stops downloads or loses files, each with its cause
   and evidence, most severe first. Say "none" when there is none.
4. **Actions**: the searches of step 5 that fit the tables, then for each
   problem with a fix the exact change and what it changes.

## 5. Launch a search

A search changes nothing but the queue, still send it only once the Admin
picked it. Each is one `check.sh api` call as [nas-access.md](nas-access.md)
shows, and the reply names the command it started.

Radarr, `POST /api/v3/command` with one of these bodies:

```text
{"name": "MoviesSearch", "movieIds": [1, 2]}
{"name": "MissingMoviesSearch"}
{"name": "CutOffUnmetMoviesSearch"}
```

Sonarr, `POST /api/v3/command` with one of these bodies:

```text
{"name": "EpisodeSearch", "episodeIds": [1, 2]}
{"name": "SeriesSearch", "seriesId": 1}
{"name": "MissingEpisodeSearch"}
{"name": "CutoffUnmetEpisodeSearch"}
```

LazyLibrarian, `GET` with one book or every wanted book:

```text
cmd=searchBook&id=<BookID>
cmd=forceBookSearch
```

For example:

```sh
ssh jellynas "sudo sh /mnt/tank/jellynas/scripts/check.sh \
  api radarr POST /api/v3/command '{\"name\":\"MissingMoviesSearch\"}'"
```

Sonarr searches an anime series episode by episode, so to fetch a few
episodes search their ids (`GET /api/v3/episode?seriesId=<id>` lists them)
rather than the whole series. Searching every missing item at once fires many
queries. Nyaa has a query limit
in Prowlarr, and the app disables an indexer that hits it for hours. Prefer a
few items, or one bulk search, never several bulk searches in a row.

After a search, read the queue (`GET /api/v3/queue` or `GET 'cmd=getSnatched'`)
a minute later to show what it grabbed.

## 6. Apply a fix

Apply a fix only after the Admin said yes to that one fix. One fix at a time,
then read back the state it changed to show it took. A change that belongs in
the repo (a compose file, `policy`, `ratio`) is a pull request, not a call on
the NAS.
