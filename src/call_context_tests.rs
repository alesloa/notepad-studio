use super::*;

fn call(text: &str, language: CallLanguage) -> CallContext {
    find_call(text, text.len(), &ApiEnvironment::default(), language).unwrap().unwrap()
}

#[test]
fn counts_only_the_current_call_depth_and_returns_to_its_parent() {
    let inner = call("outer(one, inner([1, 2], {x: [3, 4]}, ", CallLanguage::General);
    assert_eq!((inner.name.as_str(), inner.argument), ("inner", 2));
    let outer = call("outer(one, inner([1, 2], {x: [3, 4]}), ", CallLanguage::General);
    assert_eq!((outer.name.as_str(), outer.argument), ("outer", 2));
    let grouped = call("outer((one, two), [3, 4], ", CallLanguage::General);
    assert_eq!(grouped.argument, 2);
    assert!(find_call("outer(one)", usize::MAX, &ApiEnvironment::default(), CallLanguage::General).unwrap().is_none());
}

#[test]
fn quotes_escapes_and_incomplete_literals_do_not_invent_calls_or_arguments() {
    for text in [r#"outer("fake(a,b)", "escaped\" (,", "#,
        "outer('a,b', `inner(one,two)`, ", "outer(\"unfinished, fake("] {
        let found = call(text, CallLanguage::JavaScript);
        assert_eq!(found.name, "outer", "{text}");
        assert_eq!(found.argument, if text.contains("unfinished") { 0 } else { 2 }, "{text}");
    }
    for text in ["\"fake(a,b)", "// fake(a,b)", "/* fake(a,b)"] {
        assert!(find_call(text, text.len(), &ApiEnvironment::default(), CallLanguage::General).unwrap().is_none(), "{text}");
    }
}

#[test]
fn comments_between_name_and_start_remain_whitespace() {
    for text in ["outer /* (, fake() */ (one, ", "outer // (, fake()\r\n(one, "] {
        let found = call(text, CallLanguage::General);
        assert_eq!((found.name.as_str(), found.argument), ("outer", 1));
        assert_eq!(text.get(found.name_range), Some("outer"));
    }
    let found = call("outer(a, /* nested /* fake(, */ more */ b, ", CallLanguage::Rust);
    assert_eq!(found.argument, 2);
    let found = call("outer(a, // fake(,\r\n b, ", CallLanguage::General);
    assert_eq!(found.argument, 2);
}

#[test]
fn rust_lifetimes_character_literals_and_raw_strings_are_distinct() {
    let found = call("let _: &'a str; outer('中', '\\'', r##\"fake(, )\"##, br#\"a,b\"#, ", CallLanguage::Rust);
    assert_eq!((found.name.as_str(), found.argument), ("outer", 4));
    let found = call("outer(r#\"still inside, fake(", CallLanguage::Rust);
    assert_eq!((found.name.as_str(), found.argument), ("outer", 0));
    let found = call("fn f<'a>(x: &'a str) { outer(x, ", CallLanguage::Rust);
    assert_eq!((found.name.as_str(), found.argument), ("outer", 1));
}

#[test]
fn python_multiline_literals_and_hash_comments_ignore_delimiters() {
    let found = call("outer('''a, fake(\r\nb)''', \"\"\"c, fake(\n)\"\"\", # (,\n ", CallLanguage::Python);
    assert_eq!((found.name.as_str(), found.argument), ("outer", 2));
    let found = call("'''no real call, fake(\n'''\nouter(a, ", CallLanguage::Python);
    assert_eq!((found.name.as_str(), found.argument), ("outer", 1));
}

#[test]
fn sql_doubled_quotes_identifiers_and_comments_do_not_count() {
    let found = call("outer('one''two, fake(', [name]], fake(], \"column\"\", fake(\", -- ,(\r\n ", CallLanguage::Sql);
    assert_eq!((found.name.as_str(), found.argument), ("outer", 3));
    let found = call("outer(a, /* /* , fake( */ ) */ b, ", CallLanguage::Sql);
    assert_eq!(found.argument, 2);
}

#[test]
fn lua_long_strings_and_comments_use_matching_equals() {
    let found = call("outer([==[a, fake( ]=] )]==], --[=[, fake(\n ]=]\n b, ", CallLanguage::Lua);
    assert_eq!((found.name.as_str(), found.argument), ("outer", 2));
    assert!(find_call("--[=[fake(a, b)", usize::MAX, &ApiEnvironment::default(), CallLanguage::Lua).unwrap().is_none());
}

#[test]
fn powershell_quoted_and_here_strings_and_block_comments_are_opaque() {
    let found = call("outer('one''two, fake(', \"escape`\" ,(\", <# , fake( #> @\"\r\na, fake(\r\n\"@, ", CallLanguage::PowerShell);
    assert_eq!((found.name.as_str(), found.argument), ("outer", 3));
    for language in [CallLanguage::Shell, CallLanguage::Ruby] {
        let found = call("outer('fake(a,b)', # fake(,\n a, ", language);
        assert_eq!((found.name.as_str(), found.argument), ("outer", 2));
    }
}

#[test]
fn javascript_regex_is_distinct_from_division_and_survives_comments() {
    for text in ["outer(/[(,)]\\/foo/g, ", "outer(// comment\n /[(,)]/, ", "outer((function() { return /[(,)]/; })(), ",
        "outer((function() { if (ok) /fake(,/.test(value); })(), "] {
        let found = call(text, CallLanguage::JavaScript);
        assert_eq!((found.name.as_str(), found.argument), ("outer", 1), "{text}");
    }
    let found = call("outer(one / two, inner(one / (two + three), ", CallLanguage::JavaScript);
    assert_eq!((found.name.as_str(), found.argument), ("inner", 1));
}

#[test]
fn cpp_raw_literal_uses_its_own_delimiter() {
    let found = call("outer(R\"tag(fake(, \" text )tag\", ", CallLanguage::General);
    assert_eq!((found.name.as_str(), found.argument), ("outer", 1));
}

#[test]
fn api_environment_controls_call_punctuation_and_unicode_names() {
    let environment = ApiEnvironment { start: '«', stop: '»', separator: '|', terminal: '!',
        additional_word_chars: ".-".to_string(), ..ApiEnvironment::default() };
    let text = "\u{feff}中.fetch-item «[one, two]| 'fake«a|b»'| ";
    let found = find_call(text, text.len(), &environment, CallLanguage::Plain).unwrap().unwrap();
    assert_eq!((found.name.as_str(), found.argument), ("中.fetch-item", 2));
    assert_eq!(text.get(found.name_range), Some("中.fetch-item"));
    assert_eq!(text.get(found.open_byte..).and_then(|tail| tail.chars().next()), Some('«'));
    assert!(find_call("name«a!", usize::MAX, &environment, CallLanguage::Plain).unwrap().is_none());
}

#[test]
fn statement_terminators_and_mismatched_closes_clear_old_calls() {
    for text in ["outer(a;", "outer(a]", "outer(a)"] {
        assert!(find_call(text, text.len(), &ApiEnvironment::default(), CallLanguage::General).unwrap().is_none(), "{text}");
    }
    let found = call("bad(]; good(a, ", CallLanguage::General);
    assert_eq!((found.name.as_str(), found.argument), ("good", 1));
    let found = call("outer(function() { statement(); }, ", CallLanguage::JavaScript);
    assert_eq!((found.name.as_str(), found.argument), ("outer", 1));
}

#[test]
fn scan_limits_are_explicit_and_positions_are_safe_at_every_byte() {
    let environment = ApiEnvironment::default();
    let deep = format!("{}call(", "(".repeat(MAX_DEPTH));
    assert_eq!(find_call(&deep, usize::MAX, &environment, CallLanguage::Plain), Err(ScanLimit::NestingTooDeep));
    let long = format!("/*{}fake(a,", " ".repeat(MAX_CALL_SCAN_BYTES));
    assert_eq!(find_call(&long, usize::MAX, &environment, CallLanguage::General), Err(ScanLimit::PrefixTooLong));
    let large_document = format!("call(one, {}", " ".repeat(MAX_CALL_SCAN_BYTES));
    assert_eq!(find_call(&large_document, 10, &environment, CallLanguage::Plain).unwrap().unwrap().argument, 1);
    assert!(find_call(&format!("{}(", "名".repeat(MAX_NAME_BYTES)), usize::MAX, &environment, CallLanguage::Plain).unwrap().is_none());
    let rust = format!("outer(r{}\"fake(,", "#".repeat(33));
    assert_eq!(find_call(&rust, usize::MAX, &environment, CallLanguage::Rust), Err(ScanLimit::LiteralDelimiterTooLong));
    for prefix in ["", "--"] {
        let lua = format!("outer({prefix}[{}[fake(,", "=".repeat(33));
        assert_eq!(find_call(&lua, usize::MAX, &environment, CallLanguage::Lua), Err(ScanLimit::LiteralDelimiterTooLong));
    }
    for text in ["\u{feff}中😀(é, 中\r\n", "outer(r#\"中, \"#, ", "/*中*/ f(😀, ", "'中'("] {
        for language in [CallLanguage::Plain, CallLanguage::General, CallLanguage::JavaScript, CallLanguage::Rust, CallLanguage::Python, CallLanguage::Shell, CallLanguage::PowerShell, CallLanguage::Ruby, CallLanguage::Lua, CallLanguage::Sql] {
            for caret in (0..=text.len()).chain([usize::MAX]) {
                if let Some(found) = find_call(text, caret, &environment, language).unwrap() {
                    assert_eq!(text.get(found.name_range.clone()), Some(found.name.as_str()));
                    assert!(found.name_range.end <= found.open_byte);
                    assert!(found.open_byte < text.floor_char_boundary(caret.min(text.len())));
                }
            }
        }
    }
}
