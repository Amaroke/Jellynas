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
}
