use serde_json::{Value, json};
use std::fs::{self, File, OpenOptions};
use std::io;
use std::os::fd::AsRawFd;
use std::path::Path;

const UNKNOWN_RUNTIME: u32 = 0xFFFF;

#[repr(C)]
struct Control {
    request_type: u8,
    request: u8,
    value: u16,
    index: u16,
    length: u16,
    timeout: u32,
    data: *mut libc::c_void,
}

const CONTROL: u64 = (3 << 30) | ((size_of::<Control>() as u64) << 16) | ((b'U' as u64) << 8);

#[derive(Clone, Debug, PartialEq)]
pub struct Reading {
    pub mains: bool,
    pub low: bool,
    pub failure: bool,
    pub replace: bool,
    pub overload: bool,
    pub imminent: bool,
    pub charge: u8,
    pub runtime: Option<u32>,
    pub load: Option<u8>,
}

impl Reading {
    pub fn json(&self) -> Value {
        json!({
            "mains": self.mains,
            "low": self.low,
            "failure": self.failure,
            "replace": self.replace,
            "overload": self.overload,
            "imminent": self.imminent,
            "charge": self.charge,
            "runtime": self.runtime,
            "load": self.load,
        })
    }
}

pub fn find(vendor: &str, sysfs: &Path, dev: &str) -> Option<String> {
    let mut devices: Vec<_> = fs::read_dir(sysfs)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .collect();
    devices.sort();
    devices.into_iter().find_map(|device| {
        let read = |name: &str| {
            fs::read_to_string(device.join(name))
                .ok()
                .map(|text| text.trim().to_string())
        };
        if !read("idVendor")?.eq_ignore_ascii_case(vendor) {
            return None;
        }
        let bus: u32 = read("busnum")?.parse().ok()?;
        let number: u32 = read("devnum")?.parse().ok()?;
        Some(format!("{dev}/{bus:03}/{number:03}"))
    })
}

fn feature(file: &File, report: u8) -> Result<Vec<u8>, String> {
    let mut buffer = [0u8; 16];
    let mut control = Control {
        request_type: 0xA1,
        request: 0x01,
        value: (3 << 8) | u16::from(report),
        index: 0,
        length: buffer.len() as u16,
        timeout: 1000,
        data: buffer.as_mut_ptr().cast(),
    };
    let got = unsafe { libc::ioctl(file.as_raw_fd(), CONTROL as _, &mut control) };
    if got < 0 {
        return Err(format!(
            "cannot read the report {report:#04x}: {}",
            io::Error::last_os_error()
        ));
    }
    Ok(buffer[..got as usize].to_vec())
}

pub fn parse(status: &[u8], capacity: &[u8], load: &[u8]) -> Result<Reading, String> {
    if status.len() < 4 || status[0] != 0x01 || capacity.len() < 6 || capacity[0] != 0x06 {
        return Err("unexpected reports from the UPS".to_string());
    }
    let flags = status[1];
    let runtime = u32::from_le_bytes([capacity[2], capacity[3], capacity[4], capacity[5]]);
    Ok(Reading {
        mains: flags & 0x01 != 0,
        low: flags & 0x02 != 0,
        failure: flags & 0x40 != 0,
        replace: flags & 0x80 != 0,
        overload: status[2] != 0,
        imminent: status[3] != 0,
        charge: capacity[1],
        runtime: (runtime < UNKNOWN_RUNTIME).then_some(runtime),
        load: (load.len() > 6 && load[0] == 0x07).then(|| load[6]),
    })
}

pub fn read(vendor: &str) -> Result<Reading, String> {
    let path = find(vendor, Path::new("/sys/bus/usb/devices"), "/dev/bus/usb")
        .ok_or_else(|| format!("no USB device of vendor {vendor}"))?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .map_err(|error| format!("cannot open {path}: {error}"))?;
    parse(
        &feature(&file, 0x01)?,
        &feature(&file, 0x06)?,
        &feature(&file, 0x07)?,
    )
}

#[cfg(test)]
pub mod tests {
    use super::*;

    pub const MAINS: [u8; 4] = [0x01, 0x31, 0x00, 0x00];
    pub const BATTERY: [u8; 4] = [0x01, 0x30, 0x00, 0x00];
    pub const LOW: [u8; 4] = [0x01, 0x32, 0x00, 0x00];
    pub const LOAD: [u8; 8] = [0x07, 0x02, 0x02, 0x01, 0x00, 0x03, 0x01, 0x02];

    pub fn capacity(charge: u8, runtime: u32) -> Vec<u8> {
        let mut found = vec![0x06, charge];
        found.extend(runtime.to_le_bytes());
        found
    }

    #[test]
    fn the_control_request_matches_the_usbfs_ioctl() {
        assert_eq!(0xC018_5500, CONTROL);
    }

    #[test]
    fn the_reports_give_the_power_the_charge_and_the_load() {
        let found = parse(&MAINS, &capacity(100, 0xFFFF), &LOAD).unwrap();
        assert!(found.mains);
        assert!(!found.low);
        assert_eq!(100, found.charge);
        assert_eq!(None, found.runtime);
        assert_eq!(Some(1), found.load);
    }

    #[test]
    fn the_discharging_bit_alone_does_not_mean_on_battery() {
        assert!(parse(&MAINS, &capacity(100, 0xFFFF), &LOAD).unwrap().mains);
        let battery = parse(&BATTERY, &capacity(90, 1800), &LOAD).unwrap();
        assert!(!battery.mains);
        assert_eq!(Some(1800), battery.runtime);
    }

    #[test]
    fn the_flags_of_trouble_are_read() {
        let low = parse(&LOW, &capacity(30, 600), &[]).unwrap();
        assert!(low.low);
        assert_eq!(None, low.load);
        let imminent = parse(&[0x01, 0x30, 0x01, 0x01], &capacity(30, 600), &LOAD).unwrap();
        assert!(imminent.overload && imminent.imminent);
    }

    #[test]
    fn reports_of_another_device_are_refused() {
        assert!(parse(&[0x02, 0x31], &capacity(100, 0), &LOAD).is_err());
        assert!(parse(&MAINS, &[0x06, 100], &LOAD).is_err());
    }

    #[test]
    fn the_device_is_found_by_its_vendor() {
        let root = tempfile::tempdir().unwrap();
        for (name, vendor, bus, number) in [("1-1", "1d6b", 1, 1), ("3-2", "2B89", 3, 3)] {
            let folder = root.path().join(name);
            fs::create_dir(&folder).unwrap();
            fs::write(folder.join("idVendor"), format!("{vendor}\n")).unwrap();
            fs::write(folder.join("busnum"), format!("{bus}\n")).unwrap();
            fs::write(folder.join("devnum"), format!("{number}\n")).unwrap();
        }
        fs::create_dir(root.path().join("usb1")).unwrap();
        assert_eq!(
            Some("/dev/bus/usb/003/003".to_string()),
            find("2b89", root.path(), "/dev/bus/usb")
        );
        assert_eq!(None, find("0463", root.path(), "/dev/bus/usb"));
    }
}
