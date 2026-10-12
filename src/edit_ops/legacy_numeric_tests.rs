use super::*;

fn decimal(text: &str, separator: char) -> Result<Option<f64>, DecimalError> {
    decimal_key(&text.encode_utf16().collect::<Vec<_>>(), separator as u16, &AtomicBool::new(false))
}

#[test]
fn decimal_admission_prefix_and_signed_zero() {
    for text in ["", " \t\r\n", "words", "+1", "\u{b}1", "\u{c}1", "\u{85}1"] {
        assert!(matches!(decimal(text, '.'), Ok(None)), "{text:?}");
    }
    for (text, number) in [("1.2.3",1.2), ("1 2",1.), ("12-3",12.), ("1e3",1.), ("-.5",-0.5), ("0001.50",1.5)] {
        assert!(matches!(decimal(text, '.'), Ok(Some(value)) if value == number), "{text:?}");
    }
    assert!(matches!(decimal(" 1,25,2", ','), Ok(Some(1.25))));
    assert!(matches!(decimal("-0.000", '.'), Ok(Some(value)) if value.to_bits() == (-0.0f64).to_bits()));
    for text in ["--1", ".", "-.", "-", " - "] {
        assert!(matches!(decimal(text, '.'), Err(DecimalError::Invalid)), "{text:?}");
    }
}

#[test]
fn decimal_default_nearest_even_sdk_vectors() {
    let vectors: serde_json::Value = serde_json::from_str(include_str!("legacy_decimal_vectors.json")).unwrap();
    for vector in vectors["vectors"].as_array().unwrap() {
        let id = vector["id"].as_str().unwrap(); let text = vector["input"].as_str().unwrap();
        let result = decimal(text, '.');
        if vector["source_inferred_status"] == "OK" {
            let bits = u64::from_str_radix(vector["source_inferred_bits"].as_str().unwrap(),16).unwrap();
            assert!(matches!(result, Ok(Some(value)) if value.to_bits() == bits), "{id}");
        } else { assert!(matches!(result, Err(DecimalError::Invalid)), "{id}"); }
    }
}

fn natural(a: &str, b: &str) -> Ordering {
    natural_compare(&a.encode_utf16().collect::<Vec<_>>(), &b.encode_utf16().collect::<Vec<_>>(), &AtomicBool::new(false)).unwrap()
}

#[test]
fn natural_arbitrary_magnitude_zero_padding_and_utf16_tails() {
    for (a,b) in [("-2","1"), ("001","01"), ("01","1"), ("-01","-1"), ("-9","-2"),
        ("x9y2","x10y1"), ("0-","0-0"), ("\u{10000}","\u{e000}"), ("1\u{10000}","1\u{e000}")] {
        assert_eq!(natural(a,b),Ordering::Less,"{a:?} {b:?}"); assert_eq!(natural(b,a),Ordering::Greater);
    }
    assert_eq!(natural(&format!("1{}", "0".repeat(12000)), &"9".repeat(12000)), Ordering::Greater);
    assert_eq!(natural("--", "--"),Ordering::Equal);
}

#[test]
fn natural_non_total_prefix_cycle_is_explicit() {
    for prefix in ["", "0"] {
        assert_eq!(natural(&format!("{prefix}-."), &format!("{prefix}.")),Ordering::Equal);
        assert_eq!(natural(&format!("{prefix}-."), &format!("{prefix}-1")),Ordering::Less);
        assert_eq!(natural(&format!("{prefix}-1"), &format!("{prefix}.")),Ordering::Less);
    }
}

#[test]
fn cancelled_numeric_scans_return_before_comparison_or_conversion() {
    let cancel = AtomicBool::new(true); let key = vec![48u16;20000];
    assert!(natural_compare(&key,&key,&cancel).is_err());
    assert!(matches!(decimal_key(&key,46,&cancel),Err(DecimalError::Refused(_))));
}
