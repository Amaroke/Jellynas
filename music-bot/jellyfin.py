import json
import urllib.error
import urllib.parse
import urllib.request
from dataclasses import dataclass

from player import Track

KINDS = ("MusicArtist", "MusicAlbum", "Playlist", "Audio")
MENU_SIZE = 25
COVER_SIZE = 300


@dataclass(frozen=True)
class Hit:
    kind: str
    id: str
    name: str
    detail: str


def artist_of(item):
    artists = item.get("Artists") or []
    return item.get("AlbumArtist") or (artists[0] if artists else "")


def track_of(item):
    return Track(
        item["Id"], item.get("Name", ""), artist_of(item), item.get("Album", ""), item["Path"], item.get("AlbumId", "")
    )


def hit_of(kind, item):
    name = item.get("Name", "")
    if kind == "MusicArtist":
        detail = "Artist"
    elif kind == "MusicAlbum":
        detail = f"Album by {artist_of(item)}" if artist_of(item) else "Album"
    elif kind == "Playlist":
        detail = "Playlist"
    else:
        detail = ", ".join(part for part in (artist_of(item), item.get("Album", "")) if part) or "Track"
    return Hit(kind, item["Id"], name, detail)


class Jellyfin:
    def __init__(self, url, key):
        self.url = url.rstrip("/")
        self.key = key

    def fetch(self, path, **params):
        query = urllib.parse.urlencode(params)
        request = urllib.request.Request(
            f"{self.url}{path}?{query}", headers={"Authorization": f'MediaBrowser Token="{self.key}"'}
        )
        with urllib.request.urlopen(request, timeout=30) as response:
            return response.read()

    def get(self, path, **params):
        return json.loads(self.fetch(path, **params))

    def cover(self, track):
        for item_id in (track.id, track.album_id):
            if not item_id:
                continue
            try:
                return self.fetch(f"/Items/{item_id}/Images/Primary", maxWidth=COVER_SIZE, format="Jpg")
            except urllib.error.HTTPError as error:
                if error.code != 404:
                    raise
        return None

    def items(self, path, **params):
        return self.get(path, **params).get("Items") or []

    def tracks_of(self, items):
        return [track_of(item) for item in items if item.get("Path")]

    def search(self, term):
        found, total = [], 0
        for kind in KINDS:
            page = self.get(
                "/Items",
                searchTerm=term,
                IncludeItemTypes=kind,
                Recursive="true",
                Limit=MENU_SIZE,
                EnableImages="false",
            )
            items = page.get("Items") or []
            total += page.get("TotalRecordCount", len(items))
            found.extend(hit_of(kind, item) for item in items[: MENU_SIZE - len(found)])
        return found, total - len(found)

    def tracks(self, hit):
        if hit.kind == "Audio":
            items = self.items("/Items", Ids=hit.id, Fields="Path")
        elif hit.kind == "MusicAlbum":
            items = self.items(
                "/Items",
                ParentId=hit.id,
                IncludeItemTypes="Audio",
                Recursive="true",
                SortBy="ParentIndexNumber,IndexNumber,SortName",
                Fields="Path",
            )
        elif hit.kind == "MusicArtist":
            items = self.items("/Items", ArtistIds=hit.id, IncludeItemTypes="Audio", Recursive="true", Fields="Path")
        else:
            items = self.items(f"/Playlists/{hit.id}/Items", Fields="Path")
        return self.tracks_of(items)

    def library(self):
        return self.tracks_of(
            self.items("/Items", IncludeItemTypes="Audio", Recursive="true", Fields="Path", EnableImages="false")
        )
