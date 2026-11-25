import tempfile
import unittest
import zipfile
from pathlib import Path

from shelver import shelve_books, shelve_manga

CONTAINER = """<?xml version="1.0"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles>
    <rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/>
  </rootfiles>
</container>
"""

PACKAGE = """<?xml version="1.0" encoding="utf-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="2.0">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>Dune</dc:title>
{extra}  </metadata>
</package>
"""

SIDECAR = """<?xml version='1.0' encoding='utf-8'?>
<package version="2.0" xmlns="http://www.idpf.org/2007/opf">
    <metadata xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:opf="http://www.idpf.org/2007/opf">
        <dc:title>Dune</dc:title>
{extra}    </metadata>
</package>
"""

SERIES = (
    '        <meta content="Dune" name="calibre:series"/>\n'
    '        <meta content="1" name="calibre:series_index"/>\n'
)


def epub(path, extra=""):
    path.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(path, "w") as book:
        book.writestr(zipfile.ZipInfo("mimetype"), "application/epub+zip")
        book.writestr("META-INF/container.xml", CONTAINER)
        book.writestr("OEBPS/content.opf", PACKAGE.format(extra=extra), zipfile.ZIP_DEFLATED)
        book.writestr("OEBPS/text.xhtml", "<html/>", zipfile.ZIP_DEFLATED)


def package(path):
    with zipfile.ZipFile(path) as book:
        return book.read("OEBPS/content.opf").decode()


def cbz(path, comic_info=None):
    path.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(path, "w") as volume:
        volume.writestr("001.jpg", b"page")
        if comic_info is not None:
            volume.writestr("ComicInfo.xml", comic_info)


class ShelveBooksTest(unittest.TestCase):
    def setUp(self):
        self.root = Path(tempfile.mkdtemp())

    def test_an_epub_without_series_gets_the_series_of_its_opf(self):
        book = self.root / "Dune" / "Dune (Dune, #01).epub"
        epub(book)
        (book.parent / "Dune (Dune, #01).opf").write_text(SIDECAR.format(extra=SERIES), encoding="utf-8")
        self.assertEqual([book], shelve_books(self.root))
        inside = package(book)
        self.assertIn('<meta name="calibre:series" content="Dune"/>', inside)
        self.assertIn('<meta name="calibre:series_index" content="1"/>', inside)
        self.assertIn('<meta name="calibre:title_sort" content="Dune 01"/>', inside)
        self.assertIn("<dc:title>Dune</dc:title>", inside)
        with zipfile.ZipFile(book) as done:
            first = done.infolist()[0]
            self.assertEqual("mimetype", first.filename)
            self.assertEqual(zipfile.ZIP_STORED, first.compress_type)
            self.assertEqual(b"<html/>", done.read("OEBPS/text.xhtml"))
        self.assertTrue((book.parent / "Dune (Dune, #01).opf").exists())

    def test_an_epub_that_holds_its_series_is_left_alone(self):
        book = self.root / "Dune" / "Dune (Dune, #01).epub"
        epub(book, extra='    <meta name="calibre:series" content="Dune"/>\n')
        (book.parent / "Dune (Dune, #01).opf").write_text(SIDECAR.format(extra=SERIES), encoding="utf-8")
        before = book.read_bytes()
        self.assertEqual([], shelve_books(self.root))
        self.assertEqual(before, book.read_bytes())

    def test_an_epub_without_series_anywhere_is_left_alone(self):
        book = self.root / "L'Étranger" / "L'Étranger.epub"
        epub(book)
        (book.parent / "L'Étranger.opf").write_text(SIDECAR.format(extra=""), encoding="utf-8")
        self.assertEqual([], shelve_books(self.root))

    def test_an_epub_without_opf_beside_it_is_left_alone(self):
        epub(self.root / "Dune" / "Dune.epub")
        self.assertEqual([], shelve_books(self.root))

    def test_the_recycle_bin_is_skipped(self):
        book = self.root / ".recycle" / "Dune" / "Dune.epub"
        epub(book)
        (book.parent / "Dune.opf").write_text(SIDECAR.format(extra=SERIES), encoding="utf-8")
        self.assertEqual([], shelve_books(self.root))

    def test_a_decimal_index_keeps_its_decimals_in_the_sort(self):
        book = self.root / "Dune" / "Dune.epub"
        epub(book)
        extra = SERIES.replace('content="1"', 'content="1.5"')
        (book.parent / "Dune.opf").write_text(SIDECAR.format(extra=extra), encoding="utf-8")
        shelve_books(self.root)
        self.assertIn('content="Dune 01.5"', package(book))


class ShelveMangaTest(unittest.TestCase):
    def setUp(self):
        self.root = Path(tempfile.mkdtemp())

    def test_a_volume_without_comic_info_is_filed_with_one(self):
        volume = self.root / "Naruto" / "Naruto 3.cbz"
        cbz(volume)
        filed = self.root / "Naruto" / "Tome 3 (Naruto, #03).cbz"
        self.assertEqual([filed], shelve_manga(self.root))
        self.assertFalse(volume.exists())
        with zipfile.ZipFile(filed) as done:
            info = done.read("ComicInfo.xml").decode()
            self.assertEqual(b"page", done.read("001.jpg"))
        for tag in (
            "<Title>Tome 3</Title>",
            "<Series>Naruto</Series>",
            "<Number>3</Number>",
            "<Volume>3</Volume>",
            "<LanguageISO>fr</LanguageISO>",
            "<Manga>Yes</Manga>",
        ):
            self.assertIn(tag, info)

    def test_a_series_name_is_escaped_in_comic_info(self):
        volume = self.root / "Tom & Jerry" / "Tom & Jerry 12.cbz"
        cbz(volume)
        shelve_manga(self.root)
        with zipfile.ZipFile(self.root / "Tom & Jerry" / "Tome 12 (Tom & Jerry, #12).cbz") as done:
            self.assertIn("<Series>Tom &amp; Jerry</Series>", done.read("ComicInfo.xml").decode())

    def test_a_volume_with_comic_info_is_left_alone(self):
        volume = self.root / "Naruto" / "Celui qui sait (Naruto, #43).cbz"
        cbz(volume, "<ComicInfo><Series>Naruto</Series></ComicInfo>")
        before = volume.read_bytes()
        self.assertEqual([], shelve_manga(self.root))
        self.assertEqual(before, volume.read_bytes())

    def test_a_name_without_a_number_is_left_alone(self):
        volume = self.root / "Naruto" / "Naruto.cbz"
        cbz(volume)
        self.assertEqual([], shelve_manga(self.root))
        self.assertTrue(volume.exists())

    def test_a_volume_is_never_filed_over_another(self):
        cbz(self.root / "Naruto" / "Tome 3 (Naruto, #03).cbz", "<ComicInfo/>")
        volume = self.root / "Naruto" / "Naruto 3.cbz"
        cbz(volume)
        self.assertEqual([], shelve_manga(self.root))
        self.assertTrue(volume.exists())


if __name__ == "__main__":
    unittest.main()
