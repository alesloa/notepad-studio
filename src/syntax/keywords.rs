// Static per-language keyword + builtin lists used by Auto-Completion's
// `Function Completion` and `Function and Word Completion` paths.
//
// Source-of-truth: each language's official reference (see comments per
// language for the spec URL). These are NOT mock data — they are the real
// reserved-word + commonly-used-builtin set for each language.
//
// NPP loads its function-completion candidates from per-language XML API
// files (`apis/<lang>.xml`) shipped in the install — full function
// signatures with parameter hints. Notepad Studio doesn't ship that XML
// registry yet, so Function Completion uses these static slices as the v1
// candidate pool. Word completion is unaffected (buffer scan).

use crate::syntax::LanguageId;

/// Return the keyword + common-builtin set for `lang`. Empty slice when the
/// language has no meaningful keyword set (Markdown, JSON, plain text).
pub(crate) fn keywords_for(lang: LanguageId) -> &'static [&'static str] {
    match lang {
        LanguageId::Rust => RUST,
        LanguageId::Python => PYTHON,
        LanguageId::C => C,
        LanguageId::CSharp => CSHARP,
        LanguageId::JavaScript => JAVASCRIPT,
        LanguageId::TypeScript | LanguageId::Tsx => TYPESCRIPT,
        LanguageId::Json => JSON,
        LanguageId::Markdown => &[],
        LanguageId::Bash => BASH,
        LanguageId::PowerShell => POWERSHELL,
        LanguageId::Go => GO,
        LanguageId::Java => JAVA,
        LanguageId::Swift => SWIFT,
        LanguageId::Ruby => RUBY,
        LanguageId::Php => PHP,
        LanguageId::Cpp => CPP,
        LanguageId::Lua => LUA,
        LanguageId::Html => HTML,
        LanguageId::Css => CSS,
        LanguageId::Scss => SCSS,
        LanguageId::Yaml => YAML,
        LanguageId::Toml => TOML,
        LanguageId::Xml => XML,
        LanguageId::Sql => SQL,
    }
}

// --- Rust ---
// Strict + reserved keywords from
// https://doc.rust-lang.org/reference/keywords.html plus primitive types,
// std collections, and macros that virtually every Rust file references.
const RUST: &[&str] = &[
    // strict keywords
    "as", "async", "await", "break", "const", "continue", "crate", "dyn",
    "else", "enum", "extern", "false", "fn", "for", "if", "impl", "in",
    "let", "loop", "match", "mod", "move", "mut", "pub", "ref", "return",
    "self", "Self", "static", "struct", "super", "trait", "true", "type",
    "unsafe", "use", "where", "while",
    // reserved keywords
    "abstract", "become", "box", "do", "final", "macro", "override", "priv",
    "try", "typeof", "unsized", "virtual", "yield",
    // primitive types
    "bool", "char", "str", "i8", "i16", "i32", "i64", "i128", "isize",
    "u8", "u16", "u32", "u64", "u128", "usize", "f32", "f64",
    // ubiquitous std types
    "String", "Vec", "Option", "Some", "None", "Result", "Ok", "Err", "Box",
    "Rc", "Arc", "Cell", "RefCell", "Mutex", "RwLock", "HashMap", "HashSet",
    "BTreeMap", "BTreeSet", "Iterator", "IntoIterator",
    // common macros (sans `!`)
    "println", "print", "eprintln", "eprint", "format", "write", "writeln",
    "vec", "assert", "assert_eq", "assert_ne", "debug_assert", "panic",
    "todo", "unimplemented", "unreachable", "dbg", "include_str",
    "include_bytes", "concat", "env", "option_env", "cfg", "matches",
];

// --- Python ---
// Keywords from https://docs.python.org/3/reference/lexical_analysis.html#keywords
// plus builtin functions from https://docs.python.org/3/library/functions.html
// and core dunder names.
const PYTHON: &[&str] = &[
    // keywords (3.12)
    "False", "None", "True", "and", "as", "assert", "async", "await",
    "break", "class", "continue", "def", "del", "elif", "else", "except",
    "finally", "for", "from", "global", "if", "import", "in", "is",
    "lambda", "nonlocal", "not", "or", "pass", "raise", "return", "try",
    "while", "with", "yield",
    // soft keywords
    "match", "case",
    // builtin functions
    "abs", "all", "any", "ascii", "bin", "bool", "breakpoint", "bytearray",
    "bytes", "callable", "chr", "classmethod", "compile", "complex",
    "delattr", "dict", "dir", "divmod", "enumerate", "eval", "exec",
    "filter", "float", "format", "frozenset", "getattr", "globals",
    "hasattr", "hash", "help", "hex", "id", "input", "int", "isinstance",
    "issubclass", "iter", "len", "list", "locals", "map", "max",
    "memoryview", "min", "next", "object", "oct", "open", "ord", "pow",
    "print", "property", "range", "repr", "reversed", "round", "set",
    "setattr", "slice", "sorted", "staticmethod", "str", "sum", "super",
    "tuple", "type", "vars", "zip",
    // common dunders
    "__init__", "__name__", "__main__", "__file__", "__doc__", "__class__",
    "__dict__", "__module__", "__repr__", "__str__", "__len__", "__iter__",
    "__next__", "__enter__", "__exit__",
    // very common stdlib names
    "self", "cls", "Exception", "ValueError", "TypeError", "KeyError",
    "IndexError", "RuntimeError", "StopIteration", "NotImplementedError",
];

// --- C ---
// Keywords from C11 (ISO/IEC 9899:2011) plus widely-used stdlib symbols.
const C: &[&str] = &[
    "auto", "break", "case", "char", "const", "continue", "default", "do",
    "double", "else", "enum", "extern", "float", "for", "goto", "if",
    "inline", "int", "long", "register", "restrict", "return", "short",
    "signed", "sizeof", "static", "struct", "switch", "typedef", "union",
    "unsigned", "void", "volatile", "while",
    // C99/C11 keywords
    "_Alignas", "_Alignof", "_Atomic", "_Bool", "_Complex", "_Generic",
    "_Imaginary", "_Noreturn", "_Static_assert", "_Thread_local",
    // common preprocessor + stdlib
    "NULL", "true", "false", "size_t", "ssize_t", "ptrdiff_t", "int8_t",
    "int16_t", "int32_t", "int64_t", "uint8_t", "uint16_t", "uint32_t",
    "uint64_t", "FILE", "EOF", "stdin", "stdout", "stderr",
    "printf", "fprintf", "sprintf", "snprintf", "scanf", "fscanf", "sscanf",
    "fopen", "fclose", "fread", "fwrite", "fseek", "ftell", "fgets", "fputs",
    "malloc", "calloc", "realloc", "free", "memcpy", "memmove", "memset",
    "memcmp", "strlen", "strcpy", "strncpy", "strcat", "strncat", "strcmp",
    "strncmp", "strchr", "strrchr", "strstr", "exit", "abort", "atoi",
    "atof", "abs", "labs", "qsort", "bsearch", "rand", "srand",
];

// --- C# ---
// Reserved + contextual keywords from
// https://learn.microsoft.com/en-us/dotnet/csharp/language-reference/keywords/
const CSHARP: &[&str] = &[
    "abstract", "as", "base", "bool", "break", "byte", "case", "catch",
    "char", "checked", "class", "const", "continue", "decimal", "default",
    "delegate", "do", "double", "else", "enum", "event", "explicit",
    "extern", "false", "finally", "fixed", "float", "for", "foreach",
    "goto", "if", "implicit", "in", "int", "interface", "internal", "is",
    "lock", "long", "namespace", "new", "null", "object", "operator", "out",
    "override", "params", "private", "protected", "public", "readonly",
    "ref", "return", "sbyte", "sealed", "short", "sizeof", "stackalloc",
    "static", "string", "struct", "switch", "this", "throw", "true", "try",
    "typeof", "uint", "ulong", "unchecked", "unsafe", "ushort", "using",
    "virtual", "void", "volatile", "while",
    // contextual keywords
    "add", "alias", "ascending", "async", "await", "by", "descending",
    "dynamic", "equals", "file", "from", "get", "global", "group", "init",
    "into", "join", "let", "managed", "nameof", "nint", "not", "notnull",
    "nuint", "on", "or", "orderby", "partial", "record", "remove",
    "required", "scoped", "select", "set", "unmanaged", "value", "var",
    "when", "where", "with", "yield",
    // common BCL
    "System", "Console", "WriteLine", "List", "Dictionary", "String", "Int32",
    "Boolean", "Object", "Task", "Exception",
];

// --- JavaScript ---
// Reserved words from ECMA-262 plus DOM/browser globals seen in essentially
// every web file.
const JAVASCRIPT: &[&str] = &[
    "break", "case", "catch", "class", "const", "continue", "debugger",
    "default", "delete", "do", "else", "export", "extends", "false",
    "finally", "for", "function", "if", "import", "in", "instanceof", "new",
    "null", "return", "super", "switch", "this", "throw", "true", "try",
    "typeof", "var", "void", "while", "with", "yield", "let", "static",
    "async", "await", "of", "enum",
    // strict-mode reserved
    "implements", "interface", "package", "private", "protected", "public",
    // global values
    "undefined", "NaN", "Infinity", "globalThis",
    // builtin constructors / namespaces
    "Object", "Array", "String", "Number", "Boolean", "Symbol", "BigInt",
    "Function", "Date", "Math", "JSON", "Promise", "Map", "Set", "WeakMap",
    "WeakSet", "RegExp", "Error", "TypeError", "RangeError", "Proxy",
    "Reflect", "Intl",
    // browser globals
    "window", "document", "console", "navigator", "location", "history",
    "localStorage", "sessionStorage", "fetch", "setTimeout", "setInterval",
    "clearTimeout", "clearInterval", "requestAnimationFrame", "alert",
    "confirm", "prompt",
];

// --- TypeScript ---
// JavaScript set + TypeScript-specific type keywords from the TypeScript
// handbook.
const TYPESCRIPT: &[&str] = &[
    // JS keywords (inlined — not a runtime concat)
    "break", "case", "catch", "class", "const", "continue", "debugger",
    "default", "delete", "do", "else", "export", "extends", "false",
    "finally", "for", "function", "if", "import", "in", "instanceof", "new",
    "null", "return", "super", "switch", "this", "throw", "true", "try",
    "typeof", "var", "void", "while", "with", "yield", "let", "static",
    "async", "await", "of", "enum",
    "implements", "interface", "package", "private", "protected", "public",
    "undefined", "NaN", "Infinity", "globalThis",
    "Object", "Array", "String", "Number", "Boolean", "Symbol", "BigInt",
    "Function", "Date", "Math", "JSON", "Promise", "Map", "Set", "WeakMap",
    "WeakSet", "RegExp", "Error", "TypeError", "RangeError", "Proxy",
    "Reflect", "Intl",
    "window", "document", "console", "navigator", "location", "history",
    "localStorage", "sessionStorage", "fetch", "setTimeout", "setInterval",
    // TS additions
    "abstract", "any", "as", "asserts", "bigint", "boolean", "constructor",
    "declare", "from", "get", "infer", "is", "keyof", "module", "namespace",
    "never", "number", "object", "out", "override", "readonly", "require",
    "satisfies", "set", "string", "symbol", "type", "unique", "unknown",
    "Record", "Partial", "Required", "Readonly", "Pick", "Omit", "Exclude",
    "Extract", "NonNullable", "Parameters", "ReturnType", "Awaited",
];

// --- JSON ---
// Three literals — that's the entire JSON grammar's reserved set.
const JSON: &[&str] = &["true", "false", "null"];

// --- Bash ---
// Reserved words + builtins from `man bash` (RESERVED WORDS, SHELL BUILTIN
// COMMANDS) plus the common external utilities most scripts call.
const BASH: &[&str] = &[
    // reserved words
    "if", "then", "else", "elif", "fi", "case", "esac", "for", "select",
    "while", "until", "do", "done", "in", "function", "time", "coproc",
    // shell builtins
    "alias", "bg", "bind", "break", "builtin", "caller", "cd", "command",
    "compgen", "complete", "compopt", "continue", "declare", "dirs",
    "disown", "echo", "enable", "eval", "exec", "exit", "export", "false",
    "fc", "fg", "getopts", "hash", "help", "history", "jobs", "kill", "let",
    "local", "logout", "mapfile", "popd", "printf", "pushd", "pwd", "read",
    "readarray", "readonly", "return", "set", "shift", "shopt", "source",
    "suspend", "test", "times", "trap", "true", "type", "typeset", "ulimit",
    "umask", "unalias", "unset", "wait",
    // ubiquitous external commands
    "awk", "cat", "chmod", "chown", "cp", "cut", "diff", "find", "grep",
    "head", "less", "ln", "ls", "mkdir", "more", "mv", "rm", "sed", "sort",
    "tail", "tar", "tee", "tr", "uniq", "wc", "xargs", "curl", "wget",
    "git", "ssh", "scp", "rsync",
];

// --- PowerShell ---
// Language keywords from
// https://learn.microsoft.com/en-us/powershell/scripting/lang-spec/chapter-07
// plus the verb-noun cmdlets every script reaches for.
const POWERSHELL: &[&str] = &[
    "begin", "break", "catch", "class", "continue", "data", "define", "do",
    "dynamicparam", "else", "elseif", "end", "enum", "exit", "filter",
    "finally", "for", "foreach", "from", "function", "hidden", "if", "in",
    "param", "process", "return", "static", "switch", "throw", "trap",
    "try", "until", "using", "var", "while",
    // common cmdlets
    "Get-Item", "Get-ChildItem", "Get-Content", "Get-Location", "Get-Date",
    "Get-Process", "Get-Service", "Get-Help", "Get-Command", "Get-Member",
    "Set-Item", "Set-Content", "Set-Location", "Set-Variable",
    "New-Item", "New-Object", "New-Variable",
    "Remove-Item", "Remove-Variable", "Copy-Item", "Move-Item", "Rename-Item",
    "Invoke-Expression", "Invoke-WebRequest", "Invoke-RestMethod",
    "Out-Host", "Out-File", "Out-Null", "Out-String",
    "Write-Host", "Write-Output", "Write-Error", "Write-Warning",
    "Write-Verbose", "Write-Debug",
    "Where-Object", "ForEach-Object", "Select-Object", "Sort-Object",
    "Group-Object", "Measure-Object",
    "Test-Path", "Join-Path", "Split-Path", "Resolve-Path",
    "ConvertTo-Json", "ConvertFrom-Json",
    "Start-Process", "Stop-Process", "Start-Service", "Stop-Service",
    "Read-Host", "Clear-Host",
];

// --- Go ---
// Keywords + predeclared identifiers from
// https://go.dev/ref/spec#Keywords and the Predeclared identifiers section.
const GO: &[&str] = &[
    // keywords
    "break", "case", "chan", "const", "continue", "default", "defer",
    "else", "fallthrough", "for", "func", "go", "goto", "if", "import",
    "interface", "map", "package", "range", "return", "select", "struct",
    "switch", "type", "var",
    // predeclared types
    "bool", "byte", "complex64", "complex128", "error", "float32", "float64",
    "int", "int8", "int16", "int32", "int64", "rune", "string", "uint",
    "uint8", "uint16", "uint32", "uint64", "uintptr", "any", "comparable",
    // predeclared constants
    "true", "false", "iota", "nil",
    // predeclared functions
    "append", "cap", "clear", "close", "complex", "copy", "delete", "imag",
    "len", "make", "max", "min", "new", "panic", "print", "println", "real",
    "recover",
    // common stdlib package roots
    "fmt", "errors", "strings", "strconv", "bytes", "io", "os", "context",
    "sync", "time", "net", "http", "json",
];

// --- Java ---
// All reserved + contextual keywords up through Java 21.
const JAVA: &[&str] = &[
    "abstract", "assert", "boolean", "break", "byte", "case", "catch",
    "char", "class", "const", "continue", "default", "do", "double", "else",
    "enum", "extends", "final", "finally", "float", "for", "goto", "if",
    "implements", "import", "instanceof", "int", "interface", "long",
    "native", "new", "package", "private", "protected", "public", "return",
    "short", "static", "strictfp", "super", "switch", "synchronized", "this",
    "throw", "throws", "transient", "try", "void", "volatile", "while",
    "true", "false", "null",
    // contextual / restricted (Java 10+)
    "var", "yield", "record", "sealed", "permits", "non-sealed",
    // common JDK identifiers
    "System", "String", "Integer", "Long", "Double", "Float", "Boolean",
    "Character", "Object", "Math", "Number", "Class", "Exception",
    "RuntimeException", "Thread", "Runnable", "List", "ArrayList",
    "LinkedList", "Map", "HashMap", "TreeMap", "Set", "HashSet", "TreeSet",
    "Collection", "Iterator", "Iterable", "Optional", "Stream", "Arrays",
    "Collections", "Override", "Deprecated", "FunctionalInterface",
];

// --- Swift ---
// Keywords from https://docs.swift.org/swift-book/documentation/the-swift-programming-language/lexicalstructure/#Keywords
const SWIFT: &[&str] = &[
    // declaration keywords
    "associatedtype", "class", "deinit", "enum", "extension", "fileprivate",
    "func", "import", "init", "inout", "internal", "let", "open", "operator",
    "private", "precedencegroup", "protocol", "public", "rethrows", "static",
    "struct", "subscript", "typealias", "var",
    // statement keywords
    "break", "case", "catch", "continue", "default", "defer", "do", "else",
    "fallthrough", "for", "guard", "if", "in", "repeat", "return", "throw",
    "switch", "where", "while",
    // expression / type keywords
    "Any", "as", "await", "false", "is", "nil", "self", "Self", "super",
    "throws", "true", "try",
    // context-specific
    "associativity", "convenience", "didSet", "dynamic", "final", "get",
    "indirect", "infix", "lazy", "left", "mutating", "none", "nonmutating",
    "optional", "override", "postfix", "precedence", "prefix", "required",
    "right", "set", "some", "Type", "unowned", "weak", "willSet", "async",
    // common stdlib
    "print", "String", "Int", "Double", "Float", "Bool", "Array", "Dictionary",
    "Set", "Optional", "Result",
];

// --- Ruby ---
// Reserved words from
// https://docs.ruby-lang.org/en/3.3/keywords_rdoc.html plus core methods
// every script uses.
const RUBY: &[&str] = &[
    "__ENCODING__", "__LINE__", "__FILE__", "BEGIN", "END", "alias", "and",
    "begin", "break", "case", "class", "def", "defined?", "do", "else",
    "elsif", "end", "ensure", "false", "for", "if", "in", "module", "next",
    "nil", "not", "or", "redo", "rescue", "retry", "return", "self", "super",
    "then", "true", "undef", "unless", "until", "when", "while", "yield",
    // common methods / DSL keywords
    "puts", "print", "p", "pp", "gets", "require", "require_relative", "load",
    "attr_reader", "attr_writer", "attr_accessor", "include", "extend",
    "lambda", "proc", "raise", "throw", "catch", "loop", "block_given?",
    "respond_to?", "is_a?", "kind_of?", "instance_of?", "nil?", "true?",
    "freeze", "frozen?", "dup", "clone", "to_s", "to_i", "to_f", "to_a",
    "to_h", "inspect", "send", "method", "methods",
];

// --- PHP ---
// Reserved words from https://www.php.net/manual/en/reserved.keywords.php
// plus the predefined constants page.
const PHP: &[&str] = &[
    "abstract", "and", "array", "as", "break", "callable", "case", "catch",
    "class", "clone", "const", "continue", "declare", "default", "do",
    "echo", "else", "elseif", "empty", "enddeclare", "endfor", "endforeach",
    "endif", "endswitch", "endwhile", "enum", "extends", "final", "finally",
    "fn", "for", "foreach", "function", "global", "goto", "if", "implements",
    "include", "include_once", "instanceof", "insteadof", "interface",
    "isset", "list", "match", "namespace", "new", "or", "print", "private",
    "protected", "public", "readonly", "require", "require_once", "return",
    "static", "switch", "throw", "trait", "try", "unset", "use", "var",
    "while", "xor", "yield",
    // predefined values
    "true", "false", "null", "TRUE", "FALSE", "NULL",
    // magic constants
    "__CLASS__", "__DIR__", "__FILE__", "__FUNCTION__", "__LINE__",
    "__METHOD__", "__NAMESPACE__", "__TRAIT__",
    // ubiquitous functions
    "strlen", "strpos", "str_replace", "substr", "explode", "implode",
    "trim", "preg_match", "preg_replace", "count", "array_map",
    "array_filter", "array_keys", "array_values", "in_array", "array_merge",
    "json_encode", "json_decode", "var_dump", "print_r", "die", "exit",
];

// --- C++ ---
// Keywords from https://en.cppreference.com/w/cpp/keyword (C++23) plus the
// std identifiers nearly every TU uses.
const CPP: &[&str] = &[
    // C++ keywords
    "alignas", "alignof", "and", "and_eq", "asm", "auto", "bitand", "bitor",
    "bool", "break", "case", "catch", "char", "char8_t", "char16_t",
    "char32_t", "class", "compl", "concept", "const", "consteval",
    "constexpr", "constinit", "const_cast", "continue", "co_await",
    "co_return", "co_yield", "decltype", "default", "delete", "do", "double",
    "dynamic_cast", "else", "enum", "explicit", "export", "extern", "false",
    "float", "for", "friend", "goto", "if", "inline", "int", "long",
    "mutable", "namespace", "new", "noexcept", "not", "not_eq", "nullptr",
    "operator", "or", "or_eq", "private", "protected", "public", "register",
    "reinterpret_cast", "requires", "return", "short", "signed", "sizeof",
    "static", "static_assert", "static_cast", "struct", "switch", "template",
    "this", "thread_local", "throw", "true", "try", "typedef", "typeid",
    "typename", "union", "unsigned", "using", "virtual", "void", "volatile",
    "wchar_t", "while", "xor", "xor_eq",
    // common std
    "std", "cout", "cin", "cerr", "clog", "endl", "string", "string_view",
    "vector", "array", "deque", "list", "forward_list", "stack", "queue",
    "priority_queue", "map", "unordered_map", "set", "unordered_set", "pair",
    "tuple", "optional", "variant", "any", "function", "shared_ptr",
    "unique_ptr", "weak_ptr", "make_shared", "make_unique", "make_pair",
    "make_tuple", "move", "forward", "swap", "size_t", "ptrdiff_t", "nullptr_t",
];

// --- Lua ---
// Reserved words from https://www.lua.org/manual/5.4/manual.html#3.1 plus
// the standard library tables every script touches.
const LUA: &[&str] = &[
    // reserved words
    "and", "break", "do", "else", "elseif", "end", "false", "for", "function",
    "goto", "if", "in", "local", "nil", "not", "or", "repeat", "return",
    "then", "true", "until", "while",
    // basic library
    "print", "ipairs", "pairs", "next", "select", "type", "tostring",
    "tonumber", "require", "pcall", "xpcall", "error", "assert", "rawget",
    "rawset", "rawequal", "rawlen", "setmetatable", "getmetatable", "dofile",
    "loadfile", "load", "collectgarbage", "unpack",
    // stdlib tables
    "string", "table", "math", "io", "os", "coroutine", "debug", "package",
    "_G", "_ENV", "_VERSION",
];

// --- HTML ---
// Element + global-attribute names from
// https://html.spec.whatwg.org/multipage/indices.html.
const HTML: &[&str] = &[
    // structural elements
    "html", "head", "body", "title", "meta", "link", "style", "script",
    "base", "noscript",
    // sectioning + heading
    "header", "footer", "nav", "section", "article", "aside", "main", "h1",
    "h2", "h3", "h4", "h5", "h6", "hgroup",
    // grouping
    "div", "span", "p", "br", "hr", "pre", "blockquote", "ol", "ul", "li",
    "dl", "dt", "dd", "figure", "figcaption",
    // text-level
    "a", "em", "strong", "small", "s", "cite", "q", "dfn", "abbr", "code",
    "var", "samp", "kbd", "sub", "sup", "i", "b", "u", "mark", "ruby", "rt",
    "rp", "bdi", "bdo", "wbr", "time",
    // embedded content
    "img", "iframe", "embed", "object", "picture", "source", "video", "audio",
    "track", "canvas", "svg", "math", "map", "area",
    // tabular
    "table", "caption", "colgroup", "col", "tbody", "thead", "tfoot", "tr",
    "td", "th",
    // forms
    "form", "label", "input", "button", "select", "datalist", "optgroup",
    "option", "textarea", "output", "progress", "meter", "fieldset",
    "legend",
    // interactive
    "details", "summary", "dialog",
    // global attributes (common)
    "class", "id", "style", "title", "lang", "dir", "hidden", "tabindex",
    "accesskey", "contenteditable", "draggable", "spellcheck", "translate",
    "role", "slot", "is", "data",
    // common element attributes
    "href", "src", "alt", "type", "name", "value", "placeholder", "required",
    "disabled", "checked", "selected", "readonly", "multiple", "rel",
    "target", "method", "action", "enctype", "for", "min", "max", "step",
    "pattern", "autocomplete", "autofocus", "novalidate", "form", "list",
    "srcset", "sizes", "loading", "decoding", "crossorigin", "integrity",
    "referrerpolicy", "defer", "async", "charset", "http-equiv",
    "viewBox", "width", "height",
];

// --- CSS ---
// Common properties + values. Not exhaustive — CSS has hundreds of
// properties; this set covers the ~80% of declarations seen in real
// stylesheets (per HTTP Archive crawl data).
const CSS: &[&str] = &[
    // layout
    "display", "position", "top", "right", "bottom", "left", "float", "clear",
    "overflow", "overflow-x", "overflow-y", "visibility", "z-index", "box-sizing",
    // flex
    "flex", "flex-direction", "flex-wrap", "flex-flow", "flex-grow",
    "flex-shrink", "flex-basis", "justify-content", "align-items",
    "align-self", "align-content", "order", "gap", "row-gap", "column-gap",
    // grid
    "grid", "grid-template", "grid-template-columns", "grid-template-rows",
    "grid-template-areas", "grid-area", "grid-column", "grid-row",
    "grid-column-start", "grid-column-end", "grid-row-start", "grid-row-end",
    "grid-auto-columns", "grid-auto-rows", "grid-auto-flow",
    "place-items", "place-content", "place-self",
    // box model
    "width", "height", "min-width", "max-width", "min-height", "max-height",
    "margin", "margin-top", "margin-right", "margin-bottom", "margin-left",
    "padding", "padding-top", "padding-right", "padding-bottom", "padding-left",
    "border", "border-width", "border-style", "border-color", "border-top",
    "border-right", "border-bottom", "border-left", "border-radius",
    "outline", "outline-width", "outline-style", "outline-color",
    // typography
    "color", "font", "font-family", "font-size", "font-weight", "font-style",
    "font-variant", "line-height", "letter-spacing", "word-spacing",
    "text-align", "text-decoration", "text-transform", "text-indent",
    "text-shadow", "text-overflow", "white-space", "word-wrap", "word-break",
    "writing-mode", "vertical-align",
    // background
    "background", "background-color", "background-image", "background-repeat",
    "background-position", "background-size", "background-attachment",
    "background-clip", "background-origin",
    // effects
    "opacity", "filter", "backdrop-filter", "mix-blend-mode", "box-shadow",
    "transform", "transform-origin", "perspective", "transform-style",
    "transition", "transition-property", "transition-duration",
    "transition-timing-function", "transition-delay",
    "animation", "animation-name", "animation-duration",
    "animation-timing-function", "animation-delay", "animation-iteration-count",
    "animation-direction", "animation-fill-mode", "animation-play-state",
    // interactivity
    "cursor", "pointer-events", "user-select", "resize", "touch-action",
    "scroll-behavior", "scroll-snap-type", "scroll-snap-align",
    // values / units
    "auto", "none", "inherit", "initial", "unset", "revert",
    "block", "inline", "inline-block", "flex", "grid", "table", "table-row",
    "table-cell", "contents",
    "absolute", "relative", "fixed", "sticky", "static",
    "hidden", "visible", "scroll", "clip",
    "hover", "focus", "active", "visited", "first-child", "last-child",
    "nth-child", "before", "after",
    "important", "currentColor", "transparent",
    "px", "em", "rem", "vh", "vw", "vmin", "vmax", "fr", "ch", "ex",
];

// --- SCSS ---
// CSS set + Sass-specific at-rules and directives.
const SCSS: &[&str] = &[
    // CSS subset (most used)
    "display", "position", "color", "background", "background-color",
    "font", "font-family", "font-size", "font-weight", "margin", "padding",
    "border", "border-radius", "width", "height", "flex", "grid",
    "justify-content", "align-items", "gap", "transform", "transition",
    "opacity", "z-index", "overflow", "cursor", "box-shadow", "text-align",
    "line-height", "auto", "none", "inherit", "initial", "block", "inline",
    "absolute", "relative", "fixed", "hidden", "visible", "hover", "focus",
    "important",
    // Sass at-rules
    "@import", "@use", "@forward", "@mixin", "@include", "@function",
    "@return", "@if", "@else", "@each", "@for", "@while", "@extend",
    "@at-root", "@debug", "@warn", "@error", "@media", "@supports",
    "@keyframes", "@charset", "@namespace", "@page",
    // Sass builtins / directives
    "from", "through", "to", "in", "and", "or", "not", "true", "false", "null",
    // Sass modules
    "math", "color", "list", "map", "meta", "selector", "string",
];

// --- YAML ---
// YAML 1.2 has a tiny reserved set (boolean / null literals). Indicators
// (`---`, `&`, `*`, `!`, `|`, `>`) aren't identifiers so they're excluded.
const YAML: &[&str] = &[
    "true", "false", "null", "yes", "no", "on", "off", "True", "False",
    "Null", "Yes", "No", "On", "Off", "TRUE", "FALSE", "NULL", "YES", "NO",
    "ON", "OFF",
];

// --- TOML ---
// TOML's only reserved identifiers are the boolean literals; everything else
// is structural punctuation.
const TOML: &[&str] = &["true", "false", "inf", "nan"];

// --- XML ---
// XML declarations / DTD keywords from
// https://www.w3.org/TR/xml/. Element/attribute names are vocabulary-
// specific so not enumerated here.
const XML: &[&str] = &[
    "xml", "version", "encoding", "standalone", "xmlns",
    "DOCTYPE", "ELEMENT", "ATTLIST", "ENTITY", "NOTATION", "PCDATA",
    "CDATA", "INCLUDE", "IGNORE", "EMPTY", "ANY", "PUBLIC", "SYSTEM",
    "NDATA", "REQUIRED", "IMPLIED", "FIXED", "NMTOKEN", "NMTOKENS", "ID",
    "IDREF", "IDREFS", "ENTITIES",
];

// --- SQL ---
// SQL:2016 reserved keywords (deduped subset covering ANSI SQL + most
// vendor extensions). Uppercase + lowercase variants so completion works
// regardless of the casing convention in the file.
const SQL: &[&str] = &[
    // DML
    "SELECT", "FROM", "WHERE", "INSERT", "UPDATE", "DELETE", "INTO",
    "VALUES", "SET", "RETURNING",
    // joins / clauses
    "JOIN", "INNER", "LEFT", "RIGHT", "FULL", "OUTER", "CROSS", "ON", "USING",
    "AS", "DISTINCT", "ALL", "ANY", "SOME",
    // boolean / null
    "AND", "OR", "NOT", "NULL", "IS", "IN", "BETWEEN", "LIKE", "ILIKE",
    "EXISTS", "TRUE", "FALSE",
    // ordering + grouping
    "ORDER", "BY", "GROUP", "HAVING", "LIMIT", "OFFSET", "FETCH", "FIRST",
    "NEXT", "ROWS", "ONLY",
    // set ops
    "UNION", "INTERSECT", "EXCEPT", "MINUS",
    // DDL
    "CREATE", "ALTER", "DROP", "TRUNCATE", "RENAME",
    "TABLE", "INDEX", "VIEW", "DATABASE", "SCHEMA", "SEQUENCE", "TRIGGER",
    "FUNCTION", "PROCEDURE",
    // constraints
    "PRIMARY", "FOREIGN", "KEY", "REFERENCES", "CONSTRAINT", "UNIQUE",
    "CHECK", "DEFAULT", "CASCADE", "RESTRICT", "ACTION",
    // control flow
    "CASE", "WHEN", "THEN", "ELSE", "END", "IF", "BEGIN", "DECLARE",
    "WHILE", "LOOP", "FOR", "EXIT", "CONTINUE",
    // transactions
    "COMMIT", "ROLLBACK", "TRANSACTION", "SAVEPOINT", "RELEASE",
    // permissions
    "GRANT", "REVOKE", "WITH", "OPTION", "ADMIN",
    // types
    "INT", "INTEGER", "BIGINT", "SMALLINT", "TINYINT", "FLOAT", "DOUBLE",
    "REAL", "DECIMAL", "NUMERIC", "VARCHAR", "CHAR", "TEXT", "BLOB",
    "DATE", "TIME", "TIMESTAMP", "DATETIME", "INTERVAL", "BOOLEAN", "BIT",
    "JSON", "JSONB", "UUID",
    // lowercase variants
    "select", "from", "where", "insert", "update", "delete", "into",
    "values", "set", "join", "inner", "left", "right", "full", "outer",
    "on", "as", "distinct", "and", "or", "not", "null", "is", "in",
    "between", "like", "exists", "true", "false", "order", "by", "group",
    "having", "limit", "offset", "union", "create", "alter", "drop",
    "table", "index", "view", "primary", "foreign", "key", "references",
    "unique", "case", "when", "then", "else", "end", "if", "begin",
    "commit", "rollback", "int", "integer", "varchar", "char", "text",
    "date", "time", "timestamp", "boolean",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_language_has_a_match_arm() {
        // Touch every variant so adding a new LanguageId without updating
        // `keywords_for` fails CI here instead of silently returning empty.
        for lang in LanguageId::all() {
            let _ = keywords_for(*lang);
        }
    }

    #[test]
    fn rust_includes_core_keywords() {
        let kw = keywords_for(LanguageId::Rust);
        for must in &["fn", "let", "match", "impl", "trait", "Self"] {
            assert!(kw.contains(must), "Rust keywords missing {must}");
        }
    }

    #[test]
    fn python_includes_core_keywords() {
        let kw = keywords_for(LanguageId::Python);
        for must in &["def", "class", "import", "return", "self", "print"] {
            assert!(kw.contains(must), "Python keywords missing {must}");
        }
    }

    #[test]
    fn markdown_is_empty() {
        assert!(keywords_for(LanguageId::Markdown).is_empty());
    }
}
