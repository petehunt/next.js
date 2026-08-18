//! Formatting a post's date.
//!
//! `blog-starter` does this in a React component with `date-fns`:
//!
//! ```tsx
//! const DateFormatter = ({ dateString }) => {
//!   const date = parseISO(dateString);
//!   return <time dateTime={dateString}>{format(date, "LLLL d, yyyy")}</time>;
//! };
//! ```
//!
//! In the Rust port the same output is produced on the server. That removes a
//! component from the browser bundle, and — more usefully — makes the rendered
//! date independent of the reader's locale and clock, which the `date-fns`
//! version is not.

/// Why a date could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DateError {
    pub input: String,
}

impl std::fmt::Display for DateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "`{}` is not an ISO 8601 date", self.input)
    }
}

impl std::error::Error for DateError {}

const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

/// Formats an ISO 8601 date the way `format(date, "LLLL d, yyyy")` does.
///
/// Only the date part is read. The posts carry a UTC timestamp, and rendering
/// the *calendar* date in UTC is the one choice that gives every reader the same
/// answer.
pub fn format_long(iso: &str) -> Result<String, DateError> {
    let (year, month, day) = parse_iso_date(iso)?;
    Ok(format!("{} {day}, {year}", MONTHS[usize::from(month) - 1]))
}

/// Reads `YYYY-MM-DD`, ignoring any time component that follows.
pub fn parse_iso_date(iso: &str) -> Result<(i32, u8, u8), DateError> {
    let error = || DateError {
        input: iso.to_owned(),
    };
    let date = iso.split(['T', ' ']).next().ok_or_else(error)?;
    let mut parts = date.split('-');

    let year: i32 = parts
        .next()
        .ok_or_else(error)?
        .parse()
        .map_err(|_| error())?;
    let month: u8 = parts
        .next()
        .ok_or_else(error)?
        .parse()
        .map_err(|_| error())?;
    let day: u8 = parts
        .next()
        .ok_or_else(error)?
        .parse()
        .map_err(|_| error())?;

    if parts.next().is_some() {
        return Err(error());
    }
    if !(1..=12).contains(&month) || day == 0 || day > days_in_month(year, month) {
        return Err(error());
    }
    Ok((year, month, day))
}

fn days_in_month(year: i32, month: u8) -> u8 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

fn is_leap_year(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

/// Sort key for "newest first".
///
/// `blog-starter` sorts by comparing the raw ISO strings, which works because
/// they are all the same fixed-width UTC format. Comparing the parsed date is
/// the same order for well-formed input and a defined one for the rest.
pub fn sort_key(iso: &str) -> (i32, u8, u8) {
    parse_iso_date(iso).unwrap_or((0, 1, 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_the_way_date_fns_does() {
        assert_eq!(
            format_long("2020-03-16T05:35:07.322Z").unwrap(),
            "March 16, 2020"
        );
        assert_eq!(format_long("2020-01-01").unwrap(), "January 1, 2020");
        assert_eq!(format_long("1999-12-31").unwrap(), "December 31, 1999");
    }

    #[test]
    fn the_day_is_not_zero_padded() {
        assert_eq!(format_long("2020-03-05").unwrap(), "March 5, 2020");
    }

    #[test]
    fn every_month_has_a_name() {
        for month in 1..=12u8 {
            let formatted = format_long(&format!("2021-{month:02}-01")).unwrap();
            assert!(formatted.ends_with("1, 2021"), "{formatted}");
        }
    }

    #[test]
    fn a_space_separated_timestamp_is_accepted() {
        assert_eq!(
            format_long("2020-03-16 05:35:07").unwrap(),
            "March 16, 2020"
        );
    }

    #[test]
    fn nonsense_is_an_error_rather_than_a_wrong_date() {
        for input in [
            "",
            "not a date",
            "2020",
            "2020-13-01",
            "2020-00-01",
            "2020-02-30",
            "2020-03-00",
            "2020-03-16-01",
        ] {
            assert!(format_long(input).is_err(), "{input} should fail");
        }
    }

    #[test]
    fn february_has_the_right_length() {
        assert!(format_long("2020-02-29").is_ok(), "2020 is a leap year");
        assert!(format_long("2021-02-29").is_err(), "2021 is not");
        assert!(format_long("2000-02-29").is_ok(), "2000 is a leap year");
        assert!(format_long("1900-02-29").is_err(), "1900 is not");
    }

    #[test]
    fn sorting_puts_the_newest_first() {
        let mut dates = vec!["2020-03-16T05:35:07.322Z", "2021-01-01", "2019-06-30"];
        dates.sort_by_key(|date| std::cmp::Reverse(sort_key(date)));
        assert_eq!(
            dates,
            ["2021-01-01", "2020-03-16T05:35:07.322Z", "2019-06-30"]
        );
    }

    #[test]
    fn an_unreadable_date_sorts_last_rather_than_panicking() {
        assert_eq!(sort_key("nonsense"), (0, 1, 1));
    }
}
