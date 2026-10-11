# Regex replacement

In **Regular expression** mode, Replace, Replace All, All Opened, and Replace
in Files expand captures from each fresh match. Normal mode inserts literal
replacement text. Extended mode keeps its existing escape decoding and does
not expand captures. CSV grid replacement remains literal.

Supported replacement forms:

| Form | Result |
| --- | --- |
| `\1` through `\9`, `$1`, `${1}` | Numbered capture |
| `$+{name}`, `${name}`, `$name` | Named capture |
| `$0`, `${0}`, `$&` | Whole match |
| `$$`, `\$` | Literal dollar |
| `\\` | Literal backslash |
| `\(`, `\)`, `\?` | Literal punctuation |
| `\n`, `\r`, `\t`, `\a`, `\e`, `\f`, `\v` | Control character |

For search `(\w+)-(\d+)`, replacing with `\2:${1}` changes `item-42` to
`42:item`. An existing optional capture that did not participate expands to
empty text. An undefined capture is an error. Braces disambiguate a capture
from following letters: `${1}suffix`. Existing Rust `$name` spellings from
Find in Files remain supported. See the [Rust captures documentation](https://docs.rs/regex/latest/regex/struct.Captures.html#method.expand).

This is a bounded subset, not Boost replacement compatibility. Case conversion,
conditional/grouped replacements, prefix/suffix/last-capture variables, and
numeric character escapes (including `\0`/octal) are rejected before editing.
`\b` inserts literal `b`. Escape literal
parentheses and question marks. Enter Unicode characters directly. The
[Notepad++ substitution reference](https://raw.githubusercontent.com/notepad-plus-plus/npp-usermanual/master/content/docs/searching.md)
describes the broader syntax. The capture behavior was also checked against
[Scintilla's regex target replacement](https://scintilla.org/ScintillaDoc.html#SCI_REPLACETARGETRE).
Notepad++ uses Boost's replacement format; its `\0` is a numeric/NUL escape,
while generic Scintilla uses that spelling for the whole match. Use `$0` or
`$&` here. See the [Boost format reference](https://www.boost.org/doc/libs/latest/libs/regex/doc/html/boost_regex/format/perl_format.html)
and its [replacement tests](https://raw.githubusercontent.com/boostorg/regex/develop/test/regress/test_replace.cpp).

Source planning, capture expansion, and the post-edit match scan run in the
background. Editing the query or template, changing a search flag, closing the
dialog, or switching tabs/views cancels pending work. A stale result or native
confirmation cannot replace newer document contents. Each edited document has
one Undo group; unchanged replacements preserve Redo. Zero-length replacement
matches are supported, including Unicode boundaries. Other Find navigation
still has its existing zero-length filtering.

The first editor batch accepts at most 4096 replacements across 256 requested
documents, 64 MiB of source, 1 MiB each of pattern/template, 16 MiB of expanded
text, and one million cached post-edit matches. Foreground application refuses
more than 4 MiB of changed old/new text or 262,144 estimated annotation visits.
The annotation count is a work proxy, not an exact operation count. Refusals
leave text, Undo, Redo, and CSV drafts unchanged; use a smaller selection.

Doc View prepares an AST candidate and checks the expanded output before
applying any edits. It supports paragraph/heading documents, up to 256
replacements per document, 1 MiB of visible text, a 2 MiB estimated AST,
65536 nodes, and prior history of 4096 edits/1 MiB. Aggregate application is
limited to 16 MiB of estimated AST/history copy work and 262,144 node visits.
Before cloning or flattening, aggregate capture is limited to 8 MiB of
estimated copies and 65,536 nodes. Read-only documents retain identity stamps
without capturing their text or AST.
Tables, images, and prior formatted-block paste history are explicitly refused
in this batch. These conservative limits keep history and formatting work
bounded; they do not measure native font shaping or painting.

Replace in Files uses the same template parser on its background worker, with
at most one million replacements per file. Its existing UTF-8-only file policy
is retained; UTF-8 BOM and line endings are preserved. Each file is planned
fully before an atomic write, and failures report the first affected path.
New Find/Replace recordings capture their parameters and support saved playback;
see [search macros](search-macros.md) for scope, cancellation and legacy limits.

Clean-room behavior references: `FindReplaceDlg.cpp:3198,3667`,
`ScintillaEditView.cpp:2954–2963`, and `boostregex/BoostRegExSearch.cxx:475–476`
in the local Notepad++ checkout. No Notepad++
implementation, bundled API data, or assets were copied.
