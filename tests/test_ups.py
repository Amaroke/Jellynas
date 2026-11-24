import json
import tempfile
import unittest
import urllib.error
import urllib.request
from pathlib import Path

from ups import Watch, find, parse, serve, settings, state

MAINS = bytes.fromhex("01310000")
BATTERY = bytes.fromhex("01300000")
LOW = bytes.fromhex("01320000")


def capacity(charge, runtime=0xFFFF):
    return bytes([0x06, charge]) + runtime.to_bytes(4, "little")


LOAD = bytes.fromhex("0702020100030102")


class Clock:
    def __init__(self):
        self.now = 1000.0

    def __call__(self):
        return self.now


class ParseTest(unittest.TestCase):
    def test_the_reports_give_the_power_the_charge_and_the_load(self):
        found = parse(MAINS, capacity(100), LOAD)
        self.assertTrue(found["mains"])
        self.assertFalse(found["low"])
        self.assertEqual(100, found["charge"])
        self.assertIsNone(found["runtime"])
        self.assertEqual(1, found["load"])
        self.assertEqual("on mains", state(found))

    def test_the_discharging_bit_alone_does_not_mean_on_battery(self):
        self.assertTrue(parse(MAINS, capacity(100), LOAD)["mains"])
        self.assertFalse(parse(BATTERY, capacity(90, 1800), LOAD)["mains"])
        self.assertEqual(1800, parse(BATTERY, capacity(90, 1800), LOAD)["runtime"])

    def test_the_low_flag_or_an_imminent_shutdown_is_a_low_battery(self):
        self.assertEqual("low battery", state(parse(LOW, capacity(30), LOAD)))
        self.assertEqual("low battery", state(parse(bytes.fromhex("01300001"), capacity(30), LOAD)))
        self.assertEqual("unreachable", state(None))

    def test_reports_of_another_device_are_refused(self):
        with self.assertRaises(ValueError):
            parse(bytes.fromhex("0231"), capacity(100), LOAD)


class FindTest(unittest.TestCase):
    def test_the_device_is_found_by_its_vendor(self):
        with tempfile.TemporaryDirectory() as root:
            for name, vendor, bus, number in (("1-1", "1d6b", 1, 1), ("3-2", "2B89", 3, 3)):
                folder = Path(root, name)
                folder.mkdir()
                (folder / "idVendor").write_text(vendor + "\n")
                (folder / "busnum").write_text(f"{bus}\n")
                (folder / "devnum").write_text(f"{number}\n")
            self.assertEqual("/dev/bus/usb/003/003", find("2b89", sysfs=root))
            self.assertIsNone(find("0463", sysfs=root))


class WatchTest(unittest.TestCase):
    def setUp(self):
        self.clock = Clock()
        self.watch = Watch(20, clock=self.clock)

    def tick(self, status, charge):
        self.clock.now += 5
        return self.watch.update(parse(status, capacity(charge), LOAD))

    def test_a_first_reading_on_mains_sends_nothing(self):
        self.assertEqual([], self.tick(MAINS, 100))
        self.assertEqual("on mains", self.watch.status()["state"])

    def test_an_outage_and_its_end_are_sent_once(self):
        self.tick(MAINS, 100)
        self.assertEqual(["on battery"], self.tick(BATTERY, 99))
        self.assertEqual([], self.tick(BATTERY, 98))
        self.assertEqual(["on mains"], self.tick(MAINS, 98))

    def test_a_low_battery_shuts_down_once_after_two_readings_on_battery(self):
        self.tick(MAINS, 100)
        self.assertEqual(["on battery"], self.tick(BATTERY, 20))
        self.assertEqual(["shutdown"], self.tick(BATTERY, 19))
        self.assertTrue(self.watch.stopping)
        self.assertEqual([], self.tick(BATTERY, 18))

    def test_a_low_charge_on_mains_never_shuts_down(self):
        self.tick(MAINS, 5)
        self.assertEqual([], self.tick(MAINS, 5))
        self.assertFalse(self.watch.stopping)

    def test_a_stale_reading_is_unreachable(self):
        self.tick(MAINS, 100)
        self.clock.now += 60
        self.assertEqual("unreachable", self.watch.status()["state"])
        self.assertFalse(self.watch.healthy())


class ServeTest(unittest.TestCase):
    def test_the_status_and_the_health_are_served(self):
        clock = Clock()
        watch = Watch(20, clock=clock)
        server = serve(watch, port=0)
        base = f"http://127.0.0.1:{server.server_address[1]}"
        try:
            with self.assertRaises(urllib.error.HTTPError) as caught:
                urllib.request.urlopen(base + "/health", timeout=5)
            self.assertEqual(503, caught.exception.code)
            caught.exception.close()
            watch.update(parse(MAINS, capacity(100), LOAD))
            with urllib.request.urlopen(base + "/status", timeout=5) as response:
                status = json.loads(response.read())
            self.assertEqual("on mains", status["state"])
            self.assertEqual(100, status["charge"])
            with urllib.request.urlopen(base + "/health", timeout=5) as response:
                self.assertEqual(200, response.status)
        finally:
            server.shutdown()
            server.server_close()


class SettingsTest(unittest.TestCase):
    def test_the_api_key_is_required(self):
        with self.assertRaises(SystemExit):
            settings({})

    def test_the_defaults_fit_the_ugreen_us3000(self):
        found = settings({"TRUENAS_API_KEY": "key", "DISCORD_ALERTS_WEBHOOK_ID": "1", "DISCORD_ALERTS_WEBHOOK_TOKEN": "t"})
        self.assertEqual("2b89", found["vendor"])
        self.assertEqual(20, found["threshold"])
        self.assertEqual("host.docker.internal", found["host"])
        self.assertEqual("1/t", found["hook"])
        self.assertEqual("", settings({"TRUENAS_API_KEY": "key"})["hook"])


if __name__ == "__main__":
    unittest.main()
