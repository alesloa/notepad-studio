use super::*;
use std::io::Cursor;

const ALGORITHMS: [(Algorithm, &str); 4] = [(Algorithm::Md5, "md5"), (Algorithm::Sha1, "sha1"),
    (Algorithm::Sha256, "sha256"), (Algorithm::Sha512, "sha512")];
fn vectors() -> serde_json::Value { serde_json::from_str(include_str!("../tests/data/hash_tools_vectors.json")).unwrap() }
fn flags() -> (AtomicBool, AtomicU64) { (AtomicBool::new(false), AtomicU64::new(0)) }
fn bytes(hex: &str) -> Vec<u8> {
    hex.as_bytes().as_chunks::<2>().0.iter().map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap()).collect()
}
fn expected(name: &str, algorithm: &str) -> String {
    vectors()["raw_vectors"].as_array().unwrap().iter().find(|row| row["id"] == name).unwrap()["digests"][algorithm].as_str().unwrap().into()
}

#[test]
fn hash_tools_engine_all_four_raw_digests_match_independent_file_vectors() {
    let data = vectors();
    for row in data["raw_vectors"].as_array().unwrap() {
        let input = bytes(row["input_hex"].as_str().unwrap());
        for (algorithm, name) in ALGORITHMS {
            let (cancel, progress) = flags();
            assert_eq!(digest_reader(algorithm, &mut Cursor::new(&input), &cancel, &progress).unwrap(), row["digests"][name].as_str().unwrap());
            assert_eq!(progress.load(Ordering::Relaxed), input.len() as u64);
        }
    }
}
#[test]
fn hash_tools_engine_whole_and_per_line_match_independent_nul_cr_lf_vectors() {
    let data = vectors();
    for row in data["text_vectors"].as_array().unwrap() {
        let input = Rope::from_str(row["input"].as_str().unwrap());
        for (algorithm, name) in ALGORITHMS {
            for (mode, field) in [(TextMode::Whole, "whole"), (TextMode::PerLine, "per_line")] {
                let (cancel, progress) = flags();
                assert_eq!(text_result(algorithm, &input, mode, &cancel, &progress).unwrap(), row[field][name].as_str().unwrap(), "{} / {name} / {field}", row["id"]);
            }
        }
    }
}
#[test]
fn hash_tools_engine_typed_crlf_is_distinct_from_lf_and_native_protocol_vectors_match() {
    let data: serde_json::Value = serde_json::from_str(include_str!("../tests/data/hash_tools_input_protocol_vectors.json")).unwrap();
    assert_ne!(data["cases"][0]["whole"]["md5"], data["cases"][1]["whole"]["md5"]);
    for row in data["cases"].as_array().unwrap() {
        let input = Rope::from_str(row["input"].as_str().unwrap());
        for (algorithm, name) in ALGORITHMS {
            for (mode, field) in [(TextMode::Whole, "whole"), (TextMode::PerLine, "per_line")] {
                let (cancel, progress) = flags();
                assert_eq!(text_result(algorithm, &input, mode, &cancel, &progress).unwrap(), row[field][name].as_str().unwrap());
            }
        }
    }
}
#[test]
fn hash_tools_engine_unicode_selection_restores_editor_eol_without_transport_bom() {
    for encoding in [Encoding::Utf8, Encoding::Utf8Bom, Encoding::Utf16Le, Encoding::Utf16Be] {
        let rope = Rope::from_str("é中\nnext\n");
        for (algorithm, name) in ALGORITHMS {
            let (cancel, progress) = flags();
            assert_eq!(selection_digest(algorithm, &rope, 0..rope.len_bytes(), EditorFormat { encoding, ending: LineEnding::Crlf }, &cancel, &progress).unwrap(),
                Some(expected("unicode_crlf", name)));
        }
    }
    let rope = Rope::from_str("\u{feff}body"); let (cancel, progress) = flags();
    assert_eq!(selection_digest(Algorithm::Md5, &rope, 0..rope.len_bytes(), EditorFormat { encoding: Encoding::Utf16Le, ending: LineEnding::Lf }, &cancel, &progress).unwrap(),
        Some(expected("literal_bom", "md5")));
}
#[test]
fn hash_tools_engine_mixed_selection_bytes_and_partial_raw_crlf_keep_context() {
    let rope = Rope::from_str("é\r\n中\nlast\rtail"); let (cancel, progress) = flags();
    assert_eq!(selection_digest(Algorithm::Md5, &rope, 0..rope.len_bytes(), EditorFormat { encoding: Encoding::Utf8Bom, ending: LineEnding::Lf }, &cancel, &progress).unwrap(),
        Some(expected("mixed_eol", "md5")));
    let rope = Rope::from_str("\r\n");
    for (ending, input) in [(LineEnding::Crlf, "\n"), (LineEnding::Cr, ""), (LineEnding::Lf, "\n")] {
        let (cancel, progress) = flags(); let digest = digest_reader(Algorithm::Md5, &mut Cursor::new(input), &cancel, &progress).unwrap();
        assert_eq!(selection_digest(Algorithm::Md5, &rope, 1..2, EditorFormat { encoding: Encoding::Utf8, ending }, &cancel, &progress).unwrap(), Some(digest));
    }
}
#[test]
fn hash_tools_engine_ansi_is_strict_and_ignores_unmappable_suffix_after_nul() {
    let data: serde_json::Value = serde_json::from_str(include_str!("../tests/data/hash_tools_ansi_fixture_vectors.json")).unwrap();
    let rope = Rope::from_str("é€\u{81}ÿ\n"); let format = EditorFormat { encoding: Encoding::Latin1, ending: LineEnding::Crlf };
    for (algorithm, name) in ALGORITHMS {
        let (cancel, progress) = flags();
        assert_eq!(selection_digest(algorithm, &rope, 0..rope.len_bytes(), format, &cancel, &progress).unwrap().as_deref(), data["case"]["digests"][name].as_str());
    }
    for (text, expected) in [("abc\0中", "abc"), ("\0中", "")] {
        let rope = Rope::from_str(text); let (cancel, progress) = flags();
        let digest = digest_reader(Algorithm::Md5, &mut Cursor::new(expected), &cancel, &progress).unwrap();
        assert_eq!(selection_digest(Algorithm::Md5, &rope, 0..rope.len_bytes(), format, &cancel, &progress).unwrap(), Some(digest));
    }
    let rope = Rope::from_str("中\0abc"); let (cancel, progress) = flags();
    assert!(matches!(selection_digest(Algorithm::Md5, &rope, 0..rope.len_bytes(), format, &cancel, &progress), Err(HashError::Unmappable { ch: '中' })));
}
#[test]
fn hash_tools_engine_ansi_all_decoded_bytes_match_existing_strict_codec() {
    let format = EditorFormat { encoding: Encoding::Latin1, ending: LineEnding::Lf };
    for byte in 1u8..=255 {
        let decoded = crate::text::encoding::decode_bytes(&[byte]);
        let (cancel, progress) = flags(); let rope = Rope::from_str(&decoded.text);
        let actual = selection_digest(Algorithm::Md5, &rope, 0..rope.len_bytes(), format, &cancel, &progress).unwrap();
        let expected = digest_reader(Algorithm::Md5, &mut Cursor::new([byte]), &cancel, &progress).unwrap();
        assert_eq!(actual, Some(expected), "byte {byte}");
    }
}
#[test]
fn hash_tools_engine_hostile_offsets_are_clamped_without_changing_source() {
    let rope = Rope::from_str("é😀abc"); let format = EditorFormat { encoding: Encoding::Utf8, ending: LineEnding::Lf };
    let (cancel, progress) = flags();
    assert_eq!(selection_digest(Algorithm::Md5, &rope, usize::MAX..usize::MAX, format, &cancel, &progress).unwrap(), None);
    assert_eq!(selection_digest(Algorithm::Md5, &rope, 1..2, format, &cancel, &progress).unwrap(),
        Some(digest_reader(Algorithm::Md5, &mut Cursor::new("é"), &cancel, &progress).unwrap()));
    assert_eq!(rope.to_string(), "é😀abc");
}
#[test]
fn hash_tools_engine_reader_retries_interrupted_and_bounds_reads() {
    struct Short { interrupted: bool, bytes: Cursor<Vec<u8>>, maximum: usize }
    impl Read for Short {
        fn read(&mut self, target: &mut [u8]) -> io::Result<usize> {
            self.maximum = self.maximum.max(target.len());
            if !self.interrupted { self.interrupted = true; return Err(io::ErrorKind::Interrupted.into()); }
            self.bytes.read(&mut target[..1])
        }
    }
    let mut reader = Short { interrupted: false, bytes: Cursor::new(b"abc".to_vec()), maximum: 0 }; let (cancel, progress) = flags();
    assert_eq!(digest_reader(Algorithm::Sha512, &mut reader, &cancel, &progress).unwrap(), expected("abc", "sha512"));
    assert_eq!(reader.maximum, CHUNK);
    struct Hostile; impl Read for Hostile { fn read(&mut self, _: &mut [u8]) -> io::Result<usize> { Ok(usize::MAX) } }
    assert!(matches!(digest_reader(Algorithm::Md5, &mut Hostile, &cancel, &progress), Err(HashError::Input)));
}
#[test]
fn hash_tools_engine_cancel_and_io_error_never_publish_a_partial_digest() {
    let cancel = AtomicBool::new(true); let progress = AtomicU64::new(0);
    assert!(matches!(digest_reader(Algorithm::Md5, &mut Cursor::new("abc"), &cancel, &progress), Err(HashError::Cancelled)));
    let rope = Rope::from_str("abc");
    assert!(matches!(text_result(Algorithm::Md5, &rope, TextMode::Whole, &cancel, &progress), Err(HashError::Cancelled)));
    struct Broken; impl Read for Broken { fn read(&mut self, _: &mut [u8]) -> io::Result<usize> { Err(io::ErrorKind::PermissionDenied.into()) } }
    assert!(matches!(digest_reader(Algorithm::Md5, &mut Broken, &AtomicBool::new(false), &progress), Err(HashError::Io(_))));
    struct CancelAfterRead<'a>(&'a AtomicBool); impl Read for CancelAfterRead<'_> {
        fn read(&mut self, target: &mut [u8]) -> io::Result<usize> { target[0] = b'a'; self.0.store(true, Ordering::Relaxed); Ok(1) }
    }
    let cancel = AtomicBool::new(false);
    assert!(matches!(digest_reader(Algorithm::Md5, &mut CancelAfterRead(&cancel), &cancel, &progress), Err(HashError::Cancelled)));
}
#[test]
fn hash_tools_engine_output_budget_is_checked_before_append() {
    let mut result = "x".repeat(MAX_TEXT_OUTPUT); let before = result.len();
    assert!(matches!(append(&mut result, "\r\n"), Err(HashError::OutputLimit))); assert_eq!(result.len(), before);
}
