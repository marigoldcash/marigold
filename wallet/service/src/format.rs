//! Plain-text formatting the service and every front end share: amounts, waits
//! and rates, written the way a person reads them (no colour, no markup).

use separator::Separatable;

/// A ledger amount, to the hundredth and no further.
///
/// Below 0.01 nothing can be done with it: the smallest note is 0.01, so a
/// remainder under that cannot be minted, cannot be sent to an exchange, and
/// cannot be spent as a note. Printing `812,524.99677725` asks somebody to
/// read eight digits that change nothing, and hides the figure that matters
/// in the middle of them.
///
/// Truncated, never rounded. `812,524.99677725` shows as `812,524.99`, not
/// `812,525.00` — a balance may be shown as less than it is, and must never
/// be shown as more.
pub fn ledger_amount(petals: u64) -> String {
    use kaspa_consensus_core::constants::SOMPI_PER_KASPA;
    const PER_HUNDREDTH: u64 = SOMPI_PER_KASPA / 100;
    let whole = petals / SOMPI_PER_KASPA;
    let hundredths = (petals % SOMPI_PER_KASPA) / PER_HUNDREDTH;
    format!("{}.{hundredths:02}", whole.separated_string())
}

/// "about 4 hours", "about 20 minutes" — a figure somebody can plan around,
/// not one they should hold us to.
pub fn humanised_minutes(minutes: u64) -> String {
    match minutes {
        0 => "a moment".to_string(),
        1 => "a minute".to_string(),
        m if m < 90 => format!("{m} minutes"),
        m => {
            let hours = (m + 30) / 60;
            format!("{hours} hour{}", if hours == 1 { "" } else { "s" })
        }
    }
}

/// Human-readable hash rate. A CPU on this chain is in the kilohash range, so
/// "0.00 MH/s" — which is what the stratum bridge shows — tells nobody
/// anything.
pub fn format_hashrate(rate: f64) -> String {
    if rate >= 1_000_000.0 {
        format!("{:.2} MH/s", rate / 1_000_000.0)
    } else if rate >= 1_000.0 {
        format!("{:.1} kH/s", rate / 1_000.0)
    } else {
        format!("{rate:.0} H/s")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_ledger_amount_stops_at_the_hundredth() {
        assert_eq!(ledger_amount(81_252_499_677_725), "812,524.99");
        assert_eq!(ledger_amount(34_148_284_000_000), "341,482.84");
        assert_eq!(ledger_amount(0), "0.00");
        assert_eq!(ledger_amount(100_000_000), "1.00");
    }

    /// The direction that matters. Rounding would have turned
    /// 812,524.99677725 into 812,525.00 — more money than the person has.
    #[test]
    fn it_truncates_rather_than_rounds() {
        assert_eq!(ledger_amount(199_999_999), "1.99", "1.99999999 is not 2.00");
        assert_eq!(ledger_amount(999_999), "0.00", "dust below a hundredth is nothing you can use");
    }

    #[test]
    fn large_amounts_keep_their_separators() {
        assert_eq!(ledger_amount(1_234_567_800_000_000), "12,345,678.00");
    }

    #[test]
    fn hashrate_reads_in_units_a_person_can_use() {
        assert_eq!(format_hashrate(12.0), "12 H/s");
        assert_eq!(format_hashrate(4_500.0), "4.5 kH/s");
        assert_eq!(format_hashrate(2_500_000.0), "2.50 MH/s");
    }
}
