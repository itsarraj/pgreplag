/// A small, dependency-free `pg_size_pretty`-style byte formatter
/// (binary units, matching Postgres's own "kB means 1024 bytes"
/// convention).
pub fn format_bytes(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;

    if bytes < 1024 {
        return format!("{bytes} bytes");
    }
    let b = bytes as f64;
    if b >= GB {
        format!("{:.2} GB", b / GB)
    } else if b >= MB {
        format!("{:.2} MB", b / MB)
    } else {
        format!("{:.2} kB", b / KB)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_bytes_small() {
        assert_eq!(format_bytes(0), "0 bytes");
        assert_eq!(format_bytes(1023), "1023 bytes");
    }

    #[test]
    fn format_bytes_kb_mb_gb() {
        assert_eq!(format_bytes(1024), "1.00 kB");
        assert_eq!(format_bytes(1_048_576), "1.00 MB");
        assert_eq!(format_bytes(1_073_741_824), "1.00 GB");
    }
}
