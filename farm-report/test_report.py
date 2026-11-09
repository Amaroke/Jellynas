import datetime
import io
import json
import tempfile
import unittest
import urllib.error
import zlib
from pathlib import Path

from report import (
    ROSE,
    VIOLET,
    Webhook,
    chunks,
    cropped,
    embeds,
    hidden_height,
    multipart,
    next_hour,
    publish,
    report,
    report_day,
)

PARIS = datetime.timezone(datetime.timedelta(hours=1))
SUMMARY = {
    "updated_at": "2026-11-09T13:00:00+00:00",
    "account": {"ratio": 1.4567, "floor": 1.2, "left_gb": 123.45, "paused": False, "zen": 12345, "week_uploaded_gb": 67.8},
}


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
    def test_a_day_starts_at_6(self):
        self.assertEqual(report_day(at(9, 5, 59)), "2026-11-08")
        self.assertEqual(report_day(at(9, 6)), "2026-11-09")

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
    def test_the_message_holds_the_pictures_of_the_board_and_of_the_farm_disk(self):
        self.assertEqual(
            embeds(SUMMARY, at(9, 14)),
            [
                {"color": VIOLET, "image": {"url": "attachment://part1.png"}},
                {"color": VIOLET, "image": {"url": "attachment://partdisk.png"}, "timestamp": "2026-11-09T14:00:00+01:00"},
            ],
        )

    def test_a_pause_under_the_floor_turns_the_message_rose(self):
        paused = embeds({"account": {**SUMMARY["account"], "paused": True}}, at(9, 14))
        self.assertEqual([e["color"] for e in paused], [ROSE, ROSE])


class PublishTest(unittest.TestCase):
    def test_the_first_report_posts_a_message(self):
        webhook = FakeWebhook()
        state = publish(webhook, {}, SUMMARY, [b"a", b"b"], at(9, 14))
        self.assertEqual(state, {"day": "2026-11-09", "message_id": "m1"})
        kind, _, payload, files = webhook.calls[0]
        self.assertEqual(kind, "post")
        self.assertEqual(files, [("part1.png", b"a"), ("partdisk.png", b"b")])
        self.assertEqual(payload["attachments"], [{"id": 0, "filename": "part1.png"}, {"id": 1, "filename": "partdisk.png"}])

    def test_each_hour_of_the_day_edits_the_same_message(self):
        webhook = FakeWebhook()
        state = {"day": "2026-11-09", "message_id": "m0"}
        self.assertEqual(publish(webhook, state, SUMMARY, [b"a", b"b"], at(10, 5)), state)
        self.assertEqual(webhook.calls[0][:2], ("edit", "m0"))
        self.assertEqual(webhook.calls[0][2]["embeds"][-1]["timestamp"], "2026-11-10T05:00:00+01:00")

    def test_6_oclock_posts_a_new_message_and_keeps_the_previous_one(self):
        webhook = FakeWebhook()
        state = publish(webhook, {"day": "2026-11-09", "message_id": "m0"}, SUMMARY, [b"a", b"b"], at(10, 6))
        self.assertEqual(state, {"day": "2026-11-10", "message_id": "m1"})
        self.assertEqual([call[0] for call in webhook.calls], ["post"])

    def test_a_deleted_message_is_posted_again(self):
        webhook = FakeWebhook(missing=True)
        state = publish(webhook, {"day": "2026-11-09", "message_id": "m0"}, SUMMARY, [b"a", b"b"], at(9, 15))
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


if __name__ == "__main__":
    unittest.main()
