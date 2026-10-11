use super::*;

fn document(body: &str) -> String {
    format!("<NotepadPlus><AutoComplete language='test'>{body}</AutoComplete></NotepadPlus>")
}

#[test]
fn original_api_fixture_preserves_overloads_metadata_and_sorted_names() {
    let registry = ApiRegistry::parse(&document(
        "<KeyWord name='writeItem' func='yes'>\
         <Overload retVal='Result&lt;bool&gt;' descr='Writes an item.&#10;May return an error.'>\
         <Param name='label: &amp;str'/><Param name='value: 数字'/></Overload>\
         <Overload retVal='bool'/></KeyWord><KeyWord name='ALPHA'/><KeyWord name='beta'/>"
    )).unwrap();
    assert_eq!(registry.environment, ApiEnvironment::default());
    assert_eq!(registry.entries().iter().map(|entry| entry.name.as_str()).collect::<Vec<_>>(),
        ["ALPHA", "beta", "writeItem"]);
    let entry = registry.find("WRITEITEM").unwrap();
    assert!(entry.is_function);
    assert_eq!(entry.overloads.len(), 2);
    assert_eq!(entry.overloads[0].return_text, "Result<bool>");
    assert_eq!(entry.overloads[0].parameters, ["label: &str", "value: 数字"]);
    assert_eq!(entry.overloads[0].description, "Writes an item.\nMay return an error.");
    assert!(entry.overloads[1].parameters.is_empty());
    assert!(!registry.find("alpha").unwrap().is_function);
}

#[test]
fn environment_controls_case_prefixes_delimiters_and_extra_word_characters() {
    let registry = ApiRegistry::parse(&document(
        "<Environment ignoreCase='no' startFunc='«' stopFunc='»' paramSeparator='|' terminal='!' additionalWordChar='.:$'/>\
         <KeyWord name='Beta'/><KeyWord name='beta'/><KeyWord name='βeta'/><KeyWord name='berry'/>"
    )).unwrap();
    let env = &registry.environment;
    assert!(!env.ignore_case);
    assert_eq!((env.start, env.stop, env.separator, env.terminal), ('«', '»', '|', '!'));
    assert!(env.is_word_char('中'));
    assert!(env.is_word_char('$'));
    assert!(env.is_word_char('_'));
    assert!(!env.is_word_char(' '));
    assert!(env.matches_prefix("Beta", "Be"));
    assert!(!env.matches_prefix("Beta", "be"));
    assert!(!env.matches_prefix("βeta", "b"));
    assert!(registry.find("BETA").is_none());
    assert_eq!(registry.matching("be").map(|entry| entry.name.as_str()).collect::<Vec<_>>(), ["berry", "beta"]);
    assert_eq!(registry.matching("β").count(), 1);
    assert_eq!(registry.matching("nothing").count(), 0);
    let insensitive = ApiEnvironment::default();
    assert!(insensitive.matches_prefix("Beta", "bE"));
    assert!(!insensitive.matches_prefix("中", "a"));
    assert!(!insensitive.matches_prefix("a", "ab"));
}

#[test]
fn duplicate_function_names_merge_overloads_in_source_order_under_case_policy() {
    let registry = ApiRegistry::parse(&document(
        "<KeyWord name='call' func='yes'><Overload retVal='first'/></KeyWord>\
         <KeyWord name='CALL' func='yes'><Overload retVal='second'/></KeyWord>"
    )).unwrap();
    assert_eq!(registry.entries().len(), 1);
    let entry = registry.find("Call").unwrap();
    assert_eq!(entry.name, "call");
    assert_eq!(entry.overloads.iter().map(|overload| overload.return_text.as_str()).collect::<Vec<_>>(), ["first", "second"]);
    assert!(ApiRegistry::parse(&document("<KeyWord name='call'/><KeyWord name='CALL' func='yes'/>")).is_err());
}

#[test]
fn malformed_xml_or_records_never_return_a_partial_registry() {
    for xml in [
        "", "<NotepadPlus>", "<NotepadPlus><AutoComplete></NotepadPlus>",
        "<NotepadPlus/>", "<Other/>",
        "<NotepadPlus><AutoComplete/></NotepadPlus><NotepadPlus/>",
        "<NotepadPlus><AutoComplete/><AutoComplete/></NotepadPlus>",
        "<NotepadPlus><AutoComplete><?xml version='1.0'?></AutoComplete></NotepadPlus>",
        "<?xml encoding='UTF-8'?><NotepadPlus><AutoComplete/></NotepadPlus>",
        "<?xml version='9.0'?><NotepadPlus><AutoComplete/></NotepadPlus>",
    ] { assert!(ApiRegistry::parse(xml).is_err(), "{xml}"); }
    for body in [
        "<KeyWord/>", "<KeyWord name=''/>", "<KeyWord name='line&#10;break'/>",
        "<KeyWord name='word' func='true'/>", "<KeyWord name='a' name='b'/>",
        "<KeyWord name='word'><Overload retVal='bool'/></KeyWord>",
        "<KeyWord name='call' func='yes'><Overload/></KeyWord>",
        "<KeyWord name='call' func='yes'><Overload retVal='bool'><Param/></Overload></KeyWord>",
        "<Param name='bad'/>", "<Environment/><Environment/>",
        "<KeyWord name='okay'/>unexpected text", "<KeyWord name='&unknown;'/>",
        "<KeyWord name='&#0;'/>", "<![CDATA[raw]]>",
    ] { assert!(ApiRegistry::parse(&document(body)).is_err(), "{body}"); }
}

#[test]
fn rejects_dtd_and_external_entities_without_reading_them() {
    for declaration in [
        "<!DOCTYPE NotepadPlus SYSTEM 'file:///missing-secret'>",
        "<!DOCTYPE NotepadPlus [<!ENTITY data SYSTEM 'file:///missing-secret'>]>",
        "<!DOCTYPE NotepadPlus [<!ENTITY many 'repeated'>]>",
    ] {
        let error = ApiRegistry::parse(&format!("{declaration}{}", document("<KeyWord name='safe'/>"))).unwrap_err();
        assert!(error.to_string().contains("DTD"));
    }
}

#[test]
fn rejects_ambiguous_and_oversized_environment_values() {
    for attrs in [
        "ignoreCase='sometimes'", "startFunc=''", "startFunc='(('", "startFunc='a'",
        "startFunc=' '", "startFunc='&#10;'", "stopFunc='('", "terminal=','",
        "additionalWordChar='('", "additionalWordChar='a b'",
    ] {
        assert!(ApiRegistry::parse(&document(&format!("<Environment {attrs}/><KeyWord name='safe'/>"))).is_err(), "{attrs}");
    }
    assert!(ApiRegistry::parse(&document(&format!("<Environment additionalWordChar='{}'/>", ".".repeat(129)))).is_err());
}

#[test]
fn resource_limits_include_extensions_attributes_and_merged_records() {
    let attr = "x".repeat(MAX_ATTRIBUTE_BYTES + 1);
    assert!(ApiRegistry::parse(&document(&format!("<KeyWord name='{attr}'/>"))).is_err());
    let attrs: String = (0..33).map(|i| format!(" a{i}='v'")).collect();
    assert!(ApiRegistry::parse(&document(&format!("<Extension{attrs}/>"))).is_err());
    let deep = format!("{}{}", "<Extension>".repeat(MAX_DEPTH), "</Extension>".repeat(MAX_DEPTH));
    assert!(ApiRegistry::parse(&document(&deep)).is_err());
    let params = "<Param name='value'/>".repeat(MAX_PARAMETERS + 1);
    assert!(ApiRegistry::parse(&document(&format!("<KeyWord name='call' func='yes'><Overload retVal=''>{params}</Overload></KeyWord>"))).is_err());
    let overload = "<Overload retVal='bool'/>".repeat(MAX_OVERLOADS + 1);
    assert!(ApiRegistry::parse(&document(&format!("<KeyWord name='call' func='yes'>{overload}</KeyWord>"))).is_err());
    let duplicate = "<KeyWord name='call' func='yes'><Overload retVal=''/></KeyWord>".repeat(MAX_OVERLOADS + 1);
    assert!(ApiRegistry::parse(&document(&duplicate)).is_err());
    let entries = "<KeyWord name='word'/>".repeat(MAX_ENTRIES + 1);
    assert!(ApiRegistry::parse(&document(&entries)).is_err());
    assert!(ApiRegistry::parse(&" ".repeat(MAX_API_BYTES + 1)).is_err());
}

#[test]
fn supports_safe_extension_nodes_empty_registry_and_bom_comments() {
    let registry = ApiRegistry::parse(&format!("\u{feff}<?xml version='1.0' encoding='UTF-8'?><!--original--> {}",
        document("<Extension><Label>text &amp; facts</Label></Extension><KeyWord name='word'/>"))).unwrap();
    assert_eq!(registry.entries().len(), 1);
    assert!(ApiRegistry::parse(&document("")).unwrap().entries().is_empty());
    assert!(ApiRegistry::parse(&format!("<?xml version='1.0' encoding='windows-1252'?>{}", document(""))).is_err());
}

struct TempApi { dir: std::path::PathBuf }

impl TempApi {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("notepad-studio-api-tests-{}-{id}", std::process::id()));
        std::fs::create_dir(&dir).unwrap();
        Self { dir }
    }
    fn path(&self) -> std::path::PathBuf { self.dir.join("test.xml") }
}

impl Drop for TempApi {
    fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.dir); }
}

#[test]
fn loader_handles_missing_utf8_and_bom_utf16_files_without_modifying_them() {
    let temp = TempApi::new();
    assert!(ApiRegistry::load(&temp.path()).unwrap().is_none());
    let xml = document("<KeyWord name='写入' func='yes'><Overload retVal='结果'/></KeyWord>");
    let mut utf8_bom = vec![0xef, 0xbb, 0xbf]; utf8_bom.extend(xml.as_bytes());
    let mut little = vec![0xff, 0xfe]; let mut big = vec![0xfe, 0xff];
    for unit in xml.encode_utf16() { little.extend(unit.to_le_bytes()); big.extend(unit.to_be_bytes()); }
    for bytes in [xml.as_bytes().to_vec(), utf8_bom, little, big] {
        std::fs::write(temp.path(), &bytes).unwrap();
        let registry = ApiRegistry::load(&temp.path()).unwrap().unwrap();
        assert_eq!(registry.find("写入").unwrap().overloads[0].return_text, "结果");
        assert_eq!(std::fs::read(temp.path()).unwrap(), bytes);
    }
}

#[test]
fn loader_rejects_bad_unicode_and_oversized_inputs_without_replacing_files() {
    let temp = TempApi::new();
    for bytes in [vec![0xff], vec![0xff, 0xfe, 0], vec![0xff, 0xfe, 0, 0xd8], vec![b' '; MAX_API_BYTES + 1]] {
        std::fs::write(temp.path(), &bytes).unwrap();
        assert!(ApiRegistry::load(&temp.path()).is_err());
        assert_eq!(std::fs::read(temp.path()).unwrap(), bytes);
    }
    assert!(ApiRegistry::load(&temp.dir).is_err());
}

#[test]
fn binary_prefix_index_handles_empty_and_nonexistent_ranges() {
    let registry = ApiRegistry::parse(&document("<KeyWord name='apple'/><KeyWord name='Apricot'/><KeyWord name='berry'/><KeyWord name='中文'/>" )).unwrap();
    assert_eq!(registry.matching("").count(), 4);
    assert_eq!(registry.matching("AP").map(|entry| entry.name.as_str()).collect::<Vec<_>>(), ["apple", "Apricot"]);
    assert_eq!(registry.matching("zz").count(), 0);
    assert_eq!(registry.matching("中").count(), 1);
    assert!(registry.find("missing").is_none());
}
