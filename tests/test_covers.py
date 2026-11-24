import unittest

from covers import (
    HEIGHT,
    HOLD_MS,
    POSTER_WIDTH,
    SLIDE_FRAMES,
    STEP,
    WIDTH,
    css,
    image_query,
    items_query,
    libraries,
    stale,
    timeline,
    window,
)


class TimelineTest(unittest.TestCase):
    def test_each_poster_holds_then_slides_to_the_next(self):
        frames = timeline(3)
        self.assertEqual(3 * SLIDE_FRAMES, len(frames))
        self.assertEqual((0, HOLD_MS), frames[0])
        self.assertEqual((STEP, HOLD_MS), frames[SLIDE_FRAMES])

    def test_the_offsets_only_move_forward_and_stop_short_of_the_loop(self):
        offsets = [offset for offset, _ in timeline(4)]
        self.assertEqual(sorted(offsets), offsets)
        self.assertLess(offsets[-1], 4 * STEP)

    def test_the_held_poster_sits_in_the_middle(self):
        left, right = window(0)
        self.assertEqual(WIDTH, right - left)
        self.assertEqual((WIDTH - POSTER_WIDTH) // 2, STEP - left)
        self.assertGreater(HEIGHT, 0)


class LibrariesTest(unittest.TestCase):
    def test_only_libraries_with_posters_get_a_cover(self):
        folders = [
            {"ItemId": "a", "CollectionType": "movies"},
            {"ItemId": "b", "CollectionType": "music"},
            {"ItemId": "c", "CollectionType": "books"},
            {"ItemId": "d"},
        ]
        self.assertEqual(["a", "c"], libraries(folders))

    def test_the_posters_are_picked_at_random_inside_the_library(self):
        query = items_query("abc")
        self.assertIn("ParentId=abc", query)
        self.assertIn("SortBy=Random", query)
        self.assertIn("ImageTypes=Primary", query)
        self.assertTrue(image_query("x").startswith("/Items/x/Images/Primary?"))


class CssTest(unittest.TestCase):
    def test_each_library_card_shows_its_cover_with_a_version(self):
        text = css({"b": 2, "a": 1})
        self.assertLess(text.index('data-id="a"'), text.index('data-id="b"'))
        self.assertIn('url("/covers/a.webp?v=1") !important', text)

    def test_no_library_gives_an_empty_sheet(self):
        self.assertEqual("", css({}))

    def test_covers_of_gone_libraries_are_removed(self):
        files = ["a.webp", "b.webp", "covers.css", "a.webp.tmp"]
        self.assertEqual(["b.webp"], stale(files, {"a": 1}))


if __name__ == "__main__":
    unittest.main()
