import http.server
import json
import re
import tempfile
import threading
import unittest
from pathlib import Path

from discord_post import MAX_LENGTH, publish, read_env, render, unwrap

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
        return publish("links", self.message, self.env, self.discord.url)

    def test_posts_the_message_filled_from_the_env(self):
        self.publish()
        method, path, body = self.discord.calls[0]
        self.assertEqual(("POST", "/1234/abcd?wait=true"), (method, path))
        self.assertEqual("", body["content"])
        self.assertEqual([{"description": "https://jellyfin.example.com:443", "color": 0x60A5FA}], body["embeds"])
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

    def test_admin_lists_every_lan_name_of_caddy(self):
        caddyfile = (ROOT / "services" / "caddy" / "Caddyfile").read_text(encoding="utf-8")
        served = set(re.findall(r"^\timport lan ([a-z-]+) ", caddyfile, re.MULTILINE))
        served |= set(re.findall(r"^\t\thost ([a-z-]+)\.lan\.", caddyfile, re.MULTILINE))
        listed = set(re.findall(r"https://([a-z-]+)\.lan\.", (ROOT / "discord-posts" / "admin.md").read_text(encoding="utf-8")))
        self.assertEqual(served, listed)
