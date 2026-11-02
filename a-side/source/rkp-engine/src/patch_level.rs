//! Security patch level encoding shared by DeviceInfo validation and CSR verification.
//!
//! KeyMint encodes patch levels as decimal `YYYYMM` or `YYYYMMDD` integers
//! (`hardware/interfaces/security/keymint/aidl/.../Tag.aidl`).

/// Returns true for a real calendar date in `YYYYMM` or `YYYYMMDD` form.
pub fn is_valid(value: u32) -> bool {
    let encoded = value.to_string();
    let normalized = match encoded.len() {
        6 => format!("{encoded}01"),
        8 => encoded,
        _ => return false,
    };

    let (Ok(year), Ok(month), Ok(day)) = (
        normalized[0..4].parse::<u32>(),
        normalized[4..6].parse::<u32>(),
        normalized[6..8].parse::<u32>(),
    ) else {
        return false;
    };

    (1..=12).contains(&month) && day >= 1 && day <= days_in_month(year, month)
}

fn days_in_month(year: u32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400) => {
            29
        }
        2 => 28,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::is_valid;

    #[test]
    fn accepts_month_and_day_forms() {
        assert!(is_valid(202601));
        assert!(is_valid(20260131));
        assert!(is_valid(20240229));
    }

    #[test]
    fn rejects_impossible_dates() {
        assert!(!is_valid(0));
        assert!(!is_valid(20261301));
        assert!(!is_valid(20250229));
        assert!(!is_valid(20260100));
        assert!(!is_valid(2026));
    }
}
