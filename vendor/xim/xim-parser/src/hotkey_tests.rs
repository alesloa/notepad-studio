use crate::{
    read, write_to_vec, HotKeyState, HotKeyTriggers, ReadError, Reader, TriggerKey, XimRead,
    XimWrite,
};
use alloc::{vec, vec::Vec};

fn entries() -> Vec<(TriggerKey, HotKeyState)> {
    vec![
        (
            TriggerKey {
                keysym: 0xff0d,
                modifier: 4,
                modifier_mask: 5,
            },
            HotKeyState::On,
        ),
        (
            TriggerKey {
                keysym: 0x41,
                modifier: 8,
                modifier_mask: 9,
            },
            HotKeyState::Off,
        ),
        (
            TriggerKey {
                keysym: 0x20,
                modifier: 16,
                modifier_mask: 17,
            },
            HotKeyState::On,
        ),
    ]
}

// Encode directly from the specified CARD32 fields, independently of XimWrite.
fn wire(entries: &[(TriggerKey, HotKeyState)]) -> Vec<u8> {
    let mut bytes = (entries.len() as u32).to_ne_bytes().to_vec();
    for (key, _) in entries {
        for word in [key.keysym, key.modifier, key.modifier_mask] {
            bytes.extend_from_slice(&word.to_ne_bytes());
        }
    }
    for (_, state) in entries {
        bytes.extend_from_slice(&(*state as u32).to_ne_bytes());
    }
    bytes
}

#[test]
fn zero_hotkeys_read_and_write() {
    let expected = HotKeyTriggers {
        triggers: Vec::new(),
    };
    assert_eq!(
        read::<HotKeyTriggers>(&0u32.to_ne_bytes()).unwrap(),
        expected
    );
    assert_eq!(write_to_vec(&expected), 0u32.to_ne_bytes());
}

#[test]
fn one_hotkey_reads_its_state() {
    let expected = HotKeyTriggers {
        triggers: entries().into_iter().take(1).collect(),
    };
    assert_eq!(
        read::<HotKeyTriggers>(&wire(&expected.triggers)).unwrap(),
        expected
    );
}

#[test]
fn multiple_hotkeys_keep_key_and_state_order() {
    let expected = HotKeyTriggers {
        triggers: entries(),
    };
    assert_eq!(
        read::<HotKeyTriggers>(&wire(&expected.triggers)).unwrap(),
        expected
    );
}

#[test]
fn hotkey_size_matches_all_card32_fields() {
    let value = HotKeyTriggers {
        triggers: entries(),
    };
    assert_eq!(value.size(), wire(&value.triggers).len());
}

#[test]
fn public_hotkey_writer_round_trips() {
    let expected = HotKeyTriggers {
        triggers: entries(),
    };
    let bytes = write_to_vec(&expected);
    assert_eq!(bytes, wire(&expected.triggers));
    assert_eq!(read::<HotKeyTriggers>(&bytes).unwrap(), expected);
}

#[test]
fn all_truncated_hotkey_prefixes_return_end_of_stream() {
    let bytes = wire(&entries());
    for end in 0..bytes.len() {
        assert!(
            matches!(
                read::<HotKeyTriggers>(&bytes[..end]),
                Err(ReadError::EndOfStream)
            ),
            "length {}",
            end
        );
    }
}

#[test]
fn invalid_hotkey_state_returns_typed_error() {
    let value = entries();
    let state_start = 4 + value.len() * 12;
    for index in 0..value.len() {
        for invalid in [0u32, 3, u32::MAX] {
            let mut bytes = wire(&value);
            let start = state_start + index * 4;
            bytes[start..start + 4].copy_from_slice(&invalid.to_ne_bytes());
            assert!(matches!(
                read::<HotKeyTriggers>(&bytes),
                Err(ReadError::InvalidData("HotKeyState", _))
            ));
        }
    }
}

#[test]
fn hotkey_reader_leaves_following_fields_untouched() {
    let expected = HotKeyTriggers {
        triggers: entries(),
    };
    let mut bytes = wire(&expected.triggers);
    bytes.extend_from_slice(&0x12345678u32.to_ne_bytes());
    let mut reader = Reader::new(&bytes);
    assert_eq!(HotKeyTriggers::read(&mut reader).unwrap(), expected);
    assert_eq!(reader.u32().unwrap(), 0x12345678);
    assert_eq!(reader.cursor(), 0);
}

#[test]
fn maximum_attribute_hotkey_array_is_valid() {
    let (key, _) = entries().remove(0);
    let expected = HotKeyTriggers {
        triggers: vec![(key, HotKeyState::On); 4095],
    };
    assert_eq!(
        read::<HotKeyTriggers>(&wire(&expected.triggers)).unwrap(),
        expected
    );
}

#[test]
fn hotkey_count_above_attribute_limit_is_invalid() {
    assert!(matches!(
        read::<HotKeyTriggers>(&4096u32.to_ne_bytes()),
        Err(ReadError::InvalidData("HotKeyTriggersCount", _))
    ));
}

#[test]
fn hostile_hotkey_count_is_rejected_before_reservation() {
    assert!(matches!(
        read::<HotKeyTriggers>(&u32::MAX.to_ne_bytes()),
        Err(ReadError::InvalidData("HotKeyTriggersCount", _))
    ));
}

#[test]
fn generator_snippet_keeps_the_hotkey_implementation() {
    fn hotkey_impl(source: &str) -> &str {
        source
            .split("impl XimRead for HotKeyTriggers {")
            .nth(1)
            .unwrap()
            .split("impl XimRead for u8 {")
            .next()
            .unwrap()
    }
    assert_eq!(
        hotkey_impl(include_str!("parser.rs")),
        hotkey_impl(include_str!("../../xim-gen/res/snippet.rs"))
    );
}
