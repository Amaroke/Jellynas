import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

for folder in (ROOT / "scripts", *(ROOT / "services" / name for name in ("music-bot", "pr-radar"))):
    sys.path.insert(0, str(folder))
