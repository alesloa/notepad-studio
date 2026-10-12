// PowerEditor/src/MISC/Common/Sorters.h:156–633; Common.cpp:926–963.
// Decimal limits: Windows SDK 26100 corecrt_internal_strtox.h:429–443,1504–1713.
use std::{cmp::Ordering, fmt::Write, sync::atomic::AtomicBool};
use super::line_sort::check_cancel;

pub(super) enum DecimalError { Invalid, Refused(&'static str) }
impl From<&'static str> for DecimalError { fn from(message: &'static str) -> Self { Self::Refused(message) } }

fn whitespace(unit: u16) -> bool { matches!(unit, 9 | 10 | 13 | 32) }
fn digit(unit: u16) -> bool { (48..=57).contains(&unit) }

// Only the admitted prefix reaches the CRT. Its decimal scanner accepts a
// valid leading number and ignores later separators, spaces and minus signs.
pub(super) fn decimal_key(key: &[u16], separator: u16, cancel: &AtomicBool) -> Result<Option<f64>, DecimalError> {
    let mut start = 0usize;
    while key.get(start).is_some_and(|&unit| whitespace(unit)) {
        if start.is_multiple_of(8192) { check_cancel(cancel)?; }
        start = start.checked_add(1).ok_or("Decimal key position overflow")?;
    }
    let Some(&first) = key.get(start) else { return Ok(None); };
    if !digit(first) && first != 45 && first != separator { return Ok(None); }
    let negative = first == 45;
    let mut at = start.saturating_add(usize::from(negative));
    let (mut fraction, mut consumed, mut significant, mut exponent) = (false, false, false, 0i64);
    let mut digits = String::new();
    digits.try_reserve_exact(768).map_err(|_| DecimalError::Refused("Not enough memory to prepare decimal keys"))?;
    while let Some(&unit) = key.get(at) {
        if at.is_multiple_of(8192) { check_cancel(cancel)?; }
        if digit(unit) {
            consumed = true;
            if unit != 48 || significant {
                significant = true;
                if !fraction { exponent = exponent.checked_add(1).ok_or("Decimal key exponent overflow")?; }
                if digits.len() < 768 { digits.push(char::from(unit as u8)); }
            } else if fraction {
                exponent = exponent.checked_sub(1).ok_or("Decimal key exponent overflow")?;
            }
        } else if unit == separator && !fraction { fraction = true; }
        else { break; }
        at = at.checked_add(1).ok_or("Decimal key position overflow")?;
    }
    if !consumed { return Err(DecimalError::Invalid); }
    if !significant { return Ok(Some(if negative { -0.0 } else { 0.0 })); }
    while digits.ends_with('0') { digits.pop(); }
    let mut canonical = String::new();
    canonical.try_reserve_exact(800).map_err(|_| DecimalError::Refused("Not enough memory to prepare decimal keys"))?;
    if negative { canonical.push('-'); }
    canonical.push_str("0."); canonical.push_str(&digits);
    write!(&mut canonical, "e{exponent}").map_err(|_| DecimalError::Refused("Could not prepare a decimal key"))?;
    let value = canonical.parse::<f64>().map_err(|_| DecimalError::Invalid)?;
    if !value.is_finite() || value == 0.0 { return Err(DecimalError::Invalid); }
    Ok(Some(value))
}

pub(super) fn compare_units(left: &[u16], right: &[u16], cancel: &AtomicBool) -> Result<Ordering, &'static str> {
    for (index, (a, b)) in left.iter().zip(right).enumerate() {
        if index.is_multiple_of(8192) { check_cancel(cancel)?; }
        let result = a.cmp(b); if result != Ordering::Equal { return Ok(result); }
    }
    Ok(left.len().cmp(&right.len()))
}

struct Number { negative: bool, significant: std::ops::Range<usize>, end: usize, zeros: usize }

fn number(key: &[u16], at: usize, cancel: &AtomicBool) -> Result<Option<Number>, &'static str> {
    let Some(&first) = key.get(at) else { return Ok(None); };
    let negative = first == 45 && at.checked_add(1).and_then(|next| key.get(next)).is_some_and(|&unit| digit(unit));
    if !negative && !digit(first) { return Ok(None); }
    let start = at.checked_add(usize::from(negative)).ok_or("Integer key position overflow")?;
    let mut end = start; let mut significant = start;
    while key.get(end).is_some_and(|&unit| digit(unit)) {
        if end.is_multiple_of(8192) { check_cancel(cancel)?; }
        if significant == end && key.get(end) == Some(&48) { significant = significant.saturating_add(1); }
        end = end.checked_add(1).ok_or("Integer key position overflow")?;
    }
    Ok(Some(Number { negative, significant: significant..end, end, zeros: significant.saturating_sub(start) }))
}

fn text_end(key: &[u16], at: usize, cancel: &AtomicBool) -> Result<(usize, usize), &'static str> {
    let start = at.checked_add(usize::from(key.get(at) == Some(&45))).ok_or("Integer key position overflow")?;
    let mut end = start;
    while key.get(end).is_some_and(|&unit| !digit(unit) && unit != 45) {
        if end.is_multiple_of(8192) { check_cancel(cancel)?; }
        end = end.checked_add(1).ok_or("Integer key position overflow")?;
    }
    Ok((start, end))
}

// Natural comparison is not a total order. A bounded stable merge decides
// permutations deterministically without relying on a sort's order precondition.
pub(super) fn natural_compare(left: &[u16], right: &[u16], cancel: &AtomicBool) -> Result<Ordering, &'static str> {
    let (mut a, mut b) = (0usize, 0usize);
    loop {
        check_cancel(cancel)?;
        let (Some(&unit_a), Some(&unit_b)) = (left.get(a), right.get(b)) else {
            return compare_units(left.get(a..).unwrap_or_default(), right.get(b..).unwrap_or_default(), cancel);
        };
        match (number(left, a, cancel)?, number(right, b, cancel)?) {
            (Some(x), Some(y)) => {
                let mut order = x.negative.cmp(&y.negative).reverse();
                if order == Ordering::Equal {
                    order = x.significant.len().cmp(&y.significant.len());
                    if order == Ordering::Equal {
                        order = compare_units(left.get(x.significant.clone()).ok_or("Invalid integer key")?,
                            right.get(y.significant.clone()).ok_or("Invalid integer key")?, cancel)?;
                    }
                    if x.negative { order = order.reverse(); }
                    if order == Ordering::Equal { order = y.zeros.cmp(&x.zeros); }
                }
                if order != Ordering::Equal { return Ok(order); }
                a = x.end; b = y.end;
            }
            (None, None) => {
                let (from_a, end_a) = text_end(left, a, cancel)?;
                let (from_b, end_b) = text_end(right, b, cancel)?;
                let order = compare_units(left.get(from_a..end_a).ok_or("Invalid integer key")?,
                    right.get(from_b..end_b).ok_or("Invalid integer key")?, cancel)?;
                if order != Ordering::Equal { return Ok(order); }
                a = end_a; b = end_b;
            }
            _ => {
                let order = unit_a.cmp(&unit_b);
                if order != Ordering::Equal { return Ok(order); }
                a = a.checked_add(1).ok_or("Integer key position overflow")?;
                b = b.checked_add(1).ok_or("Integer key position overflow")?;
            }
        }
    }
}

#[cfg(test)]
#[path = "legacy_numeric_tests.rs"]
mod tests;
