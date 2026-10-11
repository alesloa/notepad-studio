use super::*;

fn replace(pattern: &str, text: &str, value: &str) -> Result<String, String> {
    let regex = Regex::new(pattern).unwrap();
    let template = Template::parse(value, &regex)?;
    let edits = template.plan(&regex, text, None, 4096)?;
    apply_to_string(text, &edits)
}

#[test]
fn numbered_named_optional_and_whole_match_references() {
    assert_eq!(
        replace(r"(?<word>\pL+)(\d)?", "é1 中", r"$+{word}/${2}/$0/$&"),
        Ok("é/1/é1/é1 中//中/中".into())
    );
    assert_eq!(
        replace("(?<a>a)(b)", "ab", r"$a${a}$1\1${2}"),
        Ok("aaaab".into())
    );
    assert_eq!(
        replace("(a)(b)(c)(d)(e)(f)(g)(h)(i)(j)", "abcdefghij", r"${10}\12"),
        Ok("ja2".into())
    );
}

#[test]
fn literal_escaping_and_control_characters_are_not_reparsed() {
    assert_eq!(
        replace("(a)", "a", r"\($1\)\?\:\\$$\$\n\t"),
        Ok("(a)?:\\$$\n\t".into())
    );
    assert_eq!(replace("(.+)", "$1", "$1"), Ok("$1".into()));
}

#[test]
fn literal_b_and_numeric_escapes_use_replacement_rules() {
    assert_eq!(replace("(a)", "a", r"\b$0"), Ok("ba".into()));
    for value in [r"\0", r"\0101", r"\000"] {
        let error = replace("(a)", "a", value).unwrap_err();
        assert!(error.contains("unsupported"));
        assert!(error.contains("$0"));
    }
}

#[test]
fn unknown_optional_and_invalid_references_are_distinct() {
    assert_eq!(replace("(a)(b)?", "a", "[$2]"), Ok("[]".into()));
    for value in [
        "$2",
        "${unknown}",
        "$+{missing}",
        "${",
        "${}",
        "${99999999999999999999999999}",
    ] {
        assert!(replace("(a)", "a", value).is_err(), "{value}");
    }
}

#[test]
fn unsupported_operators_refuse_instead_of_inserting_the_expression() {
    for value in [
        r"\U$1\E",
        r"\l$1",
        "$`",
        "$'",
        "$+",
        "$^N",
        "${^MATCH}",
        "?1yes:no",
        "($1)",
        r"\x41",
    ] {
        assert!(
            replace("(a)", "a", value)
                .unwrap_err()
                .contains("unsupported"),
            "{value}"
        );
    }
}

#[test]
fn legacy_rust_name_resolution_requires_braces_before_a_suffix() {
    assert!(replace("(a)", "a", "$1suffix").is_err());
    assert_eq!(replace("(a)", "a", "${1}suffix"), Ok("asuffix".into()));
}

#[test]
fn zero_length_regex_matches_progress_by_utf8_boundaries() {
    assert_eq!(replace("", "é中", ">"), Ok(">é>中>".into()));
    assert_eq!(replace("(?m)^", "é\r\n中", ">"), Ok(">é\r\n>中".into()));
    assert_eq!(replace("a*", "b", "x"), Ok("xbx".into()));
}

#[test]
fn selection_is_a_filter_on_full_match_context() {
    let regex = Regex::new(r"\b(é)\b").unwrap();
    let template = Template::parse("$1$1", &regex).unwrap();
    assert!(template
        .plan(&regex, "aé", Some(&(1..3)), 4096)
        .unwrap()
        .is_empty());
    assert!(template.plan(&regex, "é", Some(&(1..2)), 4096).is_err());
    let regex = Regex::new("(?m)^é").unwrap();
    assert!(Template::parse("x", &regex)
        .unwrap()
        .plan(&regex, "aé", Some(&(1..3)), 4096)
        .unwrap()
        .is_empty());
}

#[test]
fn varying_expansion_lengths_shift_selection_exactly() {
    let regex = Regex::new(r"(\w+)").unwrap();
    let template = Template::parse("$1$1", &regex).unwrap();
    let edits = template
        .plan(&regex, "x a é outside", Some(&(2..6)), 4096)
        .unwrap();
    assert_eq!(shifted_scope(Some(&(2..6)), &edits), Some(2..9));
    assert_eq!(
        apply_to_string("x a é outside", &edits).unwrap(),
        "x aa éé outside"
    );
}

#[test]
fn template_count_and_expansion_limits_are_atomic_errors() {
    let regex = Regex::new("(a)").unwrap();
    assert!(Template::parse(&"x".repeat(MAX_TEMPLATE_BYTES + 1), &regex).is_err());
    let template = Template::parse("$1$1", &regex).unwrap();
    assert!(template.plan(&regex, "aaaa", None, 3).is_err());
    let regex = Regex::new("(.+)").unwrap();
    let template = Template::parse("$1$1", &regex).unwrap();
    assert!(template
        .plan(&regex, &"a".repeat(MAX_EXPANDED_BYTES / 2 + 1), None, 1)
        .is_err());
}

#[test]
fn current_plan_uses_fresh_captures_and_keeps_unchanged_edits_noop() {
    let regex = Regex::new("(a)").unwrap();
    let template = Template::parse("$1", &regex).unwrap();
    let edits = template
        .plan_one(&regex, "b a", None, Some(&(0..1)), 0)
        .unwrap();
    assert_eq!(edits[0].range, 2..3);
    assert!(!edits[0].changed);
}

#[test]
fn malformed_plan_ranges_cannot_panic() {
    let edits = [Replacement {
        range: 1..usize::MAX,
        text: "x".into(),
        changed: true,
    }];
    assert!(apply_to_string("é", &edits).is_err());
}
