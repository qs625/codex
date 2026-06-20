/// Format an i64 with digit separators (e.g. "12345" -> "12,345"
/// for en-US).
pub fn format_with_separators(n: i64) -> String {
    let negative = n < 0;
    let mut digits = i128::from(n).abs().to_string();
    let mut formatted = String::with_capacity(digits.len() + digits.len() / 3 + 1);

    let first_group_len = digits.len() % 3;
    if negative {
        formatted.push('-');
    }

    if first_group_len > 0 {
        formatted.push_str(&digits[..first_group_len]);
        digits.drain(..first_group_len);
        if !digits.is_empty() {
            formatted.push(',');
        }
    }

    for (idx, chunk) in digits.as_bytes().chunks(3).enumerate() {
        if idx > 0 {
            formatted.push(',');
        }
        for &byte in chunk {
            formatted.push(char::from(byte));
        }
    }

    formatted
}

fn format_scaled(n: i64, scale: i64, frac_digits: u32) -> String {
    let factor = 10_i64.pow(frac_digits);
    let value = n as f64 / scale as f64;
    let scaled = (value * factor as f64).round() as i64;
    let whole = scaled / factor;

    if frac_digits == 0 {
        return format_with_separators(whole);
    }

    let frac = scaled.abs() % factor;
    format!(
        "{}.{:0width$}",
        format_with_separators(whole),
        frac,
        width = frac_digits as usize
    )
}

fn format_si_suffix_inner(n: i64) -> String {
    let n = n.max(0);
    if n < 1000 {
        return format_with_separators(n);
    }

    const UNITS: [(i64, &str); 3] = [(1_000, "K"), (1_000_000, "M"), (1_000_000_000, "G")];
    let f = n as f64;
    for &(scale, suffix) in &UNITS {
        if (100.0 * f / scale as f64).round() < 1000.0 {
            return format!("{}{}", format_scaled(n, scale, 2), suffix);
        } else if (10.0 * f / scale as f64).round() < 1000.0 {
            return format!("{}{}", format_scaled(n, scale, 1), suffix);
        } else if (f / scale as f64).round() < 1000.0 {
            return format!("{}{}", format_scaled(n, scale, 0), suffix);
        }
    }

    // Above 1000G, keep whole-G precision.
    format!(
        "{}G",
        format_with_separators(((n as f64) / 1e9).round() as i64)
    )
}

/// Format token counts to 3 significant figures, using base-10 SI suffixes.
///
/// Examples (en-US):
///   - 999 -> "999"
///   - 1200 -> "1.20K"
///   - 123456789 -> "123M"
pub fn format_si_suffix(n: i64) -> String {
    format_si_suffix_inner(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kmg() {
        let fmt = format_si_suffix;
        assert_eq!(fmt(0), "0");
        assert_eq!(fmt(999), "999");
        assert_eq!(fmt(1_000), "1.00K");
        assert_eq!(fmt(1_200), "1.20K");
        assert_eq!(fmt(10_000), "10.0K");
        assert_eq!(fmt(100_000), "100K");
        assert_eq!(fmt(999_500), "1.00M");
        assert_eq!(fmt(1_000_000), "1.00M");
        assert_eq!(fmt(1_234_000), "1.23M");
        assert_eq!(fmt(12_345_678), "12.3M");
        assert_eq!(fmt(999_950_000), "1.00G");
        assert_eq!(fmt(1_000_000_000), "1.00G");
        assert_eq!(fmt(1_234_000_000), "1.23G");
        // Above 1000G we keep whole-G precision (no higher unit supported here).
        assert_eq!(fmt(1_234_000_000_000), "1,234G");
    }

    #[test]
    fn separators() {
        assert_eq!(format_with_separators(0), "0");
        assert_eq!(format_with_separators(999), "999");
        assert_eq!(format_with_separators(1_000), "1,000");
        assert_eq!(format_with_separators(12_345_678), "12,345,678");
        assert_eq!(format_with_separators(-12_345_678), "-12,345,678");
        assert_eq!(
            format_with_separators(i64::MIN),
            "-9,223,372,036,854,775,808"
        );
    }
}
