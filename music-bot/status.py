import http.server
import json
import threading
import time

STALE_SECONDS = 90


def summary(player, playing, paused, channel):
    if playing:
        state = "Playing"
    elif paused:
        state = "Paused"
    else:
        state = "Idle"
    return {
        "state": state,
        "track": player.current.label if player.current and (playing or paused) else "Nothing",
        "queue": len(player.queue),
        "shuffle": "On" if player.shuffle else "Off",
        "channel": channel or "None",
    }


class Status:
    def __init__(self, clock=time.monotonic):
        self.clock = clock
        self.beat = None
        self.snapshot = {"state": "Idle", "track": "Nothing", "queue": 0, "shuffle": "Off", "channel": "None"}

    def update(self, connected, snapshot):
        self.snapshot = snapshot
        if connected:
            self.beat = self.clock()

    def healthy(self):
        return self.beat is not None and self.clock() - self.beat < STALE_SECONDS


def serve(status, host, port):
    class Handler(http.server.BaseHTTPRequestHandler):
        def log_message(self, *args):
            pass

        def reply(self, code, body):
            data = json.dumps(body).encode()
            self.send_response(code)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)

        def do_GET(self):
            if self.path == "/health":
                healthy = status.healthy()
                self.reply(200 if healthy else 503, {"healthy": healthy})
            elif self.path == "/status":
                self.reply(200, status.snapshot)
            else:
                self.reply(404, {"error": "not found"})

    server = http.server.ThreadingHTTPServer((host, port), Handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return server
