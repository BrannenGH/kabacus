// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Exact money: integer mantissas and decimal places, never floating point.

use num_bigint::{BigInt, BigUint};
use num_traits::{Signed, Zero};
use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::ops::{Add, Neg, Sub};

/// The most decimal places an amount can have.
pub const MAX_PLACES: u32 = 255;

/// An exact decimal amount: `mantissa * 10^-places`. Never a float. The
/// mantissa is an integer of any size.
///
/// Two `Money` values with different `places` compare and combine correctly:
/// arithmetic scales the smaller-precision operand up first, so `1.00` and
/// `0.678` add to `1.678` rather than silently truncating. Equality and
/// hashing are by *value*, not representation: `0` at 2 places equals `0` at
/// 0 places (both compare and hash via [`Money::canonical`]).
#[derive(Debug, Clone)]
pub struct Money {
    mantissa: BigInt,
    places: u32,
}

impl Money {
    pub fn zero() -> Self {
        Money {
            mantissa: BigInt::zero(),
            places: 0,
        }
    }

    /// Build from an integer mantissa and decimal places.
    pub fn from_decimal(mantissa: i128, places: u32) -> Self {
        Money::from_big(BigInt::from(mantissa), places)
    }

    /// [`Money::from_decimal`] for a mantissa of any size.
    pub fn from_big(mantissa: BigInt, places: u32) -> Self {
        Money { mantissa, places }
    }

    pub fn mantissa(&self) -> &BigInt {
        &self.mantissa
    }

    pub fn places(&self) -> u32 {
        self.places
    }

    pub fn is_zero(&self) -> bool {
        self.mantissa.is_zero()
    }

    pub fn is_negative(&self) -> bool {
        self.mantissa.is_negative()
    }

    pub fn is_positive(&self) -> bool {
        self.mantissa.is_positive()
    }

    /// Rescale to at least `places` decimal places, never losing precision
    /// (only ever multiplies the mantissa up).
    pub fn rescaled(&self, places: u32) -> Self {
        if places <= self.places {
            return self.clone();
        }
        Money {
            mantissa: &self.mantissa * BigInt::from(10).pow(places - self.places),
            places,
        }
    }

    fn common_places(a: &Money, b: &Money) -> u32 {
        a.places.max(b.places)
    }

    /// `None` when the sum would need more than [`MAX_PLACES`] places, which
    /// can't happen for operands within the limit.
    pub fn checked_add(&self, other: &Money) -> Option<Money> {
        let places = Self::common_places(self, other);
        if places > MAX_PLACES {
            return None;
        }
        Some(Money {
            mantissa: self.rescaled(places).mantissa + other.rescaled(places).mantissa,
            places,
        })
    }

    pub fn checked_sub(&self, other: &Money) -> Option<Money> {
        self.checked_add(&-other.clone())
    }

    /// The exact product, with trailing zero places dropped.
    pub fn times(&self, other: &Money) -> Money {
        Money {
            mantissa: &self.mantissa * &other.mantissa,
            places: self.places + other.places,
        }
        .normalized()
    }

    pub fn abs_value(&self) -> Money {
        Money {
            mantissa: self.mantissa.abs(),
            places: self.places,
        }
    }

    /// The same value with no trailing zero places.
    pub fn normalized(&self) -> Money {
        let (mantissa, places) = self.canonical();
        Money { mantissa, places }
    }

    /// The minimal `(mantissa, places)` representing this value, with all
    /// trailing zero digits stripped (so `1000` @ 3 places and `1` @ 0
    /// places both canonicalize to `(1, 0)`). The basis for equality,
    /// hashing and ordering across mismatched precisions.
    fn canonical(&self) -> (BigInt, u32) {
        if self.mantissa.is_zero() {
            return (BigInt::zero(), 0);
        }
        let ten = BigInt::from(10);
        let mut m = self.mantissa.clone();
        let mut p = self.places;
        while p > 0 && (&m % &ten).is_zero() {
            m /= &ten;
            p -= 1;
        }
        (m, p)
    }

    /// The magnitude's integer and fractional parts, for formatting. The
    /// sign is not in either (-0.04 gives `(0, 4)`): take it from
    /// [`Money::is_negative`].
    pub fn to_parts(&self) -> (BigUint, BigUint) {
        let magnitude = self.mantissa.magnitude();
        let scale = BigUint::from(10u8).pow(self.places);
        (magnitude / &scale, magnitude % &scale)
    }
}

impl Add for Money {
    type Output = Money;
    fn add(self, rhs: Money) -> Money {
        &self + &rhs
    }
}

impl Add for &Money {
    type Output = Money;
    fn add(self, rhs: &Money) -> Money {
        self.checked_add(rhs).expect("Money addition overflow")
    }
}

impl Add<&Money> for Money {
    type Output = Money;
    fn add(self, rhs: &Money) -> Money {
        &self + rhs
    }
}

impl Add<Money> for &Money {
    type Output = Money;
    fn add(self, rhs: Money) -> Money {
        self + &rhs
    }
}

impl Sub for Money {
    type Output = Money;
    fn sub(self, rhs: Money) -> Money {
        &self - &rhs
    }
}

impl Sub for &Money {
    type Output = Money;
    fn sub(self, rhs: &Money) -> Money {
        self.checked_sub(rhs).expect("Money subtraction overflow")
    }
}

impl Sub<&Money> for Money {
    type Output = Money;
    fn sub(self, rhs: &Money) -> Money {
        &self - rhs
    }
}

impl Sub<Money> for &Money {
    type Output = Money;
    fn sub(self, rhs: Money) -> Money {
        self - &rhs
    }
}

impl Neg for Money {
    type Output = Money;
    fn neg(self) -> Money {
        Money {
            mantissa: -self.mantissa,
            places: self.places,
        }
    }
}

impl Neg for &Money {
    type Output = Money;
    fn neg(self) -> Money {
        -self.clone()
    }
}

impl PartialEq for Money {
    fn eq(&self, other: &Self) -> bool {
        self.canonical() == other.canonical()
    }
}

impl Eq for Money {}

impl Hash for Money {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.canonical().hash(state);
    }
}

impl PartialOrd for Money {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Money {
    fn cmp(&self, other: &Self) -> Ordering {
        let places = Self::common_places(self, other);
        self.rescaled(places)
            .mantissa
            .cmp(&other.rescaled(places).mantissa)
    }
}

impl fmt::Display for Money {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The sign comes from the mantissa, not the whole part: -0.04 has a
        // whole part of 0, which can't carry it.
        let sign = if self.is_negative() { "-" } else { "" };
        let (whole, frac) = self.to_parts();
        if self.places == 0 {
            write!(f, "{sign}{whole}")
        } else {
            write!(
                f,
                "{sign}{whole}.{frac:0width$}",
                width = self.places as usize
            )
        }
    }
}

/// Why a typed amount was refused. `Display` is the message the
/// app and the server both show.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AmountError {
    #[error("Enter an amount")]
    Empty,
    #[error("Not an amount: use digits and a decimal point, like 12.34")]
    NotANumber,
    #[error("Use a decimal point: {suggest}")]
    DecimalComma { suggest: String },
    #[error("{}", too_many_places(*.places))]
    TooManyPlaces { places: u32 },
}

fn too_many_places(places: u32) -> String {
    match places {
        0 => "Use a whole number, without decimal places".to_string(),
        1 => "Use at most 1 decimal place".to_string(),
        n => format!("Use at most {n} decimal places"),
    }
}

/// The one parser for typed amounts ("12", "-4.50",
/// "1,234.56", "$7.25"), straight into mantissa and places, never through
/// a float. At most `places` decimal places (and never more than
/// [`MAX_PLACES`]); any number of digits.
pub fn parse_amount(text: &str, places: u32) -> Result<Money, AmountError> {
    let mut rest = text.trim();
    if rest.is_empty() {
        return Err(AmountError::Empty);
    }
    let (mut negative, mut dollar) = (false, false);
    loop {
        if !negative && rest.starts_with('-') {
            negative = true;
        } else if !dollar && rest.starts_with('$') {
            dollar = true;
        } else {
            break;
        }
        rest = rest[1..].trim_start();
    }
    let (whole, frac) = rest.split_once('.').unwrap_or((rest, ""));
    let digits = |d: &str| d.bytes().all(|b| b.is_ascii_digit());
    if !digits(frac) || (whole.is_empty() && frac.is_empty()) {
        return Err(AmountError::NotANumber);
    }
    let groups: Vec<&str> = whole.split(',').collect();
    if let [int, decimals] = groups[..] {
        if !rest.contains('.') && decimals.len() != 3 && digits(int) && digits(decimals) {
            let sign = if negative { "-" } else { "" };
            return Err(AmountError::DecimalComma {
                suggest: format!("{sign}{int}.{decimals}"),
            });
        }
    }
    let grouped = groups.len() == 1
        || (matches!(groups[0].len(), 1..=3) && groups[1..].iter().all(|g| g.len() == 3));
    if !grouped || !groups.iter().all(|g| digits(g)) {
        return Err(AmountError::NotANumber);
    }
    let places = places.min(MAX_PLACES);
    if frac.len() > places as usize {
        return Err(AmountError::TooManyPlaces { places });
    }
    let all: String = groups.concat() + frac;
    let mantissa: BigInt = if all.is_empty() {
        BigInt::zero()
    } else {
        all.parse().expect("digits only")
    };
    Ok(Money::from_big(
        if negative { -mantissa } else { mantissa },
        frac.len() as u32,
    ))
}

/// [`parse_amount`] without a limit on decimal places.
impl std::str::FromStr for Money {
    type Err = AmountError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        parse_amount(s, u32::MAX)
    }
}

impl std::iter::Sum for Money {
    fn sum<I: Iterator<Item = Money>>(iter: I) -> Money {
        iter.fold(Money::zero(), |a, b| a + b)
    }
}

/// A mantissa on the wire: a JSON integer while it fits an `i64`, a decimal
/// string beyond that, so no JSON reader rounds it through a float. Reading
/// accepts either form.
pub mod wire_mantissa {
    use num_bigint::BigInt;
    use num_traits::ToPrimitive;
    use serde::de::{self, Visitor};
    use serde::{Deserializer, Serializer};
    use std::fmt;

    pub fn serialize<S: Serializer>(mantissa: &BigInt, s: S) -> Result<S::Ok, S::Error> {
        match mantissa.to_i64() {
            Some(n) => s.serialize_i64(n),
            None => s.serialize_str(&mantissa.to_string()),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<BigInt, D::Error> {
        struct Wire;
        impl Visitor<'_> for Wire {
            type Value = BigInt;

            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("an integer, or a string of digits")
            }

            fn visit_i64<E: de::Error>(self, v: i64) -> Result<BigInt, E> {
                Ok(v.into())
            }

            fn visit_u64<E: de::Error>(self, v: u64) -> Result<BigInt, E> {
                Ok(v.into())
            }

            fn visit_i128<E: de::Error>(self, v: i128) -> Result<BigInt, E> {
                Ok(v.into())
            }

            fn visit_u128<E: de::Error>(self, v: u128) -> Result<BigInt, E> {
                Ok(v.into())
            }

            fn visit_str<E: de::Error>(self, v: &str) -> Result<BigInt, E> {
                v.parse().map_err(|_| E::custom("not an integer"))
            }
        }
        d.deserialize_any(Wire)
    }
}

/// A commodity-tagged amount (without display style and
/// cost/price fields we don't need: there's no multi-currency
/// conversion).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Amount {
    pub commodity: String,
    pub money: Money,
}

impl Amount {
    pub fn new(commodity: impl Into<String>, money: Money) -> Self {
        Amount {
            commodity: commodity.into(),
            money,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_typed_amounts_exactly() {
        let parse = |s: &str| s.parse::<Money>().map(|m| m.to_string());
        assert_eq!(parse("12").unwrap(), "12");
        assert_eq!(parse("-4.50").unwrap(), "-4.50");
        assert_eq!(parse("$1,234.567").unwrap(), "1234.567");
        assert_eq!(parse(".5").unwrap(), "0.5");
        assert!(parse("").is_err());
        assert!(parse("12.3.4").is_err());
        assert!(parse("1e5").is_err());
    }
    #[test]
    fn one_grammar_for_typed_amounts_at_the_commodity_precision() {
        use AmountError::*;
        let parse = |s: &str| parse_amount(s, 2).map(|m| m.to_string());
        let ok = |s: &str| Ok(s.to_string());
        assert_eq!(parse("0"), ok("0"));
        assert_eq!(parse("-5"), ok("-5"));
        assert_eq!(parse("abc"), Err(NotANumber));
        assert_eq!(parse("1e3"), Err(NotANumber));
        assert_eq!(parse("12.345"), Err(TooManyPlaces { places: 2 }));
        assert_eq!(parse("12.340"), Err(TooManyPlaces { places: 2 }));
        assert_eq!(parse("1,234.56"), ok("1234.56"));
        assert_eq!(parse("$9.99"), ok("9.99"));
        assert_eq!(parse("-$9.99"), ok("-9.99"));
        assert_eq!(parse("99999999999999999999"), ok("99999999999999999999"));
        assert_eq!(parse("5."), ok("5"));
        assert_eq!(parse("  42.18  "), ok("42.18"));
        assert_eq!(parse(""), Err(Empty));
        assert_eq!(parse("   "), Err(Empty));
        assert_eq!(
            parse("23,40"),
            Err(DecimalComma {
                suggest: "23.40".into()
            })
        );
        assert_eq!(parse("1,234"), ok("1234"));
        assert_eq!(parse("12,34,567"), Err(NotANumber));
        assert_eq!(parse("1,2345.00"), Err(NotANumber));
        assert_eq!(parse("$$5"), Err(NotANumber));
        assert_eq!(parse("--5"), Err(NotANumber));
        assert_eq!(parse("1.2.3"), Err(NotANumber));
        assert_eq!(parse_amount("5.5", 0), Err(TooManyPlaces { places: 0 }));
        assert_eq!(parse("92233720368547758.08"), ok("92233720368547758.08"));
    }
    #[test]
    fn amount_errors_are_plain_sentences() {
        assert_eq!(
            AmountError::TooManyPlaces { places: 2 }.to_string(),
            "Use at most 2 decimal places"
        );
        assert_eq!(
            AmountError::DecimalComma {
                suggest: "23.40".into()
            }
            .to_string(),
            "Use a decimal point: 23.40"
        );
    }
    #[test]
    fn amounts_far_beyond_128_bits_stay_exact() {
        let digits = "9".repeat(60);
        let big: Money = format!("-{digits}.{digits}").parse().unwrap();
        assert_eq!(big.to_string(), format!("-{digits}.{digits}"));
        let sum = big.clone() + Money::from_decimal(1, 60);
        assert_eq!(
            sum.to_string(),
            format!("-{digits}.{}", "9".repeat(59) + "8")
        );
        assert_eq!(
            (sum - big).to_string(),
            format!("0.{}", "0".repeat(59) + "1")
        );
    }

    #[test]
    fn up_to_255_decimal_places() {
        let at_limit = format!("0.{}1", "0".repeat(254));
        let m: Money = at_limit.parse().unwrap();
        assert_eq!(m.places(), 255);
        assert_eq!(m.to_string(), at_limit);
        assert_eq!(
            m.checked_add(&m).unwrap().to_string(),
            format!("0.{}2", "0".repeat(254))
        );
        let past = format!("0.{}1", "0".repeat(255));
        assert_eq!(
            past.parse::<Money>(),
            Err(AmountError::TooManyPlaces { places: 255 })
        );
        assert_eq!(
            Money::from_decimal(1, 255).checked_add(&Money::from_decimal(1, 256)),
            None
        );
    }

    #[test]
    fn wire_mantissa_is_an_integer_until_it_no_longer_fits_an_i64() {
        #[derive(serde::Serialize, serde::Deserialize, PartialEq, Debug)]
        struct Wire(#[serde(with = "wire_mantissa")] BigInt);
        let small = Wire(BigInt::from(-4250));
        assert_eq!(serde_json::to_string(&small).unwrap(), "-4250");
        let big = Wire("123456789012345678901234567890".parse().unwrap());
        assert_eq!(
            serde_json::to_string(&big).unwrap(),
            "\"123456789012345678901234567890\""
        );
        for wire in [small, big] {
            let back: Wire = serde_json::from_str(&serde_json::to_string(&wire).unwrap()).unwrap();
            assert_eq!(back, wire);
        }
        let u64_max: Wire = serde_json::from_str(&u64::MAX.to_string()).unwrap();
        assert_eq!(u64_max.0.to_string(), u64::MAX.to_string());
        // Past 64 bits a bare JSON number has already been rounded by the
        // reader, so only the string form is taken.
        assert!(serde_json::from_str::<Wire>(&u128::MAX.to_string()).is_err());
        assert!(serde_json::from_str::<Wire>("\"12.5\"").is_err());
    }

    #[test]
    fn from_decimal_never_touches_a_float() {
        // 45.678: mantissa 45678, places 3.
        let m = Money::from_decimal(45678, 3);
        assert_eq!(m.to_string(), "45.678");
    }
    #[test]
    fn addition_scales_mismatched_precision_exactly() {
        let a = Money::from_decimal(100000, 2); // 1000.00
        let b = Money::from_decimal(45678, 3); // 45.678
        assert_eq!((a + b).to_string(), "1045.678");
    }
    #[test]
    fn negative_amounts_round_trip() {
        let m = Money::from_decimal(-100000, 2);
        assert!(m.is_negative());
        assert_eq!(m.to_string(), "-1000.00");
    }
    #[test]
    fn negative_amounts_under_one_keep_their_sign() {
        // The whole part of -0.04 is 0, which has no sign of its own.
        assert_eq!(Money::from_decimal(-4, 2).to_string(), "-0.04");
        assert_eq!(Money::from_decimal(-49, 2).to_string(), "-0.49");
        assert_eq!(Money::from_decimal(-1, 2).to_string(), "-0.01");
        assert_eq!(Money::from_decimal(-5, 1).to_string(), "-0.5");
        assert_eq!(Money::from_decimal(4, 2).to_string(), "0.04");
    }
    #[test]
    fn equal_value_at_different_precisions_compares_equal_and_hashes_equal() {
        use std::collections::hash_map::DefaultHasher;
        let a = Money::from_decimal(0, 2);
        let b = Money::from_decimal(0, 0);
        assert_eq!(a, b);
        let c = Money::from_decimal(100, 2); // 1.00
        let d = Money::from_decimal(1, 0); // 1
        assert_eq!(c, d);

        let hash_of = |m: Money| {
            let mut h = DefaultHasher::new();
            m.hash(&mut h);
            h.finish()
        };
        assert_eq!(hash_of(a), hash_of(b));
        assert_eq!(hash_of(c), hash_of(d));
    }

    #[test]
    fn zero_is_zero() {
        assert!(Money::zero().is_zero());
        assert!(!Money::zero().is_positive());
        assert!(!Money::zero().is_negative());
    }

    #[test]
    fn ordering_across_precisions() {
        let a = Money::from_decimal(100, 2); // 1.00
        let b = Money::from_decimal(999, 3); // 0.999
        assert!(a > b);
    }

    proptest::proptest! {
        #[test]
        fn addition_is_commutative(a in -1_000_000_000i64..1_000_000_000, b in -1_000_000_000i64..1_000_000_000, places in 0u32..6) {
            let ma = Money::from_decimal(a as i128, places);
            let mb = Money::from_decimal(b as i128, places);
            proptest::prop_assert_eq!(ma.clone() + mb.clone(), mb + ma);
        }
        #[test]
        fn add_then_subtract_is_identity(a in -1_000_000_000i64..1_000_000_000, b in -1_000_000_000i64..1_000_000_000, places in 0u32..6) {
            let ma = Money::from_decimal(a as i128, places);
            let mb = Money::from_decimal(b as i128, places);
            proptest::prop_assert_eq!(ma.clone() + mb.clone() - mb, ma);
        }
        #[test]
        fn display_round_trips_through_parse(m in -100_000i64..100_000, places in 0u32..6) {
            let money = Money::from_decimal(m as i128, places);
            let back: Money = money.to_string().parse().unwrap();
            proptest::prop_assert_eq!(back.mantissa(), money.mantissa());
            proptest::prop_assert_eq!(back.places(), money.places());
        }
    }
}
