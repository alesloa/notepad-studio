# API completion definitions

Notepad Studio can load your language's API names from an optional XML file.
Place it in `<config>/Notepad Studio/autoCompletion/`:

| Platform | Config directory |
| --- | --- |
| Windows | `%APPDATA%` |
| macOS | `~/Library/Application Support` |
| Linux | `$XDG_CONFIG_HOME`, or `~/.config` when unset |

The active buffer's language determines the filename. Normal Text uses
`normal.xml`. Most languages use their lowercase name: `rust.xml`,
`python.xml`, `c.xml`, `cpp.xml`, `css.xml`, `go.xml`, `html.xml`, `java.xml`,
`json.xml`, `lua.xml`, `markdown.xml`, `php.xml`, `ruby.xml`, `scss.xml`,
`sql.xml`, `swift.xml`, `toml.xml`, `xml.xml`, and `yaml.xml`.

| Language | Filename |
| --- | --- |
| C# | `cs.xml` |
| JavaScript | `javascript.xml` |
| TypeScript | `typescript.xml` |
| TSX | `tsx.xml` |
| Bash/Sh/Zsh/Ksh | `bash.xml` |
| PowerShell | `powershell.xml` |

There are no bundled Notepad++ API files. Here is an original small example
you can adapt to your own API:

```xml
<?xml version="1.0" encoding="UTF-8"?>
<NotepadPlus>
  <AutoComplete language="rust">
    <Environment ignoreCase="yes" additionalWordChar="." />
    <KeyWord name="canvas.clear" func="yes">
      <Overload retVal="bool" descr="Clear the current drawing.">
        <Param name="color: Color" />
      </Overload>
    </KeyWord>
    <KeyWord name="Canvas" />
  </AutoComplete>
</NotepadPlus>
```

Function Completion inserts the selected name only. Function and Word
Completion also includes identifiers from the current buffer. The environment's
`ignoreCase` controls ASCII case matching; `additionalWordChar` adds characters
such as `.` to identifiers. Return types, descriptions and ordered overloads
appear in parameter hints. Edit → Auto-Completion → Function Parameters Hint
(`Ctrl+Shift+Space`) shows the open call's signature. Previous Hint (`Alt+Up`)
and Next Hint (`Alt+Down`), or the popup arrows, cycle through overloads.
The active argument is highlighted; long signatures show a bounded window
around that parameter. The popup stays inside the editor pane and avoids the
caret line and completion list.

With Function Parameters Hint enabled in Preferences, committed typing of
the call opener or parameter separator can open a hint. An existing hint
updates as the call changes. IME preedit hides hints; final Source commits
and unmarking can open them again. Escape, focus/window changes, Source/Raw
view transitions, tab changes, cursor movement and history/macro replay
dismiss hints. Hint navigation does not edit the document or add Undo entries.

Call scanning uses fresh text from the document's start on a background
executor. It scans at most 8 MiB before the caret, with nesting limited to
128 levels, and does not guess the lexical state of a truncated window.
A manual request reports when a limit prevents a hint. Completion lists
can open after a final Source commit or unmarking, using the configured mode
and character threshold. Preedit, cancellation, nonword text and selected
replacements do not open a new list.

Invoke an Edit → Auto-Completion command to reload an edited definition file.
Reads and parsing run in the background. Original static completion, or the
last valid definitions, stays available while loading. Automatic completion
uses a four-language cache, including missing-file results, until a manual
command reloads that language. A missing file uses original keywords. An invalid
file keeps the last valid definitions and reports the filename and error in the
status bar; Notepad Studio leaves the definition file unchanged.

An initial manual Function list includes other names when the typed prefix has
a match. You can choose a different visible name with the mouse or arrows.
After typing, acceptance requires a match for the current prefix. A prefix with
no matching name closes the list. Lists contain at most 50 names; buffer word
scans visit at most 128 KiB and skip identifiers longer than 256 bytes.

Definition files may be UTF-8 (with or without a BOM), or UTF-16 LE/BE with a BOM.
Files and decoded XML are limited to 8 MiB, nesting to eight levels, names to
100,000, overloads to 64 per function, and parameters to 128 per overload.
Attribute values are limited to 8 KiB and extra word characters to 128 bytes.
DTD declarations and custom entities are rejected.
