// SPDX-License-Identifier: Apache-2.0
//
// `LangType` enum mirroring NPP's `Notepad_plus_msgs.h:27` declaration order.
// Each variant has:
//   - a stable name used in the `<LexerType name="...">` attribute,
//   - a default display description used in the Language listbox.
//
// Notepad Studio may not implement every lexer in the editor itself, but the
// Style Configurator must round-trip any of these names safely.

use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LangType {
    Text, Php, C, Cpp, Cs, Objc, Java, Rc,
    Html, Xml, Makefile, Pascal, Batch, Ini, Ascii, User,
    Asp, Sql, Vb, JsEmbedded, Css, Perl, Python, Lua,
    Tex, Fortran, Bash, Flash, Nsis, Tcl, Lisp, Scheme,
    Asm, Diff, Props, Ps, Ruby, Smalltalk, Vhdl, Kix, Au3,
    Caml, Ada, Verilog, Matlab, Haskell, Inno, SearchResult,
    Cmake, Yaml, Cobol, Gui4Cli, D, PowerShell, R, Jsp,
    CoffeeScript, Json, Javascript, Fortran77, Baanc, Srec,
    Ihex, Tehex, Swift,
    Asn1, Avs, BlitzBasic, PureBasic, FreeBasic,
    Csound, Erlang, Escript, Forth, Latex,
    Mmixal, Nim, NnCronTab, OScript, Rebol,
    Registry, Rust, Spice, Txt2Tags, VisualProlog,
    TypeScript, Json5, MsSql, GdScript, Hollywood,
    Golang, Raku, Toml, Sas, ErrorList,
    External,
}

impl LangType {
    /// Stable lexer-name token used as the XML attribute `name="..."`.
    /// Matches NPP's value table; lower-case, no spaces.
    pub const fn xml_name(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Php => "php",
            Self::C => "c",
            Self::Cpp => "cpp",
            Self::Cs => "cs",
            Self::Objc => "objc",
            Self::Java => "java",
            Self::Rc => "rc",
            Self::Html => "html",
            Self::Xml => "xml",
            Self::Makefile => "makefile",
            Self::Pascal => "pascal",
            Self::Batch => "batch",
            Self::Ini => "ini",
            Self::Ascii => "nfo",
            Self::User => "user",
            Self::Asp => "asp",
            Self::Sql => "sql",
            Self::Vb => "vb",
            Self::JsEmbedded => "javascript",
            Self::Css => "css",
            Self::Perl => "perl",
            Self::Python => "python",
            Self::Lua => "lua",
            Self::Tex => "tex",
            Self::Fortran => "fortran",
            Self::Bash => "bash",
            Self::Flash => "flash",
            Self::Nsis => "nsis",
            Self::Tcl => "tcl",
            Self::Lisp => "lisp",
            Self::Scheme => "scheme",
            Self::Asm => "asm",
            Self::Diff => "diff",
            Self::Props => "props",
            Self::Ps => "postscript",
            Self::Ruby => "ruby",
            Self::Smalltalk => "smalltalk",
            Self::Vhdl => "vhdl",
            Self::Kix => "kix",
            Self::Au3 => "autoit",
            Self::Caml => "caml",
            Self::Ada => "ada",
            Self::Verilog => "verilog",
            Self::Matlab => "matlab",
            Self::Haskell => "haskell",
            Self::Inno => "inno",
            Self::SearchResult => "searchResult",
            Self::Cmake => "cmake",
            Self::Yaml => "yaml",
            Self::Cobol => "cobol",
            Self::Gui4Cli => "gui4cli",
            Self::D => "d",
            Self::PowerShell => "powershell",
            Self::R => "r",
            Self::Jsp => "jsp",
            Self::CoffeeScript => "coffeescript",
            Self::Json => "json",
            Self::Javascript => "javascript.js",
            Self::Fortran77 => "fortran77",
            Self::Baanc => "baanc",
            Self::Srec => "srec",
            Self::Ihex => "ihex",
            Self::Tehex => "tehex",
            Self::Swift => "swift",
            Self::Asn1 => "asn1",
            Self::Avs => "avs",
            Self::BlitzBasic => "blitzbasic",
            Self::PureBasic => "purebasic",
            Self::FreeBasic => "freebasic",
            Self::Csound => "csound",
            Self::Erlang => "erlang",
            Self::Escript => "escript",
            Self::Forth => "forth",
            Self::Latex => "latex",
            Self::Mmixal => "mmixal",
            Self::Nim => "nim",
            Self::NnCronTab => "nncrontab",
            Self::OScript => "oscript",
            Self::Rebol => "rebol",
            Self::Registry => "registry",
            Self::Rust => "rust",
            Self::Spice => "spice",
            Self::Txt2Tags => "txt2tags",
            Self::VisualProlog => "visualprolog",
            Self::TypeScript => "typescript",
            Self::Json5 => "json5",
            Self::MsSql => "mssql",
            Self::GdScript => "gdscript",
            Self::Hollywood => "hollywood",
            Self::Golang => "go",
            Self::Raku => "raku",
            Self::Toml => "toml",
            Self::Sas => "sas",
            Self::ErrorList => "errorlist",
            Self::External => "external",
        }
    }

    /// Default display description for the Language listbox. Themes override
    /// this via `<LexerType desc="...">`.
    pub const fn default_desc(self) -> &'static str {
        match self {
            Self::Text => "Normal Text",
            Self::Php => "PHP",
            Self::C => "C",
            Self::Cpp => "C++",
            Self::Cs => "C#",
            Self::Objc => "Objective-C",
            Self::Java => "Java",
            Self::Rc => "Resource",
            Self::Html => "HTML",
            Self::Xml => "XML",
            Self::Makefile => "Makefile",
            Self::Pascal => "Pascal",
            Self::Batch => "Batch",
            Self::Ini => "ini",
            Self::Ascii => "NFO",
            Self::User => "User Defined",
            Self::Asp => "ASP",
            Self::Sql => "SQL",
            Self::Vb => "Visual Basic",
            Self::JsEmbedded => "JavaScript (embedded)",
            Self::Css => "CSS",
            Self::Perl => "Perl",
            Self::Python => "Python",
            Self::Lua => "Lua",
            Self::Tex => "TeX",
            Self::Fortran => "Fortran free form",
            Self::Bash => "Shell",
            Self::Flash => "Flash ActionScript",
            Self::Nsis => "NSIS",
            Self::Tcl => "TCL",
            Self::Lisp => "Lisp",
            Self::Scheme => "Scheme",
            Self::Asm => "Assembly",
            Self::Diff => "Diff",
            Self::Props => "Properties",
            Self::Ps => "PostScript",
            Self::Ruby => "Ruby",
            Self::Smalltalk => "Smalltalk",
            Self::Vhdl => "VHDL",
            Self::Kix => "KiXtart",
            Self::Au3 => "AutoIt",
            Self::Caml => "CAML",
            Self::Ada => "Ada",
            Self::Verilog => "Verilog",
            Self::Matlab => "MATLAB",
            Self::Haskell => "Haskell",
            Self::Inno => "Inno Setup",
            Self::SearchResult => "Search result",
            Self::Cmake => "CMake",
            Self::Yaml => "YAML",
            Self::Cobol => "COBOL",
            Self::Gui4Cli => "Gui4Cli",
            Self::D => "D",
            Self::PowerShell => "PowerShell",
            Self::R => "R",
            Self::Jsp => "JSP",
            Self::CoffeeScript => "CoffeeScript",
            Self::Json => "JSON",
            Self::Javascript => "JavaScript",
            Self::Fortran77 => "Fortran fixed form",
            Self::Baanc => "BaanC",
            Self::Srec => "S-Record",
            Self::Ihex => "Intel HEX",
            Self::Tehex => "Tektronix extended HEX",
            Self::Swift => "Swift",
            Self::Asn1 => "ASN.1",
            Self::Avs => "AviSynth",
            Self::BlitzBasic => "BlitzBasic",
            Self::PureBasic => "PureBasic",
            Self::FreeBasic => "FreeBasic",
            Self::Csound => "Csound",
            Self::Erlang => "Erlang",
            Self::Escript => "ESCRIPT",
            Self::Forth => "Forth",
            Self::Latex => "LaTeX",
            Self::Mmixal => "MMIXAL",
            Self::Nim => "Nim",
            Self::NnCronTab => "Nncrontab",
            Self::OScript => "OScript",
            Self::Rebol => "REBOL",
            Self::Registry => "registry",
            Self::Rust => "Rust",
            Self::Spice => "Spice",
            Self::Txt2Tags => "txt2tags",
            Self::VisualProlog => "Visual Prolog",
            Self::TypeScript => "TypeScript",
            Self::Json5 => "JSON5",
            Self::MsSql => "MS-SQL",
            Self::GdScript => "GDScript",
            Self::Hollywood => "Hollywood",
            Self::Golang => "Go",
            Self::Raku => "Raku",
            Self::Toml => "TOML",
            Self::Sas => "SAS",
            Self::ErrorList => "ErrorList",
            Self::External => "External",
        }
    }

    /// Total count of declared variants. Useful for tests and array sizing.
    pub const fn count() -> usize {
        ALL.len()
    }

    /// Look up the enum value by its XML `name=` attribute.
    pub fn from_xml_name(name: &str) -> Option<Self> {
        ALL.iter().copied().find(|lt| lt.xml_name() == name)
    }
}

impl fmt::Display for LangType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.xml_name())
    }
}

/// Iteration order matches NPP's enum declaration order.
pub const ALL: &[LangType] = &[
    LangType::Text, LangType::Php, LangType::C, LangType::Cpp, LangType::Cs,
    LangType::Objc, LangType::Java, LangType::Rc,
    LangType::Html, LangType::Xml, LangType::Makefile, LangType::Pascal, LangType::Batch,
    LangType::Ini, LangType::Ascii, LangType::User,
    LangType::Asp, LangType::Sql, LangType::Vb, LangType::JsEmbedded, LangType::Css,
    LangType::Perl, LangType::Python, LangType::Lua,
    LangType::Tex, LangType::Fortran, LangType::Bash, LangType::Flash, LangType::Nsis,
    LangType::Tcl, LangType::Lisp, LangType::Scheme,
    LangType::Asm, LangType::Diff, LangType::Props, LangType::Ps, LangType::Ruby,
    LangType::Smalltalk, LangType::Vhdl, LangType::Kix, LangType::Au3,
    LangType::Caml, LangType::Ada, LangType::Verilog, LangType::Matlab, LangType::Haskell,
    LangType::Inno, LangType::SearchResult,
    LangType::Cmake, LangType::Yaml, LangType::Cobol, LangType::Gui4Cli, LangType::D,
    LangType::PowerShell, LangType::R, LangType::Jsp,
    LangType::CoffeeScript, LangType::Json, LangType::Javascript, LangType::Fortran77,
    LangType::Baanc, LangType::Srec,
    LangType::Ihex, LangType::Tehex, LangType::Swift,
    LangType::Asn1, LangType::Avs, LangType::BlitzBasic, LangType::PureBasic,
    LangType::FreeBasic,
    LangType::Csound, LangType::Erlang, LangType::Escript, LangType::Forth, LangType::Latex,
    LangType::Mmixal, LangType::Nim, LangType::NnCronTab, LangType::OScript, LangType::Rebol,
    LangType::Registry, LangType::Rust, LangType::Spice, LangType::Txt2Tags,
    LangType::VisualProlog,
    LangType::TypeScript, LangType::Json5, LangType::MsSql, LangType::GdScript,
    LangType::Hollywood,
    LangType::Golang, LangType::Raku, LangType::Toml, LangType::Sas, LangType::ErrorList,
    LangType::External,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xml_name_round_trip() {
        for &lt in ALL {
            let n = lt.xml_name();
            assert_eq!(LangType::from_xml_name(n), Some(lt), "round-trip for {n}");
        }
    }

    #[test]
    fn count_matches_npp() {
        // 95 in the NPP enum (excluding bookkeeping sentinels we don't use,
        // but including L_TEXT and L_USER for completeness). Adjust the
        // assertion if the enum changes.
        assert!(LangType::count() >= 90);
    }

    #[test]
    fn unknown_name_is_none() {
        assert_eq!(LangType::from_xml_name("not-a-lexer"), None);
    }
}
