import re
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
DOCO_CD = ROOT / "services" / "doco-cd" / "compose.yaml"
REPO = "https://github.com/Amaroke/Jellynas.git"


def poll_targets():
    lines = DOCO_CD.read_text(encoding="utf-8").splitlines()
    start = next(i for i, line in enumerate(lines) if re.match(r"^\s+POLL_CONFIG: \|$", line))
    indent = len(lines[start]) - len(lines[start].lstrip()) + 2
    targets = []
    for line in lines[start + 1:]:
        if line.strip() and len(line) - len(line.lstrip()) < indent:
            break
        entry = re.match(r"^\s*(- )?([a-z_]+): (.+)$", line)
        if not entry:
            continue
        if entry.group(1):
            targets.append({})
        targets[-1][entry.group(2)] = entry.group(3).strip()
    return targets


class DocoCdTest(unittest.TestCase):
    def test_polls_main_of_the_repo_every_minute(self):
        main = [t for t in poll_targets() if t.get("url") == REPO and t.get("reference") == "main"]
        self.assertEqual(len(main), 1, poll_targets())
        self.assertEqual(main[0].get("interval"), "1m")


if __name__ == "__main__":
    unittest.main()
