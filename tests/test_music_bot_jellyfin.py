import http.server
import json
import threading
import unittest
import urllib.parse

from jellyfin import MENU_SIZE, Hit, Jellyfin
from player import Track

KEY = "secret-key"


def audio(number, path=True):
    item = {"Id": f"t{number}", "Name": f"Song {number}", "AlbumArtist": "Artist", "Album": "Album"}
    if path:
        item["Path"] = f"/data/media/music/{number}.flac"
    return item


class FakeJellyfin:
    def __init__(self, found):
        self.found = found
        self.queries = []
        self.server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), self.handler())
        threading.Thread(target=self.server.serve_forever, daemon=True).start()
        self.url = f"http://127.0.0.1:{self.server.server_port}"

    def close(self):
        self.server.shutdown()
        self.server.server_close()

    def handler(self):
        fake = self

        class Handler(http.server.BaseHTTPRequestHandler):
            def log_message(self, *args):
                pass

            def do_GET(self):
                parsed = urllib.parse.urlparse(self.path)
                query = dict(urllib.parse.parse_qsl(parsed.query))
                fake.queries.append((parsed.path, query, self.headers.get("Authorization")))
                items = fake.found(parsed.path, query)
                data = json.dumps({"Items": items[: int(query.get("Limit", len(items)))],
                                   "TotalRecordCount": len(items)}).encode()
                self.send_response(200)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(data)))
                self.end_headers()
                self.wfile.write(data)

        return Handler


class JellyfinTest(unittest.TestCase):
    def serve(self, found):
        fake = FakeJellyfin(found)
        self.addCleanup(fake.close)
        return fake, Jellyfin(fake.url, KEY)

    def test_search_lists_artists_albums_playlists_then_tracks(self):
        by_kind = {
            "MusicArtist": [{"Id": "a", "Name": "Daft Punk"}],
            "MusicAlbum": [{"Id": "b", "Name": "Discovery", "AlbumArtist": "Daft Punk"}],
            "Playlist": [{"Id": "c", "Name": "Party"}],
            "Audio": [audio(1)],
        }
        fake, jellyfin = self.serve(lambda path, query: by_kind[query["IncludeItemTypes"]])
        hits, hidden = jellyfin.search("daft")
        self.assertEqual(["MusicArtist", "MusicAlbum", "Playlist", "Audio"], [hit.kind for hit in hits])
        self.assertEqual("Album by Daft Punk", hits[1].detail)
        self.assertEqual(0, hidden)
        self.assertTrue(all(auth == f'MediaBrowser Token="{KEY}"' for _, _, auth in fake.queries))
        self.assertTrue(all(query["searchTerm"] == "daft" for _, query, _ in fake.queries))

    def test_search_keeps_the_menu_size_and_counts_the_rest(self):
        by_kind = {
            "MusicArtist": [{"Id": f"a{n}", "Name": f"Artist {n}"} for n in range(10)],
            "MusicAlbum": [{"Id": f"b{n}", "Name": f"Album {n}"} for n in range(20)],
            "Playlist": [],
            "Audio": [audio(n) for n in range(40)],
        }
        _, jellyfin = self.serve(lambda path, query: by_kind[query["IncludeItemTypes"]])
        hits, hidden = jellyfin.search("x")
        self.assertEqual(MENU_SIZE, len(hits))
        self.assertEqual(["MusicArtist"] * 10 + ["MusicAlbum"] * 15, [hit.kind for hit in hits])
        self.assertEqual(70 - MENU_SIZE, hidden)

    def test_album_tracks_come_in_disc_and_track_order(self):
        fake, jellyfin = self.serve(lambda path, query: [audio(1), audio(2, path=False), audio(3)])
        tracks = jellyfin.tracks(Hit("MusicAlbum", "b", "Discovery", ""))
        self.assertEqual(["t1", "t3"], [track.id for track in tracks])
        path, query, _ = fake.queries[0]
        self.assertEqual(("/Items", "b", "ParentIndexNumber,IndexNumber,SortName"),
                         (path, query["ParentId"], query["SortBy"]))

    def test_playlist_tracks_come_from_the_playlist(self):
        fake, jellyfin = self.serve(lambda path, query: [audio(5)])
        jellyfin.tracks(Hit("Playlist", "c", "Party", ""))
        self.assertEqual("/Playlists/c/Items", fake.queries[0][0])

    def test_artist_tracks_come_by_artist(self):
        fake, jellyfin = self.serve(lambda path, query: [audio(5)])
        jellyfin.tracks(Hit("MusicArtist", "a", "Daft Punk", ""))
        self.assertEqual("a", fake.queries[0][1]["ArtistIds"])

    def test_library_is_every_track_with_a_file(self):
        _, jellyfin = self.serve(lambda path, query: [audio(1), audio(2, path=False)])
        tracks = jellyfin.library()
        self.assertEqual(["/data/media/music/1.flac"], [track.path for track in tracks])
        self.assertEqual("Artist - Song 1", tracks[0].label)


class FakeCovers:
    def __init__(self, covers):
        self.covers = covers
        self.server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), self.handler())
        threading.Thread(target=self.server.serve_forever, daemon=True).start()
        self.url = f"http://127.0.0.1:{self.server.server_port}"

    def close(self):
        self.server.shutdown()
        self.server.server_close()

    def handler(self):
        covers = self.covers

        class Handler(http.server.BaseHTTPRequestHandler):
            def log_message(self, *args):
                pass

            def do_GET(self):
                item_id = urllib.parse.urlparse(self.path).path.split("/")[2]
                if item_id not in covers:
                    self.send_error(404)
                    return
                self.send_response(200)
                self.send_header("Content-Length", str(len(covers[item_id])))
                self.end_headers()
                self.wfile.write(covers[item_id])

        return Handler


class CoverTest(unittest.TestCase):
    def cover(self, covers, track):
        fake = FakeCovers(covers)
        self.addCleanup(fake.close)
        return Jellyfin(fake.url, KEY).cover(track)

    def test_cover_is_the_image_of_the_track(self):
        track = Track("t1", "Song", "Artist", "Album", "/x.flac", "a1")
        self.assertEqual(b"track", self.cover({"t1": b"track", "a1": b"album"}, track))

    def test_cover_falls_back_to_the_album(self):
        track = Track("t1", "Song", "Artist", "Album", "/x.flac", "a1")
        self.assertEqual(b"album", self.cover({"a1": b"album"}, track))

    def test_no_cover_without_an_image(self):
        self.assertIsNone(self.cover({}, Track("t1", "Song", "Artist", "Album", "/x.flac")))


if __name__ == "__main__":
    unittest.main()
