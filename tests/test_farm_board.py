import datetime
import io
import json
import tempfile
import threading
import unittest
import urllib.error
import urllib.parse
import urllib.request
import zlib
from pathlib import Path

from board import (
    ROSE,
    VIOLET,
    Webhook,
    chunks,
    control_of,
    cropped,
    embeds,
    hidden_height,
    multipart,
    next_hour,
    publish,
    report,
    report_day,
    serve,
)
from page import board_page

PARIS = datetime.timezone(datetime.timedelta(hours=1))
SUMMARY = {"updated_at": "2026-11-09T13:00:00+00:00", "account": {"ratio": 1.4567, "floor": 1.2, "paused": False, "zen": 12345}}
IMAGES = [b"a", b"b", b"c"]
NAMES = [("part1.png", b"a"), ("part2.png", b"b"), ("part3.png", b"c")]


def at(day, hour, minute=0):
    return datetime.datetime(2026, 11, day, hour, minute, tzinfo=PARIS)


class FakeWebhook:
    def __init__(self, missing=False):
        self.calls = []
        self.missing = missing

    def post(self, payload, files):
        self.calls.append(("post", None, payload, files))
        return f"m{len(self.calls)}"

    def edit(self, message_id, payload, files):
        self.calls.append(("edit", message_id, payload, files))
        if self.missing:
            raise urllib.error.HTTPError("url", 404, "Unknown Message", {}, io.BytesIO(b""))


class Response(io.BytesIO):
    def __enter__(self):
        return self

    def __exit__(self, *args):
        self.close()


class ReportDayTest(unittest.TestCase):
    def test_a_day_starts_at_midnight(self):
        self.assertEqual(report_day(at(8, 23, 59)), "2026-11-08")
        self.assertEqual(report_day(at(9, 0)), "2026-11-09")

    def test_the_next_run_is_on_the_next_hour(self):
        self.assertEqual(next_hour(at(9, 14, 7)), at(9, 15))
        self.assertEqual(next_hour(at(9, 23, 59)), at(10, 0))


def png(width, rows):
    def chunk(kind, data):
        return len(data).to_bytes(4) + kind + data + zlib.crc32(kind + data).to_bytes(4)

    header = width.to_bytes(4) + len(rows).to_bytes(4) + bytes([8, 2, 0, 0, 0])
    pixels = b"".join(bytes(1) + bytes(row) for row in rows)
    return bytes([137, 80, 78, 71, 13, 10, 26, 10]) + chunk(b"IHDR", header) + chunk(b"IDAT", zlib.compress(pixels)) + chunk(b"IEND", b"")


class PictureTest(unittest.TestCase):
    def test_the_height_chromium_hides_is_the_window_minus_the_viewport(self):
        self.assertEqual(hidden_height("<html><head></head><body>913</body></html>"), 87)

    def test_cropping_keeps_the_top_rows_of_the_picture(self):
        rows = [[n] * 6 for n in range(5)]
        found = dict(chunks(cropped(png(2, rows), 3)))
        self.assertEqual(int.from_bytes(found[b"IHDR"][4:8]), 3)
        self.assertEqual(zlib.decompress(found[b"IDAT"]), b"".join(bytes(1) + bytes(row) for row in rows[:3]))


class EmbedsTest(unittest.TestCase):
    def test_the_message_holds_the_pictures_of_the_three_parts_of_the_board(self):
        self.assertEqual(
            embeds(SUMMARY, at(9, 14)),
            [
                {"color": VIOLET, "image": {"url": "attachment://part1.png"}},
                {"color": VIOLET, "image": {"url": "attachment://part2.png"}},
                {"color": VIOLET, "image": {"url": "attachment://part3.png"}, "timestamp": "2026-11-09T14:00:00+01:00"},
            ],
        )

    def test_a_pause_under_the_floor_turns_the_message_rose(self):
        paused = embeds({"account": {**SUMMARY["account"], "paused": True}}, at(9, 14))
        self.assertEqual([e["color"] for e in paused], [ROSE, ROSE, ROSE])


class PublishTest(unittest.TestCase):
    def test_the_first_report_posts_a_message(self):
        webhook = FakeWebhook()
        state = publish(webhook, {}, SUMMARY, IMAGES, at(9, 14))
        self.assertEqual(state, {"day": "2026-11-09", "message_id": "m1"})
        kind, _, payload, files = webhook.calls[0]
        self.assertEqual(kind, "post")
        self.assertEqual(files, NAMES)
        self.assertEqual([a["filename"] for a in payload["attachments"]], ["part1.png", "part2.png", "part3.png"])

    def test_each_hour_of_the_day_edits_the_same_message(self):
        webhook = FakeWebhook()
        state = {"day": "2026-11-09", "message_id": "m0"}
        self.assertEqual(publish(webhook, state, SUMMARY, IMAGES, at(9, 23)), state)
        self.assertEqual(webhook.calls[0][:2], ("edit", "m0"))
        self.assertEqual(webhook.calls[0][2]["embeds"][-1]["timestamp"], "2026-11-09T23:00:00+01:00")

    def test_midnight_posts_a_new_message_and_keeps_the_previous_one(self):
        webhook = FakeWebhook()
        state = publish(webhook, {"day": "2026-11-09", "message_id": "m0"}, SUMMARY, IMAGES, at(10, 0))
        self.assertEqual(state, {"day": "2026-11-10", "message_id": "m1"})
        self.assertEqual([call[0] for call in webhook.calls], ["post"])

    def test_a_deleted_message_is_posted_again(self):
        webhook = FakeWebhook(missing=True)
        state = publish(webhook, {"day": "2026-11-09", "message_id": "m0"}, SUMMARY, IMAGES, at(9, 15))
        self.assertEqual([call[0] for call in webhook.calls], ["edit", "post"])
        self.assertEqual(state["message_id"], "m2")


class WebhookTest(unittest.TestCase):
    def test_posts_and_edits_through_the_webhook_api(self):
        requests = []

        def opener(request, timeout):
            requests.append(request)
            return Response(b'{"id": "42"}')

        webhook = Webhook("123", "abc", opener)
        self.assertEqual(webhook.post({"embeds": []}, [("part1.png", b"png")]), "42")
        webhook.edit("42", {"embeds": []}, [])
        self.assertEqual(
            [(r.get_method(), r.full_url) for r in requests],
            [
                ("POST", "https://discord.com/api/v10/webhooks/123/abc?wait=true"),
                ("PATCH", "https://discord.com/api/v10/webhooks/123/abc/messages/42"),
            ],
        )
        self.assertIn(b'name="files[0]"; filename="part1.png"', requests[0].data)
        self.assertTrue(requests[0].get_header("User-agent").startswith("DiscordBot"))

    def test_the_payload_travels_as_json_beside_the_files(self):
        body, content_type = multipart({"embeds": [{"title": "Farm"}]}, [("part1.png", b"\x89PNG")])
        boundary = content_type.split("boundary=")[1].encode()
        self.assertTrue(body.endswith(b"--" + boundary + b"--\r\n"))
        self.assertIn(b'name="payload_json"', body)
        self.assertIn(json.dumps({"embeds": [{"title": "Farm"}]}).encode(), body)
        self.assertIn(b"\x89PNG", body)


class ReportTest(unittest.TestCase):
    def test_keeps_the_message_of_the_day_in_the_state(self):
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "state.json"
            webhook = FakeWebhook()
            for hour in (14, 15):
                report(webhook, path, at(9, hour), lambda: SUMMARY, lambda part, _: part.encode())
            self.assertEqual(json.loads(path.read_text()), {"day": "2026-11-09", "message_id": "m1"})
            self.assertEqual([call[0] for call in webhook.calls], ["post", "edit"])

    def test_a_failure_publishes_nothing_and_waits_for_the_next_hour(self):
        def broken():
            raise OSError("ratio is down")

        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "state.json"
            webhook = FakeWebhook()
            report(webhook, path, at(9, 14), broken, lambda part, _: b"")
            self.assertEqual(webhook.calls, [])
            self.assertFalse(path.exists())


class RefreshTest(unittest.TestCase):
    def start(self):
        self.wake = threading.Event()
        server = serve(self.wake, port=0)
        self.addCleanup(server.server_close)
        self.addCleanup(server.shutdown)
        return f"http://127.0.0.1:{server.server_address[1]}"

    def test_a_refresh_wakes_the_report(self):
        url = self.start()
        request = urllib.request.Request(f"{url}/refresh", data=b"", method="POST")
        with urllib.request.urlopen(request, timeout=10) as response:
            self.assertEqual(202, response.status)
        self.assertTrue(self.wake.is_set())

    def test_another_path_wakes_nothing(self):
        url = self.start()
        request = urllib.request.Request(f"{url}/other", data=b"", method="POST")
        with self.assertRaises(urllib.error.HTTPError) as caught:
            urllib.request.urlopen(request, timeout=10)
        caught.exception.close()
        self.assertFalse(self.wake.is_set())


def entry(n, **changes):
    return {
        "day": f"2027-01-{n:02d}", "floor": 1.2, "ratio": 1.5, "left_gb": 20.0, "uploaded_gb": 60.0, "actual_gb": 5.0,
        "downloaded_gb": 40.0, "downloaded_day_gb": 1.5, "zen": 25, "earned": 25, "zen_paid": 3, "seeds": 199, **changes,
    }


BOARD = {
    "updated_at": "2027-01-30T08:00:00+00:00",
    "account": {
        "ratio": 1.5, "floor": 1.2, "paused": False, "allowed": True, "uploaded_gb": 60, "downloaded_gb": 40,
        "actual_gb": 10, "credit_gb": 50, "zen": 616, "class": "swarm_keeper", "grade": ["bronze", 3],
        "voies": {"seed": 3, "specialization": 6},
    },
    "tiers": [
        {"name": "blocked", "ratio": 0.8, "gap_gb": -35.0}, {"name": "even", "ratio": 1.0, "gap_gb": -20.0},
        {"name": "floor", "ratio": 1.2, "gap_gb": -10.0}, {"name": "goal", "ratio": 2.0, "gap_gb": 20.0},
    ],
    "history": [entry(n) for n in range(1, 31)],
    "seeds": 199,
    "known": 190,
    "voies": {"seed": {"done": 3, "total": 19}, "specialization": {"done": 6, "total": 9}},
    "challenges": [
        {"key": "quantity_seed", "voie": "seed", "level": 3, "levels": 5, "done": False, "description": "200 seeds",
         "current": 199, "target": 200, "switch": "quantity_seed"},
        {"key": "duration_seed", "voie": "seed", "level": 1, "levels": 5, "done": False,
         "description": "25 seeds pendant 7 jours", "current": 3, "target": 25, "switch": None},
    ],
    "categories": [
        {"slug": "films-videos", "name": "Films & Vidéos", "count": 75, "target": 100, "switch": "volume:films-videos"},
        {"slug": "ebook", "name": "Ebook", "count": 36, "target": 50, "switch": "volume:ebook"},
    ],
    "controls": {
        "switches": {
            "free": True, "twins": True, "paid": False, "quantity_seed": True, "clean_public": True, "drop_twins": True,
            "hold": True, "adopt": False, "volume:films-videos": False, "volume:ebook": True,
        },
        "numbers": {"farm_size_gb": 100, "floor": 1.2, "daily_gb": 5, "max_size_gb": 1},
    },
    "farm": {"budget_gb": 100, "slices": [{"source": "free", "gb": 30.5}, {"source": "quest", "gb": 9.5}]},
    "sources": [
        {"source": "free", "torrents": 12, "downloaded_gb": 0, "uploaded_gb": 80.5, "ratio": None,
         "week_downloaded_gb": 0, "week_uploaded_gb": 20.25, "week_ratio": None, "farm_days": 40.5},
    ],
}


class PageTest(unittest.TestCase):
    def test_the_first_part_shows_the_figures_the_tiers_and_three_curves(self):
        page = board_page(BOARD, "1", "fr")
        for text in ("Ratio du compte", "616", "Bronze III", "199", "Paliers de ratio", "téléchargement bloqué", "objectif"):
            self.assertIn(text, page)
        self.assertIn("Palier suivant 2,0, objectif : 20,0 Go à envoyer", page)
        self.assertIn("Palier en dessous 1,2, plancher : 10,0 Go de marge", page)
        self.assertEqual(page.count("<figure>"), 3)
        self.assertNotIn('action="/controls"', page)

    def test_the_second_part_shows_the_quests_with_a_switch_only_where_the_farm_can_help(self):
        page = board_page(BOARD, "2", "fr")
        self.assertIn("199 / 200", page)
        self.assertIn("75 / 100", page)
        self.assertIn('name="name" value="quantity_seed"', page)
        self.assertIn('name="name" value="volume:ebook"', page)
        self.assertNotIn('value="duration_seed"', page)
        self.assertIn("indicateur", page)
        self.assertIn('class="switch on"', page)
        self.assertIn('name="name" value="volume:films-videos"><input type="hidden" name="value" value="on">', page)

    def test_the_third_part_shows_the_switches_the_settings_the_disk_and_the_earnings(self):
        page = board_page(BOARD, "3", "fr")
        for text in ("Grabs payants frais", "Pause Prowlarr sous le plancher", "Budget de la Farm (Go)", "Disque de la Farm", "Gains par Source"):
            self.assertIn(text, page)
        self.assertIn('type="number" name="value" step="0.05" min="0.5" max="5" value="1.2"', page)

    def test_without_a_part_the_page_shows_everything_on_a_dark_background(self):
        page = board_page(BOARD)
        self.assertIn('class="standalone"', page)
        self.assertIn("Account ratio", page)
        self.assertIn("Seed count", page)

    def test_without_history_the_page_says_so(self):
        self.assertIn("No history yet", board_page({"history": []}, "1"))
        self.assertIn("No history yet", board_page(None, "2"))


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *args):
        return None


class ServerTest(unittest.TestCase):
    def start(self, sender=None, reader=None):
        self.wake = threading.Event()
        self.sent = []
        server = serve(
            self.wake, port=0, board_reader=reader or (lambda: BOARD),
            sender=sender or (lambda name, value: self.sent.append((name, value))),
        )
        self.addCleanup(server.server_close)
        self.addCleanup(server.shutdown)
        return f"http://127.0.0.1:{server.server_address[1]}"

    def post(self, url, fields, origin=None):
        headers = {"Origin": origin} if origin else {}
        body = urllib.parse.urlencode(fields).encode()
        request = urllib.request.Request(f"{url}/controls", data=body, method="POST", headers=headers)
        with self.assertRaises(urllib.error.HTTPError) as caught:
            urllib.request.build_opener(NoRedirect).open(request, timeout=10)
        caught.exception.close()
        return caught.exception

    def test_the_page_is_served_with_its_policy(self):
        url = self.start()
        with urllib.request.urlopen(f"{url}/graphs?lang=fr&part=2", timeout=10) as response:
            self.assertIn("form-action 'self'", response.headers["Content-Security-Policy"])
            self.assertIn("Quêtes", response.read().decode())

    def test_a_switch_goes_to_ratio_then_back_to_its_part(self):
        url = self.start()
        answer = self.post(url, {"name": "paid", "value": "on", "part": "3", "lang": "fr"})
        self.assertEqual(answer.code, 303)
        self.assertEqual(answer.headers["Location"], "/graphs?lang=fr&part=3")
        self.assertEqual(self.sent, [("paid", True)])
        self.assertTrue(self.wake.is_set())

    def test_a_number_goes_to_ratio_as_a_number(self):
        url = self.start()
        self.post(url, {"name": "floor", "value": "1,5", "part": "3"}, origin=url)
        self.assertEqual(self.sent, [("floor", 1.5)])

    def test_a_form_of_another_site_is_refused(self):
        url = self.start()
        self.assertEqual(self.post(url, {"name": "paid", "value": "on"}, origin="https://evil.example").code, 403)
        self.assertEqual(self.sent, [])

    def test_a_value_ratio_refuses_is_a_bad_request(self):
        def refuse(name, value):
            raise ValueError("no")

        url = self.start(sender=refuse)
        self.assertEqual(self.post(url, {"name": "floor", "value": "high"}).code, 400)

    def test_the_page_keeps_the_last_board_while_ratio_is_down(self):
        boards = [BOARD]

        def reader():
            if not boards:
                raise OSError("down")
            return boards.pop()

        url = self.start(reader=reader)
        for _ in range(2):
            with urllib.request.urlopen(f"{url}/graphs?part=1", timeout=10) as response:
                self.assertIn("Account ratio", response.read().decode())


class ControlTest(unittest.TestCase):
    def test_on_and_off_are_booleans_and_the_rest_numbers(self):
        self.assertEqual(control_of({"name": "paid", "value": "on"}), ("paid", True))
        self.assertEqual(control_of({"name": "paid", "value": "off"}), ("paid", False))
        self.assertEqual(control_of({"name": "daily_gb", "value": "2.5"}), ("daily_gb", 2.5))
        with self.assertRaises(ValueError):
            control_of({"name": "daily_gb", "value": "lots"})


if __name__ == "__main__":
    unittest.main()
