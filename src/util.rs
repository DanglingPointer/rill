use gettextrs::gettext;

/// Formats a byte count in binary units: B, KiB, MiB, GiB, TiB.
pub fn format_size(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KiB", "MiB", "GiB", "TiB"];
    let mut size = bytes as f64;
    let mut unit_index = 0;

    while size >= 1024.0 && unit_index < UNITS.len() - 1 {
        size /= 1024.0;
        unit_index += 1;
    }

    if unit_index == 0 {
        format!("{} {}", bytes, UNITS[0])
    } else {
        format!("{:.1} {}", size, UNITS[unit_index])
    }
}

/// A transfer rate, "1.5 MiB/s".
pub fn format_rate(bytes_per_second: u64) -> String {
    // Translators: a transfer rate; %s is a size such as "1.5 MiB".
    gettext("%s/s").replace("%s", &format_size(bytes_per_second))
}

/// The time left for `remaining` bytes at `speed` bytes a second, to the two largest
/// units: "45s", "3m 20s", "2h 5m", "1d 4h".
pub fn format_eta(remaining: u64, speed: u64) -> String {
    let secs = remaining / speed.max(1);
    let (d, h, m, s) = (
        secs / 86400,
        secs % 86400 / 3600,
        secs % 3600 / 60,
        secs % 60,
    );
    let (template, a, b) = if secs < 60 {
        // Translators: seconds.
        (gettext("%1s"), s, 0)
    } else if secs < 3600 {
        // Translators: minutes and seconds.
        (gettext("%1m %2s"), m, s)
    } else if secs < 86400 {
        // Translators: hours and minutes.
        (gettext("%1h %2m"), h, m)
    } else {
        // Translators: days and hours.
        (gettext("%1d %2h"), d, h)
    };
    template
        .replace("%1", &a.to_string())
        .replace("%2", &b.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_use_binary_units() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(1023), "1023 B");
        assert_eq!(format_size(1536), "1.5 KiB");
        assert_eq!(format_size(1024 * 1024), "1.0 MiB");
    }

    #[test]
    fn eta_shows_the_two_largest_units() {
        assert_eq!(format_eta(45, 1), "45s");
        assert_eq!(format_eta(200, 1), "3m 20s");
        assert_eq!(format_eta(7500, 1), "2h 5m");
        assert_eq!(format_eta(100_800, 1), "1d 4h");
        assert_eq!(format_eta(10, 0), "10s");
    }
}
