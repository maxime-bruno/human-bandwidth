//! Human-friendly bandwidth parser and formatter
//!
//! ## Facilities:
//!
//! * Parses bandwidth in free form like `2Gbps 340Mbps` or `2.34Gbps`
//! * Formats bandwidth in similar form `150.024kbps` (default) or `150kbps 24bps` (with feature `display-integer` enabled)
//!
//! ## Features
//!
//! * Enable `serde` feature for serde integration.
//! * Enable `display-integer` feature to display integer values only.
//! * Enable `binary-system` feature to display in binary prefix system (e.g. `1KiB/s` instead of `8.192kbps`)

use std::{error::Error as StdError, fmt, str::Chars};

#[cfg(feature = "binary-system")]
pub mod binary_system;
#[cfg(feature = "serde")]
pub mod option;
#[cfg(feature = "serde")]
pub mod serde;

/// Reexport module
pub mod re {
    pub use bandwidth;
}

use bandwidth::Bandwidth;

const FRACTION_PART_LIMIT: u32 = 12;

/// Error parsing human-friendly bandwidth
#[derive(Debug, PartialEq, Clone)]
pub enum Error {
    /// Invalid character during parsing
    ///
    /// More specifically anything that is not alphanumeric is prohibited
    ///
    /// The field is an byte offset of the character in the string.
    InvalidCharacter(usize),
    /// Non-numeric value where number is expected
    ///
    /// This usually means that either bandwidth unit is broken into words,
    /// e.g. `M bps` instead of `Mbps`, or just number is omitted,
    /// for example `2 Mbps kbps` instead of `2 Mbps 1 kbps`
    ///
    /// The field is an byte offset of the erroneous character
    /// in the string.
    NumberExpected(usize),
    /// Unit in the number is not one of allowed units
    ///
    /// See documentation of `parse_bandwidth` for the list of supported
    /// bandwidth units.
    ///
    /// The two fields are start and end (exclusive) of the slice from
    /// the original string, containing erroneous value
    UnknownUnit {
        /// Start of the invalid unit inside the original string
        start: usize,
        /// End of the invalid unit inside the original string
        end: usize,
        /// The unit verbatim
        unit: String,
        /// A number associated with the unit
        value: u64,
    },
    #[cfg(feature = "binary-system")]
    /// Unit in the number is not one of allowed units (in the binary prefix system)
    ///
    /// See documentation of `parse_binary_bandwidth` for the list of supported
    /// bandwidth units.
    ///
    /// The two fields are start and end (exclusive) of the slice from
    /// the original string, containing erroneous value
    UnknownBinaryUnit {
        /// Start of the invalid unit inside the original string
        start: usize,
        /// End of the invalid unit inside the original string
        end: usize,
        /// The unit verbatim
        unit: String,
        /// A number associated with the unit
        value: u64,
    },
    /// The numeric value is too large
    ///
    /// Usually this means value is too large to be useful.
    NumberOverflow,
    /// The value was an empty string (or consists only whitespace)
    Empty,
}

impl StdError for Error {}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::InvalidCharacter(offset) => write!(f, "invalid character at {}", offset),
            Error::NumberExpected(offset) => write!(f, "expected number at {}", offset),
            Error::UnknownUnit { unit, value, .. } if unit.is_empty() => {
                write!(
                    f,
                    "bandwidth unit needed, for example {0}Mbps or {0}bps",
                    value,
                )
            }
            Error::UnknownUnit { unit, .. } => {
                write!(
                    f,
                    "unknown bandwidth unit {:?}, \
                    supported units: bps, kbps, Mbps, Gbps, Tbps",
                    unit
                )
            }
            #[cfg(feature = "binary-system")]
            Error::UnknownBinaryUnit { unit, value, .. } if unit.is_empty() => {
                write!(
                    f,
                    "binary bandwidth unit needed, for example {0}MiB/s or {0}B/s",
                    value,
                )
            }
            #[cfg(feature = "binary-system")]
            Error::UnknownBinaryUnit { unit, .. } => {
                write!(
                    f,
                    "unknown binary bandwidth unit {:?}, \
                    supported units: B/s, KiB/s, MiB/s, GiB/s, TiB/s",
                    unit
                )
            }
            Error::NumberOverflow => write!(f, "number is too large"),
            Error::Empty => write!(f, "value was empty"),
        }
    }
}

/// A wrapper type that allows you to Display a Bandwidth
#[derive(Debug, Clone)]
pub struct FormattedBandwidth(Bandwidth);

trait OverflowOp: Sized {
    fn mul(self, other: Self) -> Result<Self, Error>;
    fn add(self, other: Self) -> Result<Self, Error>;
}

impl OverflowOp for u64 {
    fn mul(self, other: Self) -> Result<Self, Error> {
        self.checked_mul(other).ok_or(Error::NumberOverflow)
    }
    fn add(self, other: Self) -> Result<Self, Error> {
        self.checked_add(other).ok_or(Error::NumberOverflow)
    }
}

fn parse_fraction(fraction: u64, fraction_cnt: u32, need_digit: u32) -> u64 {
    if need_digit >= fraction_cnt {
        fraction * 10u64.pow(need_digit - fraction_cnt)
    } else {
        fraction / 10u64.pow(fraction_cnt - need_digit)
    }
}

struct Parser<'a> {
    iter: Chars<'a>,
    src: &'a str,
    current: Bandwidth,
}

impl<'a> Parser<'a> {
    fn new<'b: 'a>(s: &'b str) -> Self {
        Parser {
            iter: s.chars(),
            src: s,
            current: Bandwidth::new(0, 0),
        }
    }
}

impl Parser<'_> {
    fn off(&self) -> usize {
        self.src.len() - self.iter.as_str().len()
    }

    fn parse_first_char(&mut self) -> Result<Option<u64>, Error> {
        let off = self.off();
        for c in self.iter.by_ref() {
            match c {
                '0'..='9' => {
                    return Ok(Some(c as u64 - '0' as u64));
                }
                c if c.is_whitespace() => continue,
                _ => {
                    return Err(Error::NumberExpected(off));
                }
            }
        }
        Ok(None)
    }

    fn parse_unit(
        &mut self,
        n: u64,
        fraction: u64,
        fraction_cnt: u32,
        start: usize,
        end: usize,
    ) -> Result<(), Error> {
        let (gbps, bps) = match &self.src[start..end] {
            "bps" | "bit/s" | "b/s" => (0u64, n),
            "kbps" | "Kbps" | "kbit/s" | "Kbit/s" | "kb/s" | "Kb/s" => (
                0u64,
                n.mul(1000)?
                    .add(parse_fraction(fraction, fraction_cnt, 3))?,
            ),
            "Mbps" | "mbps" | "Mbit/s" | "mbit/s" | "Mb/s" | "mb/s" => (
                0u64,
                n.mul(1_000_000)?
                    .add(parse_fraction(fraction, fraction_cnt, 6))?,
            ),
            "Gbps" | "gbps" | "Gbit/s" | "gbit/s" | "Gb/s" | "gb/s" => {
                (n, parse_fraction(fraction, fraction_cnt, 9))
            }
            "Tbps" | "tbps" | "Tbit/s" | "tbit/s" | "Tb/s" | "tb/s" => {
                let bps = parse_fraction(fraction, fraction_cnt, 12);
                (n.mul(1000)?.add(bps / 1_000_000_000)?, bps % 1_000_000_000)
            }
            _ => {
                return Err(Error::UnknownUnit {
                    start,
                    end,
                    unit: self.src[start..end].to_string(),
                    value: n,
                });
            }
        };
        let (gbps, bps) = (gbps + (bps / 1_000_000_000), (bps % 1_000_000_000) as u32);
        let new_bandwidth = Bandwidth::new(gbps, bps);
        self.current = self
            .current
            .checked_add(new_bandwidth)
            .ok_or(Error::NumberOverflow)?;
        Ok(())
    }

    fn parse(mut self) -> Result<Bandwidth, Error> {
        let mut n = self.parse_first_char()?.ok_or(Error::Empty)?;
        let mut decimal = false;
        let mut fraction: u64 = 0;
        let mut fraction_cnt: u32 = 0;
        'outer: loop {
            let mut off = self.off();
            while let Some(c) = self.iter.next() {
                match c {
                    '0'..='9' => {
                        if decimal {
                            if fraction_cnt >= FRACTION_PART_LIMIT {
                                continue;
                            }
                            fraction = fraction
                                .checked_mul(10)
                                .and_then(|x| x.checked_add(c as u64 - '0' as u64))
                                .ok_or(Error::NumberOverflow)?;
                            fraction_cnt += 1;
                        } else {
                            n = n
                                .checked_mul(10)
                                .and_then(|x| x.checked_add(c as u64 - '0' as u64))
                                .ok_or(Error::NumberOverflow)?;
                        }
                    }
                    c if c.is_whitespace() => {}
                    '_' => {}
                    '.' => {
                        if decimal {
                            return Err(Error::InvalidCharacter(off));
                        }
                        decimal = true;
                    }
                    'a'..='z' | 'A'..='Z' | '/' => {
                        break;
                    }
                    _ => {
                        return Err(Error::InvalidCharacter(off));
                    }
                }
                off = self.off();
            }
            let start = off;
            let mut off = self.off();
            while let Some(c) = self.iter.next() {
                match c {
                    '0'..='9' => {
                        self.parse_unit(n, fraction, fraction_cnt, start, off)?;
                        n = c as u64 - '0' as u64;
                        fraction = 0;
                        decimal = false;
                        fraction_cnt = 0;
                        continue 'outer;
                    }
                    c if c.is_whitespace() => break,
                    'a'..='z' | 'A'..='Z' | '/' => {}
                    _ => {
                        return Err(Error::InvalidCharacter(off));
                    }
                }
                off = self.off();
            }
            self.parse_unit(n, fraction, fraction_cnt, start, off)?;
            n = match self.parse_first_char()? {
                Some(n) => n,
                None => return Ok(self.current),
            };
            fraction = 0;
            decimal = false;
            fraction_cnt = 0;
        }
    }
}

/// Parse bandwidth object `1Gbps 12Mbps 5bps` or `1.012000005Gbps`
///
/// The bandwidth object is a concatenation of rate spans. Where each rate
/// span is an number and a suffix. Supported suffixes:
///
/// * `bps`, `bit/s`, `b/s` -- bit per second
/// * `kbps`, `kbit/s`, `kb/s` -- kilobit per second
/// * `Mbps`, `Mbit/s`, `Mb/s` -- megabit per second
/// * `Gbps`, `Gbit/s`, `Gb/s` -- gigabit per second
/// * `Tbps`, `Tbit/s`, `Tb/s` -- terabit per second
///
/// While the number can be integer or decimal, the fractional part less than 1bps will always be
/// ignored.
///
/// # Examples
///
/// ```
/// use bandwidth::Bandwidth;
/// use human_bandwidth::parse_bandwidth;
///
/// assert_eq!(parse_bandwidth("9Tbps 420Gbps"), Ok(Bandwidth::new(9420, 0)));
/// assert_eq!(parse_bandwidth("32Mbps"), Ok(Bandwidth::new(0, 32_000_000)));
/// assert_eq!(parse_bandwidth("150.024kbps"), Ok(Bandwidth::new(0, 150_024)));
/// // The fractional part less than 1bps will always be ignored
/// assert_eq!(parse_bandwidth("150.02456kbps"), Ok(Bandwidth::new(0, 150_024)));
/// ```
pub fn parse_bandwidth(s: &str) -> Result<Bandwidth, Error> {
    Parser::new(s).parse()
}

/// Formats bandwidth into a human-readable string
///
/// Note: this format is guaranteed to have same value when using
/// parse_bandwidth, but we can change some details of the exact composition
/// of the value.
///
/// By default it will format the value with the largest possible unit in decimal form.
/// If you want to display integer values only, enable the `display-integer` feature.
///
/// # Examples
///
/// ```
/// use bandwidth::Bandwidth;
/// use human_bandwidth::format_bandwidth;
///
/// // Enabling the `display-integer` feature will display integer values only
/// # #[cfg(feature = "display-integer")]
/// # {
/// let val1 = Bandwidth::new(9420, 0);
/// assert_eq!(format_bandwidth(val1).to_string(), "9Tbps 420Gbps");
/// let val2 = Bandwidth::new(0, 32_000_000);
/// assert_eq!(format_bandwidth(val2).to_string(), "32Mbps");
/// # }
///
/// // Disabling the `display-integer` feature will display decimal values
/// # #[cfg(not(feature = "display-integer"))]
/// # {
/// let val1 = Bandwidth::new(9420, 0);
/// assert_eq!(format_bandwidth(val1).to_string(), "9.42Tbps");
/// let val2 = Bandwidth::new(0, 32_000_000);
/// assert_eq!(format_bandwidth(val2).to_string(), "32Mbps");
/// # }
/// ```
pub fn format_bandwidth(val: Bandwidth) -> FormattedBandwidth {
    FormattedBandwidth(val)
}

fn item(f: &mut fmt::Formatter, started: &mut bool, name: &str, value: u32) -> fmt::Result {
    if value > 0 {
        if *started {
            f.write_str(" ")?;
        }
        write!(f, "{}{}", value, name)?;
        *started = true;
    }
    Ok(())
}

#[derive(Copy, Clone)]
#[repr(usize)]
enum LargestUnit {
    Bps = 0,
    Kbps = 1,
    Mbps = 2,
    Gbps = 3,
    Tbps = 4,
}

impl fmt::Display for LargestUnit {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            LargestUnit::Bps => f.write_str("bps"),
            LargestUnit::Kbps => f.write_str("kbps"),
            LargestUnit::Mbps => f.write_str("Mbps"),
            LargestUnit::Gbps => f.write_str("Gbps"),
            LargestUnit::Tbps => f.write_str("Tbps"),
        }
    }
}

impl FormattedBandwidth {
    #[deprecated(since = "0.1.4", note = "please use `core::ops::Deref` instead")]
    /// Returns a reference to the [`Bandwidth`][] that is being formatted.
    pub fn get_ref(&self) -> &Bandwidth {
        &self.0
    }

    /// Enabling the `display-integer` feature will display integer values only
    ///
    /// This method is preserved for backward compatibility and custom formatting.
    pub fn fmt_integer(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let gbps = self.0.as_gbps();
        let bps = self.0.subgbps_bps();

        if gbps == 0 && bps == 0 {
            f.write_str("0bps")?;
            return Ok(());
        }

        let tbps = gbps / 1_000;
        let gbps = gbps % 1_000;

        let mbps = bps / 1_000_000;
        let kbps = bps / 1_000 % 1_000;
        let bps = bps % 1_000;

        let started = &mut false;
        item(f, started, "Tbps", tbps as u32)?;
        item(f, started, "Gbps", gbps as u32)?;
        item(f, started, "Mbps", mbps)?;
        item(f, started, "kbps", kbps)?;
        item(f, started, "bps", bps)?;
        Ok(())
    }

    /// Disabling the `display-integer` feature will display decimal values
    ///
    /// This method is preserved for custom formatting.
    pub fn fmt_decimal(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let gbps = self.0.as_gbps();
        let bps = self.0.subgbps_bps();

        if gbps == 0 && bps == 0 {
            f.write_str("0bps")?;
            return Ok(());
        }

        let tbps = gbps / 1_000;
        let gbps = gbps % 1_000;

        let mbps = (bps / 1_000_000) as u64;
        let kbps = (bps / 1_000 % 1_000) as u64;
        let bps = (bps % 1_000) as u64;

        let largest_unit = if tbps > 0 {
            LargestUnit::Tbps
        } else if gbps > 0 {
            LargestUnit::Gbps
        } else if mbps > 0 {
            LargestUnit::Mbps
        } else if kbps > 0 {
            LargestUnit::Kbps
        } else {
            LargestUnit::Bps
        };

        let values = [bps, kbps, mbps, gbps, tbps];
        let mut index = largest_unit as usize;
        let mut zeros = 0;
        let mut dot = true;
        write!(f, "{}", values[index])?;
        loop {
            if index == 0 {
                write!(f, "{}", largest_unit)?;
                break;
            }
            index -= 1;
            let value = values[index];
            if value == 0 {
                zeros += 3;
                continue;
            } else {
                if dot {
                    f.write_str(".")?;
                    dot = false;
                }
                if zeros > 0 {
                    write!(f, "{:0width$}", 0, width = zeros)?;
                    zeros = 0;
                }
                if value % 10 != 0 {
                    write!(f, "{:03}", value)?;
                } else if value % 100 != 0 {
                    write!(f, "{:02}", value / 10)?;
                    zeros += 1;
                } else {
                    write!(f, "{}", value / 100)?;
                    zeros += 2;
                }
            }
        }
        Ok(())
    }
}

impl fmt::Display for FormattedBandwidth {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        #[cfg(not(feature = "display-integer"))]
        self.fmt_decimal(f)?;
        #[cfg(feature = "display-integer")]
        self.fmt_integer(f)?;
        Ok(())
    }
}

impl core::ops::Deref for FormattedBandwidth {
    type Target = Bandwidth;

    fn deref(&self) -> &Bandwidth {
        &self.0
    }
}

impl core::ops::DerefMut for FormattedBandwidth {
    fn deref_mut(&mut self) -> &mut Bandwidth {
        &mut self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bandwidth::Bandwidth;

    #[test]
    fn test_units() {
        assert_eq!(parse_bandwidth("1bps"), Ok(Bandwidth::new(0, 1)));
        assert_eq!(parse_bandwidth("2bit/s"), Ok(Bandwidth::new(0, 2)));
        assert_eq!(parse_bandwidth("15b/s"), Ok(Bandwidth::new(0, 15)));
        assert_eq!(parse_bandwidth("51kbps"), Ok(Bandwidth::new(0, 51_000)));
        assert_eq!(parse_bandwidth("79Kbps"), Ok(Bandwidth::new(0, 79_000)));
        assert_eq!(parse_bandwidth("81kbit/s"), Ok(Bandwidth::new(0, 81_000)));
        assert_eq!(parse_bandwidth("100Kbit/s"), Ok(Bandwidth::new(0, 100_000)));
        assert_eq!(parse_bandwidth("150kb/s"), Ok(Bandwidth::new(0, 150_000)));
        assert_eq!(parse_bandwidth("410Kb/s"), Ok(Bandwidth::new(0, 410_000)));
        assert_eq!(parse_bandwidth("12Mbps"), Ok(Bandwidth::new(0, 12_000_000)));
        assert_eq!(parse_bandwidth("16mbps"), Ok(Bandwidth::new(0, 16_000_000)));
        assert_eq!(
            parse_bandwidth("24Mbit/s"),
            Ok(Bandwidth::new(0, 24_000_000))
        );
        assert_eq!(
            parse_bandwidth("36mbit/s"),
            Ok(Bandwidth::new(0, 36_000_000))
        );
        assert_eq!(parse_bandwidth("48Mb/s"), Ok(Bandwidth::new(0, 48_000_000)));
        assert_eq!(parse_bandwidth("96mb/s"), Ok(Bandwidth::new(0, 96_000_000)));
        assert_eq!(parse_bandwidth("2Gbps"), Ok(Bandwidth::new(2, 0)));
        assert_eq!(parse_bandwidth("4gbps"), Ok(Bandwidth::new(4, 0)));
        assert_eq!(parse_bandwidth("6Gbit/s"), Ok(Bandwidth::new(6, 0)));
        assert_eq!(parse_bandwidth("8gbit/s"), Ok(Bandwidth::new(8, 0)));
        assert_eq!(parse_bandwidth("16Gb/s"), Ok(Bandwidth::new(16, 0)));
        assert_eq!(parse_bandwidth("40gb/s"), Ok(Bandwidth::new(40, 0)));
        assert_eq!(parse_bandwidth("1Tbps"), Ok(Bandwidth::new(1_000, 0)));
        assert_eq!(parse_bandwidth("2tbps"), Ok(Bandwidth::new(2_000, 0)));
        assert_eq!(parse_bandwidth("4Tbit/s"), Ok(Bandwidth::new(4_000, 0)));
        assert_eq!(parse_bandwidth("8tbit/s"), Ok(Bandwidth::new(8_000, 0)));
        assert_eq!(parse_bandwidth("16Tb/s"), Ok(Bandwidth::new(16_000, 0)));
        assert_eq!(parse_bandwidth("32tb/s"), Ok(Bandwidth::new(32_000, 0)));
    }

    #[test]
    fn test_decimal() {
        assert_eq!(parse_bandwidth("1.5bps"), Ok(Bandwidth::new(0, 1)));
        assert_eq!(parse_bandwidth("2.5bit/s"), Ok(Bandwidth::new(0, 2)));
        assert_eq!(parse_bandwidth("15.5b/s"), Ok(Bandwidth::new(0, 15)));
        assert_eq!(parse_bandwidth("51.6kbps"), Ok(Bandwidth::new(0, 51_600)));
        assert_eq!(parse_bandwidth("79.78Kbps"), Ok(Bandwidth::new(0, 79_780)));
        assert_eq!(
            parse_bandwidth("81.923kbit/s"),
            Ok(Bandwidth::new(0, 81_923))
        );
        assert_eq!(
            parse_bandwidth("100.1234Kbit/s"),
            Ok(Bandwidth::new(0, 100_123))
        );
        assert_eq!(
            parse_bandwidth("150.12345kb/s"),
            Ok(Bandwidth::new(0, 150_123))
        );
        assert_eq!(
            parse_bandwidth("410.123456Kb/s"),
            Ok(Bandwidth::new(0, 410_123))
        );
        assert_eq!(
            parse_bandwidth("12.123Mbps"),
            Ok(Bandwidth::new(0, 12_123_000))
        );
        assert_eq!(
            parse_bandwidth("16.1234mbps"),
            Ok(Bandwidth::new(0, 16_123_400))
        );
        assert_eq!(
            parse_bandwidth("24.12345Mbit/s"),
            Ok(Bandwidth::new(0, 24_123_450))
        );
        assert_eq!(
            parse_bandwidth("36.123456mbit/s"),
            Ok(Bandwidth::new(0, 36_123_456))
        );
        assert_eq!(
            parse_bandwidth("48.123Mb/s"),
            Ok(Bandwidth::new(0, 48_123_000))
        );
        assert_eq!(
            parse_bandwidth("96.1234mb/s"),
            Ok(Bandwidth::new(0, 96_123_400))
        );
        assert_eq!(
            parse_bandwidth("2.123Gbps"),
            Ok(Bandwidth::new(2, 123_000_000))
        );
        assert_eq!(
            parse_bandwidth("4.1234gbps"),
            Ok(Bandwidth::new(4, 123_400_000))
        );
        assert_eq!(
            parse_bandwidth("6.12345Gbit/s"),
            Ok(Bandwidth::new(6, 123_450_000))
        );
        assert_eq!(
            parse_bandwidth("8.123456gbit/s"),
            Ok(Bandwidth::new(8, 123_456_000))
        );
        assert_eq!(
            parse_bandwidth("16.123456789Gb/s"),
            Ok(Bandwidth::new(16, 123_456_789))
        );
        assert_eq!(
            parse_bandwidth("40.12345678912gb/s"),
            Ok(Bandwidth::new(40, 123_456_789))
        );
        assert_eq!(parse_bandwidth("1.123Tbps"), Ok(Bandwidth::new(1_123, 0)));
        assert_eq!(
            parse_bandwidth("2.1234tbps"),
            Ok(Bandwidth::new(2_123, 400_000_000))
        );
        assert_eq!(
            parse_bandwidth("4.12345Tbit/s"),
            Ok(Bandwidth::new(4_123, 450_000_000))
        );
        assert_eq!(
            parse_bandwidth("8.123456tbit/s"),
            Ok(Bandwidth::new(8_123, 456_000_000))
        );
        assert_eq!(
            parse_bandwidth("16.123456789Tb/s"),
            Ok(Bandwidth::new(16_123, 456_789_000))
        );
        assert_eq!(
            parse_bandwidth("32.12345678912tb/s"),
            Ok(Bandwidth::new(32_123, 456_789_120))
        );
    }

    #[test]
    fn test_combo() {
        assert_eq!(
            parse_bandwidth("1bps 2bit/s 3b/s"),
            Ok(Bandwidth::new(0, 6))
        );
        assert_eq!(
            parse_bandwidth("4kbps 5Kbps 6kbit/s"),
            Ok(Bandwidth::new(0, 15_000))
        );
        assert_eq!(
            parse_bandwidth("7Mbps 8mbps 9Mbit/s"),
            Ok(Bandwidth::new(0, 24_000_000))
        );
        assert_eq!(
            parse_bandwidth("10Gbps 11gbps 12Gbit/s"),
            Ok(Bandwidth::new(33, 0))
        );
        assert_eq!(
            parse_bandwidth("13Tbps 14tbps 15Tbit/s"),
            Ok(Bandwidth::new(42_000, 0))
        );
        assert_eq!(
            parse_bandwidth("10Gbps 5Mbps 1b/s"),
            Ok(Bandwidth::new(10, 5_000_001))
        );
        assert_eq!(
            parse_bandwidth("36Mbps 12kbps 24bps"),
            Ok(Bandwidth::new(0, 36_012_024))
        );
    }

    #[test]
    fn test_decimal_combo() {
        assert_eq!(
            parse_bandwidth("1.1bps 2.2bit/s 3.3b/s"),
            Ok(Bandwidth::new(0, 6))
        );
        assert_eq!(
            parse_bandwidth("4.4kbps 5.5Kbps 6.6kbit/s"),
            Ok(Bandwidth::new(0, 16_500))
        );
        assert_eq!(
            parse_bandwidth("7.7Mbps 8.8mbps 9.9Mbit/s"),
            Ok(Bandwidth::new(0, 26_400_000))
        );
        assert_eq!(
            parse_bandwidth("10.10Gbps 11.11gbps 12.12Gbit/s"),
            Ok(Bandwidth::new(33, 330_000_000))
        );
        assert_eq!(
            parse_bandwidth("13.13Tbps 14.14tbps 15.15Tbit/s"),
            Ok(Bandwidth::new(42_420, 0))
        );
        assert_eq!(
            parse_bandwidth("10.1Gbps 5.2Mbps 1.3b/s"),
            Ok(Bandwidth::new(10, 105_200_001))
        );
        assert_eq!(
            parse_bandwidth("36.1Mbps 12.2kbps 24.3bps"),
            Ok(Bandwidth::new(0, 36_112_224))
        );
    }

    #[test]
    fn test_overflow() {
        assert_eq!(
            parse_bandwidth("100000000000000000000bps"),
            Err(Error::NumberOverflow)
        );
        assert_eq!(
            parse_bandwidth("100000000000000000kbps"),
            Err(Error::NumberOverflow)
        );
        assert_eq!(
            parse_bandwidth("100000000000000Mbps"),
            Err(Error::NumberOverflow)
        );
        assert_eq!(
            parse_bandwidth("100000000000000000000Gbps"),
            Err(Error::NumberOverflow)
        );
        assert_eq!(
            parse_bandwidth("10000000000000000000Tbps"),
            Err(Error::NumberOverflow)
        );
    }

    #[test]
    fn test_nice_error_message() {
        assert_eq!(
            parse_bandwidth("123").unwrap_err().to_string(),
            "bandwidth unit needed, for example 123Mbps or 123bps"
        );
        assert_eq!(
            parse_bandwidth("10 Gbps 1").unwrap_err().to_string(),
            "bandwidth unit needed, for example 1Mbps or 1bps"
        );
        assert_eq!(
            parse_bandwidth("10 byte/s").unwrap_err().to_string(),
            "unknown bandwidth unit \"byte/s\", \
                    supported units: bps, kbps, Mbps, Gbps, Tbps"
        );
    }

    #[test]
    fn test_formatted_bandwidth_integer() {
        struct TestInteger(FormattedBandwidth);
        impl From<FormattedBandwidth> for TestInteger {
            fn from(fb: FormattedBandwidth) -> Self {
                TestInteger(fb)
            }
        }
        impl fmt::Display for TestInteger {
            fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
                self.0.fmt_integer(f)
            }
        }
        assert_eq!(
            TestInteger::from(format_bandwidth(Bandwidth::new(0, 0))).to_string(),
            "0bps"
        );
        assert_eq!(
            TestInteger::from(format_bandwidth(Bandwidth::new(0, 1))).to_string(),
            "1bps"
        );
        assert_eq!(
            TestInteger::from(format_bandwidth(Bandwidth::new(0, 15))).to_string(),
            "15bps"
        );
        assert_eq!(
            TestInteger::from(format_bandwidth(Bandwidth::new(0, 51_000))).to_string(),
            "51kbps"
        );
        assert_eq!(
            TestInteger::from(format_bandwidth(Bandwidth::new(0, 32_000_000))).to_string(),
            "32Mbps"
        );
        assert_eq!(
            TestInteger::from(format_bandwidth(Bandwidth::new(0, 79_000_000))).to_string(),
            "79Mbps"
        );
        assert_eq!(
            TestInteger::from(format_bandwidth(Bandwidth::new(0, 100_000_000))).to_string(),
            "100Mbps"
        );
        assert_eq!(
            TestInteger::from(format_bandwidth(Bandwidth::new(0, 150_000_000))).to_string(),
            "150Mbps"
        );
        assert_eq!(
            TestInteger::from(format_bandwidth(Bandwidth::new(0, 410_000_000))).to_string(),
            "410Mbps"
        );
        assert_eq!(
            TestInteger::from(format_bandwidth(Bandwidth::new(1, 0))).to_string(),
            "1Gbps"
        );
        assert_eq!(
            TestInteger::from(format_bandwidth(Bandwidth::new(4, 500_000_000))).to_string(),
            "4Gbps 500Mbps"
        );
        assert_eq!(
            TestInteger::from(format_bandwidth(Bandwidth::new(9420, 0))).to_string(),
            "9Tbps 420Gbps"
        );
    }

    #[test]
    fn test_formatted_bandwidth_decimal() {
        struct TestDecimal(FormattedBandwidth);
        impl From<FormattedBandwidth> for TestDecimal {
            fn from(fb: FormattedBandwidth) -> Self {
                TestDecimal(fb)
            }
        }
        impl fmt::Display for TestDecimal {
            fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
                self.0.fmt_decimal(f)
            }
        }
        assert_eq!(
            TestDecimal::from(format_bandwidth(Bandwidth::new(0, 0))).to_string(),
            "0bps"
        );
        assert_eq!(
            TestDecimal::from(format_bandwidth(Bandwidth::new(0, 1))).to_string(),
            "1bps"
        );
        assert_eq!(
            TestDecimal::from(format_bandwidth(Bandwidth::new(0, 15))).to_string(),
            "15bps"
        );
        assert_eq!(
            TestDecimal::from(format_bandwidth(Bandwidth::new(0, 51_200))).to_string(),
            "51.2kbps"
        );
        assert_eq!(
            TestDecimal::from(format_bandwidth(Bandwidth::new(0, 32_300_400))).to_string(),
            "32.3004Mbps"
        );
        assert_eq!(
            TestDecimal::from(format_bandwidth(Bandwidth::new(0, 79_000_050))).to_string(),
            "79.00005Mbps"
        );
        assert_eq!(
            TestDecimal::from(format_bandwidth(Bandwidth::new(0, 100_060_007))).to_string(),
            "100.060007Mbps"
        );
        assert_eq!(
            TestDecimal::from(format_bandwidth(Bandwidth::new(0, 150_000_000))).to_string(),
            "150Mbps"
        );
        assert_eq!(
            TestDecimal::from(format_bandwidth(Bandwidth::new(0, 410_008_900))).to_string(),
            "410.0089Mbps"
        );
        assert_eq!(
            TestDecimal::from(format_bandwidth(Bandwidth::new(1, 0))).to_string(),
            "1Gbps"
        );
        assert_eq!(
            TestDecimal::from(format_bandwidth(Bandwidth::new(4, 500_000_000))).to_string(),
            "4.5Gbps"
        );
        assert_eq!(
            TestDecimal::from(format_bandwidth(Bandwidth::new(8700, 32_000_000))).to_string(),
            "8.700032Tbps"
        );
        assert_eq!(
            "9.42Tbps",
            TestDecimal::from(format_bandwidth(Bandwidth::new(9420, 0))).to_string(),
        );
    }
}
