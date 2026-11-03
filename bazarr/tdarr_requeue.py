import json
import os
import sys
import urllib.request

TDARR_URL = os.environ.get("TDARR_URL", "http://tdarr:8265")


def cruddb(data):
    request = urllib.request.Request(
        f"{TDARR_URL}/api/v2/cruddb",
        data=json.dumps({"data": data}).encode(),
        headers={"Content-Type": "application/json", "x-api-key": os.environ["TDARR_API_KEY"]},
    )
    with urllib.request.urlopen(request, timeout=10) as response:
        body = response.read()
    return json.loads(body) if body.strip() else None


def main():
    video = sys.argv[1]
    if not cruddb({"collection": "FileJSONDB", "mode": "getById", "docID": video}):
        print(f"Tdarr has not scanned {video} yet, it embeds the subtitles at its first pass")
        return
    cruddb({"collection": "FileJSONDB", "mode": "update", "docID": video, "obj": {"TranscodeDecisionMaker": "Queued"}})
    print(f"Queued {video} in Tdarr to embed its subtitles")


main()
