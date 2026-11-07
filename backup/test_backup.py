import re
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
BACKUP = ROOT / "backup" / "compose.yaml"
STOP_LABEL = "docker-volume-backup.stop-during-backup"
KEEP_RUNNING = {"backup", "caddy", "gluetun", "recyclarr", "qbit_manage", "dozzle", "autokuma"}


def included_compose_files():
    text = (ROOT / "compose.yaml").read_text(encoding="utf-8")
    return [ROOT / path for path in re.findall(r"^  - ([^ #]*compose\.yaml)$", text, re.MULTILINE)]


def service_blocks(path):
    services, section, service = {}, None, None
    for line in path.read_text(encoding="utf-8").splitlines():
        top = re.match(r"^([a-z_]+):", line)
        if top:
            section, service = top.group(1), None
            continue
        name = re.match(r"^  ([a-z0-9_-]+):$", line)
        if section == "services" and name:
            service = name.group(1)
            services[service] = []
            continue
        if service:
            services[service].append(line)
    return services


def all_services():
    services = {}
    for path in included_compose_files():
        services.update(service_blocks(path))
    return services


def setting(lines, key):
    for line in lines:
        found = re.match(rf"^      {re.escape(key)}: (.*)$", line)
        if found:
            return found.group(1).strip("'\"")
    return None


def volumes(lines):
    return [found.group(1) for line in lines if (found := re.match(r"^      - (\$\{[A-Z_]+\}\S*)$", line))]


def mounts_appdata(lines):
    return any(volume.startswith("${APPDATA_ROOT}") for volume in volumes(lines))


class BackupTest(unittest.TestCase):
    def backup(self):
        return service_blocks(BACKUP)["backup"]

    def test_backup_is_included(self):
        self.assertIn(BACKUP, included_compose_files())

    def test_backup_mounts_the_appdata_read_only(self):
        self.assertIn("${APPDATA_ROOT}/:/backup/appdata:ro", volumes(self.backup()))

    def test_archives_land_in_the_backup_root(self):
        self.assertIn("${BACKUP_ROOT}/:/archive", volumes(self.backup()))

    def test_backup_mounts_no_data(self):
        self.assertFalse([volume for volume in volumes(self.backup()) if volume.startswith("${DATA_ROOT}")])

    def test_backup_runs_every_night(self):
        self.assertRegex(setting(self.backup(), "BACKUP_CRON_EXPRESSION"), r"^\d+ \d+ \* \* \*$")

    def test_archives_are_kept_30_days(self):
        self.assertEqual("30", setting(self.backup(), "BACKUP_RETENTION_DAYS"))
        self.assertTrue(setting(self.backup(), "BACKUP_FILENAME").startswith(setting(self.backup(), "BACKUP_PRUNING_PREFIX")))

    def test_a_failed_backup_notifies_ntfy(self):
        self.assertEqual("ntfy://ntfy.sh/${NTFY_TOPIC}", setting(self.backup(), "NOTIFICATION_URLS"))
        self.assertIn(setting(self.backup(), "NOTIFICATION_LEVEL"), {None, "error"})

    def test_every_service_with_appdata_says_whether_it_stops(self):
        undecided, both = set(), set()
        for service, lines in all_services().items():
            stops = setting(lines, STOP_LABEL) == "true"
            if mounts_appdata(lines) and not stops and service not in KEEP_RUNNING:
                undecided.add(service)
            if stops and service in KEEP_RUNNING:
                both.add(service)
        self.assertEqual(set(), undecided)
        self.assertEqual(set(), both)

    def test_only_services_with_appdata_stop(self):
        for service, lines in all_services().items():
            if setting(lines, STOP_LABEL) is not None:
                self.assertEqual("true", setting(lines, STOP_LABEL), service)
                self.assertTrue(mounts_appdata(lines), service)

    def test_uptime_kuma_stops_with_the_services_it_watches(self):
        self.assertEqual("true", setting(all_services()["uptime-kuma"], STOP_LABEL))


if __name__ == "__main__":
    unittest.main()
