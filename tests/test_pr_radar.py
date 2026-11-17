import datetime
import json
import tempfile
import time
import unittest
import urllib.error
import urllib.request
from pathlib import Path

from radar import (
    INTERVAL,
    Radar,
    age,
    embed,
    events,
    poll,
    pulls_page,
    queries,
    readiness,
    serve,
    settings,
    stats,
    summarize,
)


def pull(number, state="OPEN", ci="SUCCESS", sha="a1", author="renovate", draft=False):
    return {
        "repo": "Amaroke/Jellynas",
        "number": number,
        "title": f"Update thing {number}",
        "url": f"https://github.com/Amaroke/Jellynas/pull/{number}",
        "author": author,
        "state": state,
        "draft": draft,
        "sha": sha,
        "ci": ci,
    }


def keyed(*pulls):
    return {f"{item['repo']}#{item['number']}": item for item in pulls}


class SummaryTest(unittest.TestCase):
    def test_a_node_of_the_search_becomes_a_pull(self):
        node = {
            "number": 7,
            "title": "Fix",
            "url": "https://github.com/o/r/pull/7",
            "isDraft": True,
            "state": "OPEN",
            "headRefOid": "abc",
            "author": {"login": "renovate"},
            "repository": {"nameWithOwner": "o/r"},
            "commits": {"nodes": [{"commit": {"statusCheckRollup": {"state": "FAILURE"}}}]},
            "createdAt": "2026-11-17T20:00:00Z",
            "additions": 3,
            "deletions": 1,
            "changedFiles": 2,
            "mergeStateStatus": "BLOCKED",
            "reviewDecision": "REVIEW_REQUIRED",
            "labels": {"nodes": [{"name": "major"}]},
        }
        self.assertEqual(
            {"repo": "o/r", "number": 7, "title": "Fix", "url": "https://github.com/o/r/pull/7", "author": "renovate",
             "state": "OPEN", "draft": True, "sha": "abc", "ci": "FAILURE", "created": "2026-11-17T20:00:00Z",
             "additions": 3, "deletions": 1, "files": 2, "merge": "BLOCKED", "review": "REVIEW_REQUIRED",
             "labels": ["major"]},
            summarize(node),
        )

    def test_a_pull_without_checks_or_author_still_reads(self):
        node = {"number": 1, "title": "t", "url": "u", "state": "MERGED", "author": None,
                "repository": {"nameWithOwner": "o/r"}, "commits": {"nodes": [{"commit": {"statusCheckRollup": None}}]}}
        summary = summarize(node)
        self.assertEqual(("ghost", None), (summary["author"], summary["ci"]))

    def test_the_searches_cover_own_repos_and_pulls_sent_elsewhere(self):
        found = queries("me", datetime.datetime(2026, 11, 15, tzinfo=datetime.UTC))
        self.assertIn("is:pr is:open archived:false user:me", found)
        self.assertIn("is:pr is:open archived:false author:me -user:me", found)
        self.assertIn("is:pr is:closed archived:false user:me closed:>=2026-11-15", found)


class EventTest(unittest.TestCase):
    def test_a_new_pull_is_announced(self):
        self.assertEqual([("opened", pull(1))], events({}, keyed(pull(1))))

    def test_a_known_pull_says_nothing_while_nothing_changes(self):
        self.assertEqual([], events(keyed(pull(1)), keyed(pull(1))))

    def test_a_ci_that_turns_red_is_announced_once(self):
        red = pull(1, ci="FAILURE")
        self.assertEqual([("failed", red)], events(keyed(pull(1, ci="PENDING")), keyed(red)))
        self.assertEqual([], events(keyed(red), keyed(red)))

    def test_a_new_commit_that_fails_again_is_announced(self):
        self.assertEqual(1, len(events(keyed(pull(1, ci="ERROR")), keyed(pull(1, ci="FAILURE", sha="b2")))))

    def test_a_merge_and_a_close_are_announced_once(self):
        merged, closed = pull(1, state="MERGED"), pull(2, state="CLOSED")
        self.assertEqual([("merged", merged), ("closed", closed)], events(keyed(pull(1), pull(2)), keyed(merged, closed)))
        self.assertEqual([], events(keyed(merged, closed), keyed(merged, closed)))

    def test_a_pull_merged_between_two_polls_is_still_announced(self):
        self.assertEqual([("merged", pull(3, state="MERGED"))], events({}, keyed(pull(3, state="MERGED"))))


class EmbedTest(unittest.TestCase):
    def test_a_new_pull_names_its_repo_author_and_ci(self):
        message = embed("opened", pull(5, ci="PENDING", draft=True))
        self.assertEqual("Amaroke/Jellynas · nouvelle PR", message["author"]["name"])
        self.assertEqual("#5 Update thing 5", message["title"])
        self.assertEqual("https://github.com/Amaroke/Jellynas/pull/5", message["url"])
        self.assertEqual(["Auteur", "CI", "Brouillon"], [field["name"] for field in message["fields"]])
        self.assertEqual("en cours", message["fields"][1]["value"])

    def test_each_kind_has_its_own_color(self):
        colors = {embed(kind, pull(1))["color"] for kind in ("opened", "failed", "merged", "closed")}
        self.assertEqual(4, len(colors))

    def test_a_long_title_fits_discord(self):
        long = {**pull(1), "title": "x" * 400}
        self.assertEqual(256, len(embed("opened", long)["title"]))


class PollTest(unittest.TestCase):
    def setUp(self):
        self.folder = tempfile.TemporaryDirectory()
        self.state = Path(self.folder.name) / "pr-radar" / "state.json"
        self.sent = []

    def tearDown(self):
        self.folder.cleanup()

    def test_the_first_poll_stays_silent_and_remembers(self):
        self.assertEqual([], poll(self.state, lambda: keyed(pull(1)), self.sent.append))
        self.assertEqual([], self.sent)
        self.assertEqual(keyed(pull(1)), json.loads(self.state.read_text(encoding="utf-8")))

    def test_later_polls_send_what_changed_in_one_batch(self):
        poll(self.state, lambda: keyed(pull(1)), self.sent.append)
        poll(self.state, lambda: keyed(pull(1, state="MERGED"), pull(2)), self.sent.append)
        self.assertEqual(1, len(self.sent))
        self.assertEqual(["mergée", "nouvelle PR"], [item["author"]["name"].split(" · ")[1] for item in self.sent[0]])

    def test_a_failed_send_keeps_the_old_state_to_retry(self):
        poll(self.state, lambda: keyed(pull(1)), self.sent.append)

        def broken(embeds):
            raise OSError("Discord is down")

        with self.assertRaises(OSError):
            poll(self.state, lambda: keyed(pull(2)), broken)
        self.assertEqual(keyed(pull(1)), json.loads(self.state.read_text(encoding="utf-8")))


class StatusTest(unittest.TestCase):
    def test_the_counts_cover_open_pulls_only(self):
        pulls = keyed(pull(1), pull(2, ci="FAILURE", author="Amaroke"), pull(3, state="MERGED"))
        self.assertEqual({"open": 2, "bots": 1, "failing": 1}, stats(pulls))

    def test_the_server_gives_the_counts_and_the_health(self):
        with tempfile.TemporaryDirectory() as folder:
            radar = Radar(Path(folder) / "state.json")
            server = serve(radar, port=0)
            base = f"http://127.0.0.1:{server.server_address[1]}"
            try:
                with urllib.request.urlopen(f"{base}/status") as response:
                    self.assertEqual({"open": 0, "bots": 0, "failing": 0}, json.load(response))
                with self.assertRaises(urllib.error.HTTPError) as caught:
                    urllib.request.urlopen(f"{base}/health")
                caught.exception.close()
                self.assertEqual(503, caught.exception.code)
                radar.run_once(lambda: keyed(pull(1)), lambda embeds: None)
                with urllib.request.urlopen(f"{base}/health") as response:
                    self.assertEqual(200, response.status)
                with urllib.request.urlopen(f"{base}/status") as response:
                    self.assertEqual(1, json.load(response)["open"])
                with urllib.request.urlopen(f"{base}/pulls") as response:
                    self.assertIn("default-src 'none'", response.headers["Content-Security-Policy"])
                    self.assertIn("1 ouverte(s)", response.read().decode())
            finally:
                server.shutdown()
                server.server_close()

    def test_health_fails_after_three_missed_polls(self):
        radar = Radar(Path("unused"))
        radar.last_success = time.time() - 3 * INTERVAL - 1
        self.assertFalse(radar.healthy())


class PageTest(unittest.TestCase):
    now = datetime.datetime(2026, 11, 17, 21, 0, tzinfo=datetime.UTC)

    def test_a_pull_is_ready_once_its_ci_is_green_and_github_allows_the_merge(self):
        self.assertEqual("ok", readiness({**pull(1), "merge": "CLEAN"}))
        self.assertEqual("ok", readiness({**pull(1), "merge": "UNSTABLE"}))
        self.assertEqual("wait", readiness({**pull(1, ci="PENDING"), "merge": "BLOCKED"}))
        self.assertEqual("wait", readiness({**pull(1), "merge": "BEHIND"}))
        self.assertEqual("bad", readiness({**pull(1, ci="FAILURE"), "merge": "CLEAN"}))
        self.assertEqual("bad", readiness({**pull(1), "merge": "DIRTY"}))
        self.assertEqual("muted", readiness({**pull(1, draft=True), "merge": "DRAFT"}))

    def test_the_age_reads_in_the_largest_unit(self):
        self.assertEqual("2 j", age("2026-11-15T20:00:00Z", self.now))
        self.assertEqual("3 h", age("2026-11-17T18:00:00Z", self.now))
        self.assertEqual("5 min", age("2026-11-17T20:55:00Z", self.now))
        self.assertEqual("", age("", self.now))

    def test_the_page_lists_open_pulls_ready_ones_first_with_their_links(self):
        pulls = keyed(
            {**pull(1, ci="FAILURE"), "merge": "BLOCKED"},
            {**pull(2), "merge": "CLEAN", "title": "<b>ready</b>", "labels": ["major"]},
            pull(3, state="MERGED"),
        )
        page = pulls_page(pulls, self.now)
        self.assertLess(page.index("#2</span>"), page.index("#1</span>"))
        self.assertNotIn("#3</span>", page)
        self.assertIn("&lt;b&gt;ready&lt;/b&gt;", page)
        self.assertIn('href="https://github.com/Amaroke/Jellynas/pull/2/files"', page)
        self.assertIn('href="https://github.com/Amaroke/Jellynas/pull/2/checks"', page)
        self.assertIn("prête à merger", page)
        self.assertIn("major", page)
        self.assertIn("2 ouverte(s)", page)
        self.assertIn("1 prête(s) · 0 en attente · 1 à corriger", page)

    def test_an_empty_radar_says_so(self):
        self.assertIn("Aucune pull request ouverte", pulls_page({}, self.now))


class SettingsTest(unittest.TestCase):
    def test_the_token_and_both_parts_of_the_webhook_are_required(self):
        full = {"PR_RADAR_GITHUB_TOKEN": "t", "DISCORD_PR_WEBHOOK_ID": "1", "DISCORD_PR_WEBHOOK_TOKEN": "x"}
        self.assertEqual(("t", "1/x"), settings(full))
        for missing in full:
            with self.assertRaises(SystemExit):
                settings({**full, missing: ""})


if __name__ == "__main__":
    unittest.main()
