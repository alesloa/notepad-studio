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
are retained for parameter hints; the parameter-hint popup is still pending.

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
