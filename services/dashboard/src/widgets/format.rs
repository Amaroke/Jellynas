pub fn count(value: u64) -> String {
    let digits = value.to_string();
    let mut grouped = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    grouped
}

pub fn bytes(value: f64) -> String {
    const UNITS: [&str; 5] = ["KiB", "MiB", "GiB", "TiB", "PiB"];
    if value.abs() < 1024.0 {
        return format!("{value:.0} B");
    }
    let mut scaled = value / 1024.0;
    let mut unit = 0;
    while scaled.abs() >= 1024.0 && unit < UNITS.len() - 1 {
        scaled /= 1024.0;
        unit += 1;
    }
    format!("{scaled:.1} {}", UNITS[unit])
}

pub fn rate(value: f64) -> String {
    format!("{}/s", bytes(value))
}

pub fn percent(value: f64) -> String {
    format!("{value:.1}%")
}

pub fn duration(seconds: u64) -> String {
    let (days, hours, minutes) = (seconds / 86400, seconds / 3600 % 24, seconds / 60 % 60);
    if days > 0 {
        format!("{days}d {hours}h")
    } else if hours > 0 {
        format!("{hours}h {minutes}m")
    } else {
        format!("{minutes}m")
    }
}

pub fn epoch(text: &str) -> Option<u64> {
    let number = |range: std::ops::Range<usize>| text.get(range)?.parse::<i64>().ok();
    let (year, month, day) = (number(0..4)?, number(5..7)?, number(8..10)?);
    let (hour, minute, second) = (number(11..13)?, number(14..16)?, number(17..19)?);
    let zone = text[19..].trim_start_matches(|c: char| c == '.' || c.is_ascii_digit());
    let offset = match zone {
        "Z" | "z" => 0,
        _ => {
            let sign = match zone.get(..1)? {
                "+" => 1,
                "-" => -1,
                _ => return None,
            };
            let hours: i64 = zone.get(1..3)?.parse().ok()?;
            let minutes: i64 = zone.get(4..6)?.parse().ok()?;
            sign * (hours * 3600 + minutes * 60)
        }
    };
    let shifted = if month <= 2 { year - 1 } else { year };
    let era = shifted.div_euclid(400);
    let of_era = shifted - era * 400;
    let of_year = (153 * ((month + 9) % 12) + 2) / 5 + day - 1;
    let of_cycle = of_era * 365 + of_era / 4 - of_era / 100 + of_year;
    let days = era * 146_097 + of_cycle - 719_468;
    let seconds = days * 86400 + hour * 3600 + minute * 60 + second - offset;
    u64::try_from(seconds).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_group_their_thousands() {
        assert_eq!("0", count(0));
        assert_eq!("999", count(999));
        assert_eq!("16,803", count(16803));
        assert_eq!("1,234,567", count(1234567));
    }

    #[test]
    fn sizes_use_binary_units() {
        assert_eq!("0 B", bytes(0.0));
        assert_eq!("512 B", bytes(512.0));
        assert_eq!("1.5 KiB", bytes(1536.0));
        assert_eq!("41.1 KiB", bytes(42060.0));
        assert_eq!("2.0 GiB", bytes(2_147_483_648.0));
        assert_eq!("-1.0 MiB", bytes(-1_048_576.0));
        assert_eq!("41.1 KiB/s", rate(42060.0));
    }

    #[test]
    fn percents_keep_one_decimal() {
        assert_eq!("6.4%", percent(6.4));
        assert_eq!("76.6%", percent(76.56));
        assert_eq!("100.0%", percent(100.0));
    }

    #[test]
    fn durations_show_their_two_largest_units() {
        assert_eq!("0m", duration(0));
        assert_eq!("45m", duration(45 * 60 + 12));
        assert_eq!("14h 43m", duration(52994));
        assert_eq!("3d 4h", duration(3 * 86400 + 4 * 3600 + 59));
    }

    #[test]
    fn instants_read_rfc3339_with_any_offset() {
        assert_eq!(Some(0), epoch("1970-01-01T00:00:00Z"));
        assert_eq!(Some(1_795_335_977), epoch("2026-11-22T08:26:17.955548255Z"));
        assert_eq!(Some(1_795_314_701), epoch("2026-11-22T03:31:41.565+01:00"));
        assert_eq!(Some(1_709_210_096), epoch("2024-02-29T12:34:56Z"));
        assert_eq!(None, epoch("yesterday"));
    }
}
