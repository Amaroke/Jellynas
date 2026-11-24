import io
import json
import os
import sys
import time
import urllib.parse
import urllib.request
from pathlib import Path

INTERVAL = 24 * 3600
RETRY = 3600
WIDTH, HEIGHT = 480, 270
POSTER_WIDTH, POSTER_HEIGHT, GAP, RADIUS = 160, 240, 12, 10
STEP = POSTER_WIDTH + GAP
POSTERS = 8
MIN_POSTERS = 3
HOLD_MS, SLIDE_FRAMES, SLIDE_MS = 2200, 8, 45
TYPES = "Movie,Series,BoxSet,Book"
COVERED = {"movies", "tvshows", "mixed", "boxsets", "books", "homevideos"}


def settings():
    return {
        "url": os.environ["JELLYFIN_URL"].rstrip("/"),
        "key": os.environ["JELLYFIN_API_KEY"],
        "out": Path(os.environ.get("COVERS_DIR", "/covers")),
    }


def fetch(config, path):
    request = urllib.request.Request(
        config["url"] + path,
        headers={"Authorization": f'MediaBrowser Token="{config["key"]}"'},
    )
    with urllib.request.urlopen(request, timeout=60) as response:
        return response.read()


def libraries(folders):
    return [folder["ItemId"] for folder in folders if folder.get("CollectionType") in COVERED]


def items_query(library):
    return "/Items?" + urllib.parse.urlencode(
        {
            "ParentId": library,
            "Recursive": "true",
            "IncludeItemTypes": TYPES,
            "ImageTypes": "Primary",
            "SortBy": "Random",
            "Limit": POSTERS,
        }
    )


def image_query(item):
    return f"/Items/{item}/Images/Primary?fillHeight={POSTER_HEIGHT * 2}&quality=90"


def timeline(count):
    frames = []
    for index in range(count):
        frames.append((index * STEP, HOLD_MS))
        for frame in range(1, SLIDE_FRAMES):
            t = frame / SLIDE_FRAMES
            frames.append((round((index + t * t * (3 - 2 * t)) * STEP), SLIDE_MS))
    return frames


def window(offset):
    left = offset + STEP - (WIDTH - POSTER_WIDTH) // 2
    return left, left + WIDTH


def css(versions):
    rules = [
        f'.card[data-id="{library}"] .cardImageContainer {{\n'
        f'  background-image: url("/covers/{library}.webp?v={version}") !important;\n'
        "  background-size: cover !important;\n"
        "  background-position: center !important;\n"
        "}\n"
        for library, version in sorted(versions.items())
    ]
    return "".join(rules)


def stale(files, libraries_kept):
    return sorted(name for name in files if name.endswith(".webp") and name[:-5] not in libraries_kept)


def render(posters):
    from PIL import Image, ImageDraw, ImageEnhance, ImageFilter, ImageOps

    cards = [ImageOps.fit(Image.open(io.BytesIO(data)).convert("RGB"), (POSTER_WIDTH, POSTER_HEIGHT)) for data in posters]
    mask = Image.new("L", (POSTER_WIDTH, POSTER_HEIGHT), 0)
    ImageDraw.Draw(mask).rounded_rectangle((0, 0, POSTER_WIDTH - 1, POSTER_HEIGHT - 1), RADIUS, fill=255)
    order = [cards[-1], *cards, cards[0], cards[1]]
    strip = Image.new("RGBA", (len(order) * STEP, HEIGHT), (0, 0, 0, 0))
    top = (HEIGHT - POSTER_HEIGHT) // 2
    for index, card in enumerate(order):
        strip.paste(card, (index * STEP, top), mask)
    background = ImageOps.fit(cards[0], (WIDTH, HEIGHT)).filter(ImageFilter.GaussianBlur(18))
    background = ImageEnhance.Brightness(background).enhance(0.45).convert("RGBA")
    frames, durations = [], []
    for offset, duration in timeline(len(cards)):
        frame = background.copy()
        left, right = window(offset)
        frame.alpha_composite(strip.crop((left, 0, right, HEIGHT)))
        frames.append(frame.convert("RGB"))
        durations.append(duration)
    buffer = io.BytesIO()
    frames[0].save(buffer, "WEBP", save_all=True, append_images=frames[1:], duration=durations, loop=0, quality=70, method=6, minimize_size=True)
    return buffer.getvalue()


def write(path, data):
    temporary = path.with_suffix(path.suffix + ".tmp")
    temporary.write_bytes(data)
    os.replace(temporary, path)


def run(config):
    out = config["out"]
    versions = {}
    for library in libraries(json.loads(fetch(config, "/Library/VirtualFolders"))):
        items = json.loads(fetch(config, items_query(library))).get("Items", [])
        if len(items) < MIN_POSTERS:
            continue
        posters = [fetch(config, image_query(item["Id"])) for item in items]
        write(out / f"{library}.webp", render(posters))
        versions[library] = int(time.time())
        print(f"covers: {library} from {len(posters)} posters", flush=True)
    for name in stale(os.listdir(out), versions):
        (out / name).unlink()
    write(out / "covers.css", css(versions).encode())
    return len(versions)


def main():
    config = settings()
    while True:
        try:
            print(f"covers: {run(config)} libraries", flush=True)
            time.sleep(INTERVAL)
        except (OSError, ValueError, KeyError) as error:
            print(f"covers: {error}", file=sys.stderr, flush=True)
            time.sleep(RETRY)


if __name__ == "__main__":
    main()
