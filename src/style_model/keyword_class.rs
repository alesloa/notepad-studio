// SPDX-License-Identifier: Apache-2.0
//
// `keywordClass` attribute parsing for `<WordsStyle>`. NPP uses string tokens
// `instre1`, `instre2`, `type1..type7`, `substyle1..substyle8`. Internally
// they're flattened to integers in `Parameters.cpp::getKwClassFromName`.
//
// V1 of the Style Configurator does NOT expose a per-lexer keyword editor,
// but we PARSE and PRESERVE the value so round-tripping any theme XML keeps
// the original `keywordClass` attribute intact.

use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeywordClass {
    Instre(u8),
    Type(u8),
    Substyle(u8),
}

impl KeywordClass {
    /// Parse one of `instre1`, `instre2`, `type1..type7`, `substyle1..substyle8`.
    /// Returns `None` for unrecognised tokens (parser preserves them as raw text).
    pub fn from_token(s: &str) -> Option<Self> {
        let s = s.trim();
        if let Some(rest) = s.strip_prefix("instre") {
            return rest.parse::<u8>().ok().filter(|n| (1..=2).contains(n))
                .map(Self::Instre);
        }
        if let Some(rest) = s.strip_prefix("type") {
            return rest.parse::<u8>().ok().filter(|n| (1..=7).contains(n))
                .map(Self::Type);
        }
        if let Some(rest) = s.strip_prefix("substyle") {
            return rest.parse::<u8>().ok().filter(|n| (1..=8).contains(n))
                .map(Self::Substyle);
        }
        None
    }

    /// Encode the class as the integer Style stores. Mirrors NPP's mapping:
    /// instre1=0, instre2=1, type1=2..type7=8, substyle1=9..substyle8=16.
    pub fn to_int(self) -> i32 {
        match self {
            Self::Instre(n) => (n - 1) as i32,
            Self::Type(n) => 1 + n as i32,
            Self::Substyle(n) => 8 + n as i32,
        }
    }

    /// Inverse of `to_int`. Used by `xml_save` if a caller has the integer
    /// and needs the textual token.
    pub fn from_int(n: i32) -> Option<Self> {
        match n {
            0 => Some(Self::Instre(1)),
            1 => Some(Self::Instre(2)),
            2..=8 => Some(Self::Type((n - 1) as u8)),
            9..=16 => Some(Self::Substyle((n - 8) as u8)),
            _ => None,
        }
    }
}

impl fmt::Display for KeywordClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Instre(n) => write!(f, "instre{n}"),
            Self::Type(n) => write!(f, "type{n}"),
            Self::Substyle(n) => write!(f, "substyle{n}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_round_trip() {
        for s in [
            "instre1", "instre2", "type1", "type2", "type7", "substyle1", "substyle8",
        ] {
            let parsed = KeywordClass::from_token(s).expect("known token");
            assert_eq!(parsed.to_string(), s);
        }
    }

    #[test]
    fn rejects_unknown() {
        assert_eq!(KeywordClass::from_token("type0"), None);
        assert_eq!(KeywordClass::from_token("instre3"), None);
        assert_eq!(KeywordClass::from_token("substyle9"), None);
        assert_eq!(KeywordClass::from_token("nope"), None);
    }

    #[test]
    fn int_round_trip() {
        for s in ["instre1", "instre2", "type1", "type7", "substyle1", "substyle8"] {
            let kc = KeywordClass::from_token(s).unwrap();
            assert_eq!(KeywordClass::from_int(kc.to_int()), Some(kc));
        }
    }
}
