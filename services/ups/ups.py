import base64
import ctypes
import fcntl
import glob
import http.client
import http.server
import json
import os
import ssl
import struct
import sys
import threading
import time
import urllib.request

DISCORD = "https://discord.com/api/v10/webhooks"
USER_AGENT = "DiscordBot (https://github.com/Amaroke/Jellynas, 1.0)"
INTERVAL = 5
STALE = 30
UNKNOWN_RUNTIME = 0xFFFF
FAILURES = (OSError, ValueError, KeyError, TypeError, http.client.HTTPException)


class Control(ctypes.Structure):
    _fields_ = [
        ("request_type", ctypes.c_uint8),
        ("request", ctypes.c_uint8),
        ("value", ctypes.c_uint16),
        ("index", ctypes.c_uint16),
        ("length", ctypes.c_uint16),
        ("timeout", ctypes.c_uint32),
        ("data", ctypes.c_void_p),
    ]


USBDEVFS_CONTROL = (3 << 30) | (ctypes.sizeof(Control) << 16) | (ord("U") << 8)


def find(vendor, sysfs="/sys/bus/usb/devices", dev="/dev/bus/usb"):
    for device in sorted(glob.glob(f"{sysfs}/*")):
        try:
            with open(f"{device}/idVendor") as found:
                if found.read().strip().lower() != vendor.lower():
                    continue
            with open(f"{device}/busnum") as bus, open(f"{device}/devnum") as number:
                return f"{dev}/{int(bus.read()):03d}/{int(number.read()):03d}"
        except (OSError, ValueError):
            continue
    return None


def feature(handle, report, length=16):
    buffer = ctypes.create_string_buffer(length)
    control = Control(0xA1, 0x01, (3 << 8) | report, 0, length, 1000, ctypes.cast(buffer, ctypes.c_void_p))
    got = fcntl.ioctl(handle, USBDEVFS_CONTROL, control)
    return buffer.raw[:got]


def parse(status, capacity, load):
    if len(status) < 4 or status[0] != 0x01 or len(capacity) < 6 or capacity[0] != 0x06:
        raise ValueError("unexpected reports from the UPS")
    flags = status[1]
    runtime = struct.unpack_from("<I", capacity, 2)[0]
    return {
        "mains": bool(flags & 0x01),
        "low": bool(flags & 0x02),
        "failure": bool(flags & 0x40),
        "replace": bool(flags & 0x80),
        "overload": bool(status[2]),
        "imminent": bool(status[3]),
        "charge": capacity[1],
        "runtime": None if runtime >= UNKNOWN_RUNTIME else runtime,
        "load": load[6] if len(load) > 6 and load[0] == 0x07 else None,
    }


def state(reading):
    if reading is None:
        return "unreachable"
    if reading["mains"]:
        return "on mains"
    if reading["low"] or reading["imminent"]:
        return "low battery"
    return "on battery"


def read(vendor):
    path = find(vendor)
    if not path:
        raise OSError(f"no USB device of vendor {vendor}")
    handle = os.open(path, os.O_RDWR)
    try:
        return parse(feature(handle, 0x01), feature(handle, 0x06), feature(handle, 0x07))
    finally:
        os.close(handle)


def request(url, payload):
    body = json.dumps(payload).encode()
    headers = {"Content-Type": "application/json", "User-Agent": USER_AGENT}
    with urllib.request.urlopen(urllib.request.Request(url, data=body, headers=headers), timeout=30) as response:
        response.read()


def alert(hook, text, color):
    if hook:
        request(f"{DISCORD}/{hook}", {"embeds": [{"description": text, "color": color}], "allowed_mentions": {"parse": []}})


class Socket:
    def __init__(self, host, port=443, path="/api/current"):
        context = ssl.create_default_context()
        context.check_hostname = False
        context.verify_mode = ssl.CERT_NONE
        self.connection = http.client.HTTPSConnection(host, port, timeout=30, context=context)
        key = base64.b64encode(os.urandom(16)).decode()
        self.connection.putrequest("GET", path)
        for name, value in (("Upgrade", "websocket"), ("Connection", "Upgrade"), ("Sec-WebSocket-Key", key), ("Sec-WebSocket-Version", "13")):
            self.connection.putheader(name, value)
        self.connection.endheaders()
        response = self.connection.getresponse()
        if response.status != 101:
            raise OSError(f"TrueNAS answered HTTP {response.status} to the websocket")
        self.stream = self.connection.sock
        self.calls = 0

    def send(self, text):
        data = text.encode()
        mask = os.urandom(4)
        size = len(data)
        head = bytes([0x81])
        if size < 126:
            head += bytes([0x80 | size])
        elif size < 65536:
            head += bytes([0x80 | 126]) + struct.pack(">H", size)
        else:
            head += bytes([0x80 | 127]) + struct.pack(">Q", size)
        self.stream.sendall(head + mask + bytes(byte ^ mask[index % 4] for index, byte in enumerate(data)))

    def exact(self, size):
        data = b""
        while len(data) < size:
            chunk = self.stream.recv(size - len(data))
            if not chunk:
                raise OSError("TrueNAS closed the websocket")
            data += chunk
        return data

    def receive(self):
        message = b""
        while True:
            first, second = self.exact(2)
            size = second & 0x7F
            if size == 126:
                size = struct.unpack(">H", self.exact(2))[0]
            elif size == 127:
                size = struct.unpack(">Q", self.exact(8))[0]
            mask = self.exact(4) if second & 0x80 else None
            data = self.exact(size)
            if mask:
                data = bytes(byte ^ mask[index % 4] for index, byte in enumerate(data))
            opcode = first & 0x0F
            if opcode == 0x8:
                raise OSError("TrueNAS closed the websocket")
            if opcode == 0x9:
                continue
            message += data
            if first & 0x80:
                return message.decode()

    def call(self, method, params):
        self.calls += 1
        self.send(json.dumps({"jsonrpc": "2.0", "id": self.calls, "method": method, "params": params}))
        while True:
            answer = json.loads(self.receive())
            if answer.get("id") == self.calls:
                if "error" in answer:
                    raise OSError(f"{method} failed: {answer['error']}")
                return answer.get("result")

    def close(self):
        self.connection.close()


def shutdown(host, key, reason):
    socket = Socket(host)
    try:
        if socket.call("auth.login_with_api_key", [key]) is not True:
            raise OSError("TrueNAS refused the API key")
        socket.call("system.shutdown", [reason])
    finally:
        socket.close()


class Watch:
    def __init__(self, threshold, clock=time.time):
        self.threshold = threshold
        self.clock = clock
        self.reading = None
        self.read_at = None
        self.since = None
        self.on_battery = 0
        self.stopping = False
        self.error = None

    def update(self, reading):
        events = []
        before = state(self.reading) if self.read_at else None
        self.reading, self.read_at, self.error = reading, self.clock(), None
        after = state(reading)
        if after != before:
            self.since = self.read_at
            if before is not None and before != "unreachable" or after != "on mains":
                events.append(after)
        self.on_battery = 0 if reading["mains"] else self.on_battery + 1
        low = reading["low"] or reading["imminent"] or reading["charge"] <= self.threshold
        if self.on_battery >= 2 and low and not self.stopping:
            self.stopping = True
            events.append("shutdown")
        return events

    def failed(self, error):
        self.error = str(error)

    def status(self):
        fresh = self.read_at is not None and self.clock() - self.read_at <= STALE
        reading = self.reading if fresh else None
        return {
            "state": state(reading),
            "since": self.since,
            "threshold": self.threshold,
            "stopping": self.stopping,
            "error": self.error,
            **(reading or {}),
        }

    def healthy(self):
        return self.status()["state"] != "unreachable"


MESSAGES = {
    "on mains": ("Le secteur est revenu, l'onduleur recharge ({charge} %).", 0x3FB950),
    "on battery": ("Coupure de courant, le NAS tourne sur l'onduleur ({charge} %).", 0xD29922),
    "low battery": ("Batterie de l'onduleur faible ({charge} %).", 0xF85149),
    "shutdown": ("Batterie à {charge} %, TrueNAS s'éteint proprement.", 0xF85149),
}


def serve(watch, port=8000):
    class Handler(http.server.BaseHTTPRequestHandler):
        def do_GET(self):
            if self.path == "/status":
                self.answer(200, json.dumps(watch.status()).encode(), "application/json")
            elif self.path == "/health":
                ok = watch.healthy()
                self.answer(200 if ok else 503, b"ok" if ok else b"the UPS cannot be read", "text/plain")
            else:
                self.send_error(404)

        def answer(self, code, body, kind):
            self.send_response(code)
            self.send_header("Content-Type", kind)
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def log_message(self, *args):
            pass

    server = http.server.ThreadingHTTPServer(("0.0.0.0", port), Handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return server


def settings(env):
    key = env.get("TRUENAS_API_KEY", "").strip()
    if not key:
        sys.exit("TRUENAS_API_KEY must be set in .env")
    hook = "/".join(env.get(f"DISCORD_ALERTS_WEBHOOK_{part}", "").strip() for part in ("ID", "TOKEN"))
    return {
        "vendor": env.get("UPS_VENDOR_ID", "2b89").strip(),
        "threshold": int(env.get("UPS_SHUTDOWN_CHARGE", "") or 20),
        "host": env.get("TRUENAS_HOST", "host.docker.internal").strip(),
        "key": key,
        "hook": "" if hook.startswith("/") or hook.endswith("/") else hook,
    }


def main():
    config = settings(os.environ)
    watch = Watch(config["threshold"])
    serve(watch)
    print(f"Watching the UPS of vendor {config['vendor']}, shutdown at {config['threshold']} %", flush=True)
    stopped = False
    while True:
        try:
            events = watch.update(read(config["vendor"]))
        except FAILURES as error:
            if watch.error != str(error):
                print(f"Cannot read the UPS: {error}", flush=True)
            watch.failed(error)
            events = []
        for event in events:
            text, color = MESSAGES[event]
            text = text.format(**watch.reading)
            print(text, flush=True)
            try:
                alert(config["hook"], text, color)
            except FAILURES as error:
                print(f"Cannot alert Discord: {error}", flush=True)
        if watch.stopping and not stopped:
            try:
                shutdown(config["host"], config["key"], "UPS battery low")
                stopped = True
            except FAILURES as error:
                print(f"Cannot shut TrueNAS down: {error}", flush=True)
        time.sleep(INTERVAL)


if __name__ == "__main__":
    main()
