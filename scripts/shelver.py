import os
import re
import sys
import time
import xml.etree.ElementTree as ElementTree
import zipfile
from pathlib import Path
from xml.sax.saxutils import escape, quoteattr

BOOKS = Path("/data/media/books")
MANGA = Path("/data/media/manga")
HELD = re.compile(r'name="calibre:series"|property="belongs-to-collection"')
CLOSE = re.compile(r"</(?:\w+:)?metadata>")
VOLUME = re.compile(r"^(?P<series>.+?)\s+(?P<number>\d+(?:\.\d+)?)$")


def files(root, suffix):
    for path in sorted(root.rglob(f"*{suffix}")):
        relative = path.relative_to(root)
        if not any(part.startswith(".") for part in relative.parts):
            yield path


def rewrite(path, changed, extra=None):
    temporary = path.with_name(f".{path.name}.shelver")
    try:
        with zipfile.ZipFile(path) as source, zipfile.ZipFile(temporary, "w") as target:
            for entry in source.infolist():
                data = source.read(entry)
                if entry.filename in changed:
                    data = changed[entry.filename].encode("utf-8")
                target.writestr(entry, data, compress_type=entry.compress_type)
            if extra:
                name, text = extra
                target.writestr(name, text.encode("utf-8"), compress_type=zipfile.ZIP_DEFLATED)
    except BaseException:
        temporary.unlink(missing_ok=True)
        raise
    return temporary


def package_path(book):
    container = book.read("META-INF/container.xml").decode("utf-8")
    found = re.search(r'full-path="([^"]+)"', container)
    if not found:
        raise ValueError("no package in container.xml")
    return found.group(1)


def sidecar_series(path):
    tree = ElementTree.parse(path)
    values = {}
    for element in tree.iter():
        if element.tag.rsplit("}", 1)[-1] == "meta" and element.get("name"):
            values[element.get("name")] = element.get("content", "").strip()
    series = values.get("calibre:series", "")
    index = values.get("calibre:series_index", "")
    return (series, index) if series and index else None


def sort_key(series, index):
    whole, _, decimals = index.partition(".")
    padded = whole.zfill(2)
    if decimals.strip("0"):
        padded = f"{padded}.{decimals.rstrip('0')}"
    return f"{series} {padded}"


def shelve_book(path):
    sidecar = path.with_suffix(".opf")
    if not sidecar.exists():
        return False
    found = sidecar_series(sidecar)
    if not found:
        return False
    with zipfile.ZipFile(path) as book:
        inside = package_path(book)
        text = book.read(inside).decode("utf-8")
    if HELD.search(text):
        return False
    closing = CLOSE.search(text)
    if not closing:
        raise ValueError("no metadata in the package")
    series, index = found
    lines = "".join(
        f"    <meta name={quoteattr(name)} content={quoteattr(value)}/>\n"
        for name, value in (
            ("calibre:series", series),
            ("calibre:series_index", index),
            ("calibre:title_sort", sort_key(series, index)),
        )
    )
    text = text[: closing.start()] + lines + text[closing.start():]
    os.replace(rewrite(path, {inside: text}), path)
    return True


def comic_info(series, number):
    fields = (
        ("Title", f"Tome {number}"),
        ("Series", series),
        ("Number", number),
        ("Volume", number),
        ("LanguageISO", "fr"),
        ("Manga", "Yes"),
    )
    body = "".join(f"  <{tag}>{escape(value)}</{tag}>\n" for tag, value in fields)
    return f'<?xml version="1.0" encoding="utf-8"?>\n<ComicInfo>\n{body}</ComicInfo>\n'


def shelve_volume(path):
    with zipfile.ZipFile(path) as volume:
        if any(name.lower() == "comicinfo.xml" for name in volume.namelist()):
            return None
    found = VOLUME.match(path.stem)
    if not found:
        return None
    series = path.parent.name
    raw = found.group("number")
    whole, _, decimals = raw.partition(".")
    number = str(int(whole)) + (f".{decimals}" if decimals else "")
    target = path.with_name(f"Tome {number} ({series}, #{whole.zfill(2)}{'.' + decimals if decimals else ''}).cbz")
    if target.exists():
        return None
    temporary = rewrite(path, {}, ("ComicInfo.xml", comic_info(series, number)))
    os.replace(temporary, target)
    path.unlink()
    return target


def shelve_books(root):
    done = []
    for path in files(root, ".epub"):
        try:
            if shelve_book(path):
                done.append(path)
        except (OSError, ValueError, KeyError, zipfile.BadZipFile, ElementTree.ParseError) as error:
            print(f"Cannot shelve {path}: {error}", file=sys.stderr)
    return done


def shelve_manga(root):
    done = []
    for path in files(root, ".cbz"):
        try:
            target = shelve_volume(path)
        except (OSError, ValueError, zipfile.BadZipFile) as error:
            print(f"Cannot shelve {path}: {error}", file=sys.stderr)
            continue
        if target:
            done.append(target)
    return done


def shelve(books=BOOKS, manga=MANGA):
    for path in shelve_books(books):
        print(f"Wrote the series inside {path}")
    for path in shelve_manga(manga):
        print(f"Filed {path}")


def main(arguments):
    every = int(arguments[arguments.index("--every") + 1]) if "--every" in arguments else 0
    while True:
        shelve()
        if not every:
            return
        time.sleep(every)


if __name__ == "__main__":
    main(sys.argv[1:])
