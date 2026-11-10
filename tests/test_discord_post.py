import contextlib
import datetime
import http.server
import io
import json
import tempfile
import threading
import unittest
import urllib.error
import urllib.request
from pathlib import Path
from unittest import mock

from discord_post import (
    MAX_LENGTH,
    next_run,
    publish,
    publish_all,
    read_env,
    render,
    serve,
    unwrap,
)

ROOT = Path(__file__).resolve().parent.parent


class Discord(http.server.ThreadingHTTPServer):
    def __init__(self):
        super().__init__(("127.0.0.1", 0), Handler)
        self.calls = []
        self.messages = {}
        threading.Thread(target=self.serve_forever, daemon=True).start()

    @property
    def url(self):
        return f"http://127.0.0.1:{self.server_address[1]}"


class Handler(http.server.BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def answer(self, code, body):
        data = json.dumps(body).encode()
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def body(self):
        return json.loads(self.rfile.read(int(self.headers["Content-Length"])))

    def do_POST(self):
        body = self.body()
        self.server.calls.append(("POST", self.path, body))
        message = str(len(self.server.messages) + 1)
        self.server.messages[message] = body["embeds"][0]["description"]
        self.answer(200, {"id": message})

    def do_PATCH(self):
        body = self.body()
        self.server.calls.append(("PATCH", self.path, body))
        message = self.path.rsplit("/", 1)[1]
        if message not in self.server.messages:
            self.answer(404, {"message": "Unknown Message"})
            return
        self.server.messages[message] = body["embeds"][0]["description"]
        self.answer(200, {"id": message})


class PublishTest(unittest.TestCase):
    def setUp(self):
        self.discord = Discord()
        self.addCleanup(self.discord.server_close)
        self.addCleanup(self.discord.shutdown)
        folder = tempfile.TemporaryDirectory()
        self.addCleanup(folder.cleanup)
        self.folder = Path(folder.name)
        self.message = self.folder / "links.md"
        self.message.write_text("https://jellyfin.${DOMAIN}:${CADDY_HTTPS_PORT}\n", encoding="utf-8")
        self.env = {
            "APPDATA_ROOT": str(self.folder / "appdata"),
            "DOMAIN": "example.com",
            "CADDY_HTTPS_PORT": "443",
            "DISCORD_LINKS_WEBHOOK_ID": "1234",
            "DISCORD_LINKS_WEBHOOK_TOKEN": "abcd",
        }

    def publish(self):
        return publish("links", self.message, self.env, self.discord.url, datetime.datetime(2026, 11, 10, 0, 30))

    def test_posts_the_message_filled_from_the_env(self):
        self.publish()
        method, path, body = self.discord.calls[0]
        self.assertEqual(("POST", "/1234/abcd?wait=true"), (method, path))
        self.assertEqual("", body["content"])
        footer = {"text": "Dernière mise à jour : 10/11/2026 à 00:30"}
        self.assertEqual([{"description": "https://jellyfin.example.com:443", "color": 0x60A5FA, "footer": footer}], body["embeds"])
        self.assertEqual({"parse": []}, body["allowed_mentions"])

    def test_the_first_heading_is_the_title_of_the_embed(self):
        self.message.write_text("# Liens\n\n## Jellyfin\nhttps://jellyfin.${DOMAIN}\n", encoding="utf-8")
        self.publish()
        embed = self.discord.calls[0][2]["embeds"][0]
        self.assertEqual("Liens", embed["title"])
        self.assertEqual("## Jellyfin\nhttps://jellyfin.example.com", embed["description"])

    def test_edits_the_same_message_the_next_time(self):
        self.publish()
        self.message.write_text("changed", encoding="utf-8")
        self.publish()
        self.assertEqual(("PATCH", "/1234/abcd/messages/1"), self.discord.calls[1][:2])
        self.assertEqual({"1": "changed"}, self.discord.messages)

    def test_posts_again_once_the_message_was_deleted(self):
        self.publish()
        self.discord.messages.clear()
        self.publish()
        self.assertEqual(["POST", "PATCH", "POST"], [call[0] for call in self.discord.calls])
        state = json.loads((self.folder / "appdata" / "discord-posts" / "state.json").read_text(encoding="utf-8"))
        self.assertEqual({"links/links.md": "1"}, state)

    def test_a_channel_without_its_own_color_gets_violet(self):
        self.env["DISCORD_FARM_WEBHOOK_ID"], self.env["DISCORD_FARM_WEBHOOK_TOKEN"] = "1", "a"
        publish("farm", self.message, self.env, self.discord.url)
        self.assertEqual(0xA78BFA, self.discord.calls[0][2]["embeds"][0]["color"])

    def test_another_file_is_another_message(self):
        self.publish()
        other = self.folder / "news.md"
        other.write_text("news", encoding="utf-8")
        publish("links", other, self.env, self.discord.url)
        self.assertEqual(["POST", "POST"], [call[0] for call in self.discord.calls])

    def test_refuses_a_channel_without_webhook(self):
        with self.assertRaisesRegex(SystemExit, "DISCORD_NEWS_WEBHOOK_ID"):
            publish("news", self.message, self.env, self.discord.url)
        self.assertEqual([], self.discord.calls)

    def test_refuses_a_variable_missing_from_the_env(self):
        del self.env["DOMAIN"]
        with self.assertRaisesRegex(SystemExit, "DOMAIN"):
            self.publish()
        self.assertEqual([], self.discord.calls)

    def test_refuses_a_message_too_long_for_discord(self):
        self.message.write_text("x" * (MAX_LENGTH + 1), encoding="utf-8")
        with self.assertRaisesRegex(SystemExit, "4096"):
            self.publish()


class DailyTest(unittest.TestCase):
    def test_runs_today_before_half_past_midnight(self):
        self.assertEqual(datetime.datetime(2026, 11, 10, 0, 30), next_run(datetime.datetime(2026, 11, 10, 0, 29)))

    def test_runs_tomorrow_from_half_past_midnight(self):
        self.assertEqual(datetime.datetime(2026, 11, 11, 0, 30), next_run(datetime.datetime(2026, 11, 10, 0, 30)))
        self.assertEqual(datetime.datetime(2026, 11, 11, 0, 30), next_run(datetime.datetime(2026, 11, 10, 18, 0)))

    def test_a_channel_that_fails_does_not_stop_the_others(self):
        discord = Discord()
        self.addCleanup(discord.server_close)
        self.addCleanup(discord.shutdown)
        with tempfile.TemporaryDirectory() as name:
            folder = Path(name)
            for post in ("links.md", "welcome.md"):
                (folder / post).write_text(post, encoding="utf-8")
            env = {
                "APPDATA_ROOT": str(folder / "appdata"),
                "DISCORD_LINKS_WEBHOOK_ID": "1234",
                "DISCORD_LINKS_WEBHOOK_TOKEN": "abcd",
            }
            out = io.StringIO()
            with contextlib.redirect_stdout(out):
                publish_all(env, folder, discord.url)
        self.assertEqual(["links.md"], [call[2]["embeds"][0]["description"] for call in discord.calls])
        self.assertIn("Cannot publish welcome.md", out.getvalue())


class ActionsTest(unittest.TestCase):
    def start(self, farm_report="http://127.0.0.1:9/refresh"):
        self.wake = threading.Event()
        server = serve(self.wake, farm_report, port=0)
        self.addCleanup(server.server_close)
        self.addCleanup(server.shutdown)
        self.url = f"http://127.0.0.1:{server.server_address[1]}"

    def post(self, path):
        request = urllib.request.Request(self.url + path, data=b"", method="POST")
        with urllib.request.urlopen(request, timeout=10) as response:
            return response.url, response.read().decode()

    def test_the_page_offers_both_buttons_in_post(self):
        self.start()
        with urllib.request.urlopen(f"{self.url}/actions", timeout=10) as response:
            page = response.read().decode()
        self.assertIn('<form method="post" action="/actions/discord">', page)
        self.assertIn('<form method="post" action="/actions/farm-report">', page)
        self.assertFalse(self.wake.is_set())

    def test_the_discord_button_wakes_the_publication(self):
        self.start()
        url, page = self.post("/actions/discord")
        self.assertTrue(self.wake.is_set())
        self.assertTrue(url.endswith("/actions?done=discord"))
        self.assertIn('class="discord running"', page)
        self.assertIn("Publication en cours", page)

    def test_the_farm_report_button_asks_the_farm_report(self):
        self.start("http://farm-report:8000/refresh")
        with mock.patch("discord_post.ask_farm_report") as ask:
            _, page = self.post("/actions/farm-report")
        ask.assert_called_once_with("http://farm-report:8000/refresh")
        self.assertFalse(self.wake.is_set())
        self.assertIn('class="farm-report running"', page)
        self.assertIn("Publication en cours", page)

    def test_a_farm_report_that_does_not_answer_says_so(self):
        self.start()
        with contextlib.redirect_stdout(io.StringIO()):
            _, page = self.post("/actions/farm-report")
        self.assertIn('class="farm-report failed"', page)
        self.assertIn("Pas de réponse", page)

    def test_an_unknown_action_is_refused(self):
        self.start()
        with self.assertRaises(urllib.error.HTTPError) as caught:
            self.post("/actions/other")
        caught.exception.close()
        self.assertEqual(404, caught.exception.code)


class MessagesTest(unittest.TestCase):
    def test_every_message_fits_in_discord(self):
        env = {**read_env(ROOT / ".env.example"), "DOMAIN": "example.com", "CADDY_HTTPS_PORT": "50000"}
        for path in sorted((ROOT / "discord-posts").glob("*.md")):
            with self.subTest(path.name):
                embed = render(path, env)
                self.assertTrue(embed.get("title"))
                self.assertLessEqual(len(embed["description"]), MAX_LENGTH)

    def test_joins_the_lines_wrapped_in_the_file(self):
        text = "# Title\n\nA long\nparagraph.\n-# A long\nsubtext.\n\n- An item\n  wrapped\n- Another\n\n```\nkept\nas is\n```"
        joined = "# Title\n\nA long paragraph.\n-# A long subtext.\n\n- An item wrapped\n- Another\n\n```\nkept\nas is\n```"
        self.assertEqual(joined, unwrap(text))

    def test_reads_values_and_strips_quotes(self):
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / ".env"
            path.write_text("# comment\nDOMAIN='example.com'\nPORT=\"443\"\nEMPTY=\n", encoding="utf-8")
            self.assertEqual({"DOMAIN": "example.com", "PORT": "443", "EMPTY": ""}, read_env(path))
