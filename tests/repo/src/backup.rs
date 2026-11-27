use crate::compose::{all_services, included_compose_files, re, root, service_blocks, set};
use std::path::PathBuf;

const STOP_LABEL: &str = "docker-volume-backup.stop-during-backup";
const KEEP_RUNNING: [&str; 7] = [
    "backup",
    "caddy",
    "gluetun",
    "recyclarr",
    "dozzle",
    "autokuma",
    "dashboard",
];

fn backup_compose() -> PathBuf {
    root().join("services/backup/compose.yaml")
}

fn backup() -> Vec<String> {
    service_blocks(&backup_compose()).remove("backup").unwrap()
}

fn setting(lines: &[String], key: &str) -> Option<String> {
    let pattern = re(&format!(r"^      {}:(?: (.*))?$", regex::escape(key)));
    lines.iter().enumerate().find_map(|(index, line)| {
        let found = pattern.captures(line)?;
        let value = match found.get(1).map(|value| value.as_str()) {
            Some(value) if !value.is_empty() => value.to_string(),
            _ => lines[index + 1].trim().to_string(),
        };
        Some(value.trim_matches(['\'', '"']).to_string())
    })
}

fn volumes(lines: &[String]) -> Vec<String> {
    let volume = re(r"^      - (\$\{[A-Z_]+\}\S*)$");
    lines
        .iter()
        .filter_map(|line| volume.captures(line).map(|found| found[1].to_string()))
        .collect()
}

fn mounts_appdata(lines: &[String]) -> bool {
    volumes(lines)
        .iter()
        .any(|volume| volume.starts_with("${APPDATA_ROOT}"))
}

fn backup_setting(key: &str) -> Option<String> {
    setting(&backup(), key)
}

#[test]
fn backup_is_included() {
    assert!(included_compose_files().contains(&backup_compose()));
}

#[test]
fn backup_mounts_the_appdata_read_only() {
    assert!(volumes(&backup()).contains(&"${APPDATA_ROOT}/:/backup/appdata:ro".to_string()));
}

#[test]
fn archives_land_in_the_backup_root() {
    assert!(volumes(&backup()).contains(&"${BACKUP_ROOT}/:/archive".to_string()));
}

#[test]
fn backup_mounts_no_data() {
    assert!(
        !volumes(&backup())
            .iter()
            .any(|volume| volume.starts_with("${DATA_ROOT}"))
    );
}

#[test]
fn backup_runs_every_night() {
    let cron = backup_setting("BACKUP_CRON_EXPRESSION").unwrap();
    assert!(re(r"^\d+ \d+ \* \* \*$").is_match(&cron), "{cron}");
}

#[test]
fn archives_are_kept_30_days() {
    assert_eq!(
        Some("30".to_string()),
        backup_setting("BACKUP_RETENTION_DAYS")
    );
    let prefix = backup_setting("BACKUP_PRUNING_PREFIX").unwrap();
    assert!(
        backup_setting("BACKUP_FILENAME")
            .unwrap()
            .starts_with(&prefix)
    );
}

#[test]
fn a_failed_backup_notifies_discord() {
    assert_eq!(
        Some("discord://${DISCORD_ALERTS_WEBHOOK_TOKEN}@${DISCORD_ALERTS_WEBHOOK_ID}".to_string()),
        backup_setting("NOTIFICATION_URLS")
    );
    let level = backup_setting("NOTIFICATION_LEVEL");
    assert!(
        matches!(level.as_deref(), None | Some("error")),
        "{level:?}"
    );
}

#[test]
fn every_service_with_appdata_says_whether_it_stops() {
    let keep_running = set(KEEP_RUNNING);
    let mut undecided = Vec::new();
    let mut both = Vec::new();
    for (service, lines) in all_services() {
        let stops = setting(&lines, STOP_LABEL).as_deref() == Some("true");
        if mounts_appdata(&lines) && !stops && !keep_running.contains(&service) {
            undecided.push(service.clone());
        }
        if stops && keep_running.contains(&service) {
            both.push(service);
        }
    }
    assert!(undecided.is_empty(), "{undecided:?}");
    assert!(both.is_empty(), "{both:?}");
}

#[test]
fn only_services_with_appdata_stop() {
    for (service, lines) in all_services() {
        if let Some(value) = setting(&lines, STOP_LABEL) {
            assert_eq!("true", value, "{service}");
            assert!(mounts_appdata(&lines), "{service}");
        }
    }
}

#[test]
fn uptime_kuma_stops_with_the_services_it_watches() {
    let uptime_kuma = all_services().remove("uptime-kuma").unwrap();
    assert_eq!(Some("true".to_string()), setting(&uptime_kuma, STOP_LABEL));
}

#[test]
fn authelia_waits_for_lldap_when_the_backup_restarts_them() {
    let mut services = all_services();
    let lldap = services.remove("lldap").unwrap();
    assert_eq!(Some("true".to_string()), setting(&lldap, STOP_LABEL));
    let authelia = services.remove("authelia").unwrap().join(" ");
    assert!(authelia.contains("until wget -q --spider http://lldap:17170/health"));
    assert!(authelia.contains("exec /app/entrypoint.sh \"$$@\""));
}
