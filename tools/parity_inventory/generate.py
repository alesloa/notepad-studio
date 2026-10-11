"""Inventory factual menu metadata and local Rust routes, without copying oracle code."""

import argparse
import ast
from collections import Counter, defaultdict
import hashlib
import json
from pathlib import Path
import re


ROOT = Path(__file__).resolve().parents[2]
STRING = r'"(?:\\[\s\S]|[^"\\])*"'
ALIASES = {
    "IDM_EDIT_UNDO": "SCI_UNDO",
    "IDM_EDIT_REDO": "SCI_REDO",
    "IDM_EDIT_DELETE": "SCI_CLEAR",
    "IDM_EDIT_SELECTALL": "SCI_SELECTALL",
    "IDM_VIEW_ZOOMIN": "SCI_ZOOMIN",
    "IDM_VIEW_ZOOMOUT": "SCI_ZOOMOUT",
    "IDM_VIEW_ZOOMRESTORE": "SCI_SETZOOM",
}
REVIEWED = {
    "IDM_EDIT_BEGINENDSELECT_COLUMNMODE": ("partial", "The registered handler falls back to linear selection and reports column mode unsupported; src/edit_ops/begin_end_select.rs:63-80."),
    "IDM_FORMAT_ANSI": ("partial", "The catalog maps ANSI to Latin-1. Oracle ANSI uses the system code page and has a UTF-8-system guard, NppCommands.cpp:2997-3002. Studio changes the save target only, format.rs:71-88."),
    "IDM_FORMAT_AS_UTF_8": ("partial", "Save-target change only. Oracle dirty-state rules and explicit-code-page reload branch are not represented, NppCommands.cpp:3013-3016,3040-3096; format.rs:71-88."),
    "IDM_FORMAT_UTF_8": ("partial", "Save-target change only. Oracle explicit-code-page reload branch is not represented, NppCommands.cpp:3040-3096; format.rs:71-88."),
    "IDM_FORMAT_UTF_16BE": ("partial", "Save-target change only. Oracle explicit-code-page reload branch is not represented, NppCommands.cpp:3040-3096; format.rs:71-88."),
    "IDM_FORMAT_UTF_16LE": ("partial", "Save-target change only. Oracle explicit-code-page reload branch is not represented, NppCommands.cpp:3040-3096; format.rs:71-88."),
    "IDM_MACRO_STARTRECORDINGMACRO": ("action-only", "MacroStartRecording is declared and registered, but the menu uses the combined Start/Stop toggle and there is no dedicated catalog row."),
    "IDM_MACRO_STOPRECORDINGMACRO": ("action-only", "MacroStopRecording is declared and registered, but the menu uses the combined Start/Stop toggle and there is no dedicated catalog row."),
    "IDM_EXECUTE_VALIDATE_SHORTCUTSXML": ("status-only", "The registered handler only explains shortcuts.toml validation and does not validate a shortcuts.xml file."),
    "IDM_FILE_RENAME": ("alternate-only", "Tab popup Rename calls open_tab_rename; the static File menu, action and catalog route are missing."),
    "IDM_FILE_DELETE": ("alternate-only", "Tab popup Move to Recycle Bin calls move_tab_to_trash; the static File menu, action and catalog route are missing."),
    "IDM_FILE_PRINT": ("stub", "No static File menu/action/catalog route. The tab popup Print caller only reports that printing is not yet implemented."),
    "IDM_VIEW_GOTO_NEW_INSTANCE": ("alternate-only", "Tab popup Move to New Instance calls move_tab_to_new_window at tab_context_menu.rs:758; the required View menu/action/catalog route is missing."),
    "IDM_VIEW_LOAD_IN_NEW_INSTANCE": ("alternate-only", "Tab popup Open in New Instance calls move_tab_to_new_window at tab_context_menu.rs:775; the required View menu/action/catalog route is missing."),
    "IDM_EDIT_TOGGLESYSTEMREADONLY": ("partial", "Related Clear Read-Only Flag only clears the attribute and current in-memory flag. The oracle Windows attribute command toggles the attribute."),
    "IDM_SEARCH_CLEARALLMARKS": ("partial", "Related Clear all marks clears the single Find mark collection. Five independent token-style collections and the required submenu are absent."),
    "IDM_DROPLIST_LIST": ("missing", "Static disabled placeholder populated dynamically in Notepad++; dynamic contents are outside this inventory."),
    "IDM_WINDOW_MRU_FIRST": ("missing", "Static disabled placeholder populated dynamically in Notepad++; dynamic contents are outside this inventory."),
}
RELATED_ACTIONS = {
    "IDM_MACRO_STARTRECORDINGMACRO": "MacroStartRecording",
    "IDM_MACRO_STOPRECORDINGMACRO": "MacroStopRecording",
    "IDM_EDIT_TOGGLESYSTEMREADONLY": "EditClearReadOnlyFlag",
    "IDM_SEARCH_CLEARALLMARKS": "MarkClearAll",
}
ALTERNATE_FUNCTIONS = {
    "IDM_FILE_RENAME": "open_tab_rename",
    "IDM_FILE_DELETE": "move_tab_to_trash",
    "IDM_VIEW_GOTO_NEW_INSTANCE": "move_tab_to_new_window",
    "IDM_VIEW_LOAD_IN_NEW_INSTANCE": "move_tab_to_new_window",
}
MENU_NAMES = {"macros": "Macro", "help": "?", "bookmarks": "Search > Bookmark", "app_menu": "Notepad Studio"}
KEY_NAMES = {"NULL": None, "BACK": "backspace", "RETURN": "enter", "PRIOR": "pageup", "NEXT": "pagedown", "ADD": "numpadadd", "SUBTRACT": "numpadsubtract", "DIVIDE": "numpaddivide", "MULTIPLY": "numpadmultiply", "OEM_1": ";", "OEM_2": "/", "OEM_3": "`", "OEM_4": "[", "OEM_5": "\\", "OEM_6": "]", "OEM_7": "'", "OEM_PLUS": "=", "OEM_MINUS": "-", "OEM_COMMA": ",", "OEM_PERIOD": "."}


def read(path):
    return path.read_text(encoding="utf-8-sig")


def audited_source_hashes(root):
    paths = sorted(p for p in (root / "src").rglob("*.rs") if "test" not in p.stem and "/keymap/" not in p.as_posix())
    return {p.relative_to(root).as_posix(): hashlib.sha256(p.read_bytes()).hexdigest() for p in paths}


def line_at(source, offset):
    return source.count("\n", 0, offset) + 1


def rust_string(value):
    value = re.sub(r"\\u\{([0-9a-fA-F]+)\}", lambda m: chr(int(m[1], 16)), value)
    return json.loads(value)


def strip_mnemonics(value):
    return value.replace("&&", "\0").replace("&", "").replace("\0", "&")


def mask(source):
    pattern = r"//[^\n]*|/\*[\s\S]*?\*/|(?:br|r)(?P<hashes>\#*)\"[\s\S]*?\"(?P=hashes)|\"(?:\\[\s\S]|[^\"\\])*\"|'(?:\\.|[^'\\\n])'"
    return re.sub(pattern, lambda m: re.sub(r"[^\n]", " ", m[0]), source)


def production_source(source):
    masked = mask(source)
    spans = []
    for match in re.finditer(r"^[ \t]*#\[cfg\(test\)\]", masked, re.M):
        suffix = masked[match.end():]
        item = re.match(r"\s*(?:#\[[^]]*\]\s*)*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?(fn|mod|struct|enum|impl|const|static|use|type)\b", suffix)
        if item is None:
            continue
        opening = masked.find("{", match.end())
        semicolon = masked.find(";", match.end())
        if semicolon != -1 and (opening == -1 or semicolon < opening):
            end = semicolon + 1
        elif opening != -1:
            end = matching_end(masked, opening, "{", "}") + 1
        else:
            raise ValueError("Unparsed cfg(test) item")
        spans.append((match.start(), end))
    for start, end in reversed(spans):
        source = source[:start] + re.sub(r"[^\n]", " ", source[start:end]) + source[end:]
    return source


def matching_end(source, start, opening="(", closing=")"):
    depth = 0
    for i in range(start, len(source)):
        if source[i] == opening:
            depth += 1
        elif source[i] == closing:
            depth -= 1
            if depth == 0:
                return i
    raise ValueError(f"Unclosed {opening} at offset {start}")


def split_args(source):
    masked = mask(source)
    depth = 0
    start = 0
    result = []
    for i, char in enumerate(masked):
        if char in "([{":
            depth += 1
        elif char in ")]}":
            depth -= 1
        elif char == "," and depth == 0:
            result.append(source[start:i].strip())
            start = i + 1
    result.append(source[start:].strip())
    return result


def numeric_ids(source):
    expressions = dict(re.findall(r"^\s*#define\s+(\w+)\s+([^\r\n]+)", source, re.M))
    resolved = {}

    def evaluate(node):
        if isinstance(node, ast.Constant) and isinstance(node.value, int):
            return node.value
        if isinstance(node, ast.Name):
            return resolve(node.id)
        if isinstance(node, ast.BinOp) and isinstance(node.op, (ast.Add, ast.Sub)):
            lhs, rhs = evaluate(node.left), evaluate(node.right)
            return lhs + rhs if isinstance(node.op, ast.Add) else lhs - rhs
        raise ValueError(f"Unsupported numeric ID expression: {ast.dump(node)}")

    def resolve(name):
        if name not in resolved:
            expression = expressions[name].split("//", 1)[0].strip()
            resolved[name] = evaluate(ast.parse(expression, mode="eval").body)
        return resolved[name]

    return resolve


def oracle_menu(source):
    start = re.search(r"^IDR_M30_MENU MENU\s*$", source, re.M)
    if start is None:
        raise ValueError("IDR_M30_MENU not found")
    stack = []
    pending = None
    commands, separators = [], []
    top_counts = Counter()
    conditionals = []
    end_line = None
    for offset, original in enumerate(source[start.end():].splitlines(), line_at(source, start.end())):
        line = original.strip()
        if not line or line.startswith("//"):
            continue
        if line.startswith("#"):
            conditionals.append((offset, line))
            continue
        if line == "BEGIN":
            stack.append(pending)
            pending = None
        elif line == "END":
            if not stack:
                raise ValueError("Unbalanced oracle menu END")
            stack.pop()
            if not stack:
                end_line = offset
                break
        elif line.startswith("POPUP"):
            found = re.fullmatch(rf"POPUP\s+({STRING})", line)
            if found is None:
                raise ValueError(f"Unparsed popup at {offset}: {line}")
            pending = strip_mnemonics(json.loads(found[1]))
            if len(stack) == 1:
                top_counts[pending] += 1
                if pending == "Language":
                    pending += " (flat)" if top_counts["Language"] == 1 else " (compact)"
        elif line == "MENUITEM SEPARATOR":
            separators.append({"line": offset, "path": " > ".join(x for x in stack if x)})
        elif line.startswith("MENUITEM"):
            found = re.fullmatch(rf"MENUITEM\s+({STRING})\s*,\s*(\w+)(?:,\s*(.*))?", line)
            if found is None:
                raise ValueError(f"Unparsed menu command at {offset}: {line}")
            label = strip_mnemonics(json.loads(found[1]))
            path = [x for x in stack if x] + [label]
            commands.append({"index": len(commands) + 1, "id": found[2], "label": label, "path": " > ".join(path), "top": path[0] if len(path) > 1 else "Menu-bar controls", "rc_line": offset, "flags": found[3] or ""})
        else:
            raise ValueError(f"Unparsed oracle menu line {offset}: {line}")
    if end_line is None or conditionals:
        raise ValueError(f"Unclosed or conditionally compiled menu: {conditionals}")
    raw_block = source[start.end():].splitlines()[:end_line - line_at(source, start.end()) + 1]
    raw_count = sum(bool(re.match(r'\s*MENUITEM\s+"', line)) for line in raw_block)
    if raw_count != len(commands):
        raise ValueError(f"Independent MENUITEM count differs: {raw_count} != {len(commands)}")
    return commands, separators, line_at(source, start.start()), end_line


def chord(vkey, ctrl, alt, shift):
    name = vkey.removeprefix("VK_")
    key = KEY_NAMES.get(name, name.lower())
    if vkey == "0" or key is None:
        return None
    if name.startswith("NUMPAD"):
        key = name.lower()
    return "".join(prefix + "-" for enabled, prefix in [(ctrl, "ctrl"), (alt, "alt"), (shift, "shift")] if enabled == "true") + key


def oracle_keys(source):
    keys = defaultdict(list)
    scint = defaultdict(list)
    redirects = {}
    for match in re.finditer(r"^[ \t]*\{\s*(VK_\w+|0),\s*(\w+),\s*(true|false),\s*(true|false),\s*(true|false),", source, re.M):
        value = chord(match[1], match[3], match[4], match[5])
        keys[match[2]].append({"chord": value, "line": line_at(source, match.start()), "source_id": match[2]})
    pattern = rf"^[ \t]*\{{\s*L({STRING}),\s*(SCI_\w+),\s*(true|false),\s*(true|false),\s*(true|false),\s*(VK_\w+|0),\s*(\w+)\s*\}}"
    for match in re.finditer(pattern, source, re.M):
        value = chord(match[6], match[3], match[4], match[5])
        scint[match[2]].append({"chord": value, "line": line_at(source, match.start()), "source_id": match[2]})
        if match[7].startswith("IDM_"):
            redirects[match[7]] = match[2]
    for command, sci in redirects.items():
        keys[command].extend(scint[sci])
    return keys


def functions(path, source):
    masked = mask(source)
    result = []
    for match in re.finditer(r"\bfn\s+(\w+)\s*(?:<[^{};]*?>)?\s*\(", masked):
        opening = match.end() - 1
        try:
            end = matching_end(masked, opening)
        except ValueError as error:
            raise ValueError(f"{path}::{match[1]} signature: {error}") from error
        body_start = masked.find("{", end)
        semicolon = masked.find(";", end)
        if body_start == -1 or (semicolon != -1 and semicolon < body_start):
            continue
        try:
            body_end = matching_end(masked, body_start, "{", "}")
        except ValueError as error:
            raise ValueError(f"{path}::{match[1]}: {error}") from error
        actions = re.findall(r":\s*&(?:crate::actions::)?([A-Z]\w*)", masked[opening:end])
        result.append({"name": match[1], "path": path, "line": line_at(source, match.start()), "start": match.start(), "end": body_end + 1, "actions": actions, "body": source[body_start:body_end + 1]})
    return result


def studio_routes(root):
    catalog_source = read(root / "src/shortcuts.rs")
    catalog = {}
    string_list = rf"((?:\s*{STRING}\s*,?\s*)*)"
    pattern = rf"row!\(\s*(\w+),\s*({STRING}),\s*(Some\({STRING}\)|EXTRA),\s*(\w+),\s*\[{string_list}\],\s*\[{string_list}\]\s*\)"
    for match in re.finditer(pattern, catalog_source):
        npp = re.search(STRING, match[3])
        catalog[match[1]] = {"action": match[1], "label": rust_string(match[2]), "npp": rust_string(npp[0]) if npp else None, "context": match[4], "windows": [rust_string(x) for x in re.findall(STRING, match[5])], "mac": [rust_string(x) for x in re.findall(STRING, match[6])], "line": line_at(catalog_source, match.start())}
    if len(catalog) != len(re.findall(r"^\s*row!\(", catalog_source, re.M)):
        raise ValueError("A catalog row was not parsed")
    source_hashes = audited_source_hashes(root)
    sources = {path: production_source(read(root / path)) for path in source_hashes}
    definitions = defaultdict(list)
    action_defs = {}
    for path, source in sources.items():
        for fn in functions(path, source):
            definitions[fn["name"]].append(fn)
        if path.startswith("src/actions/"):
            for match in re.finditer(r"\b[A-Z]\w*\b", mask(source)):
                action_defs.setdefault(match[0], {"path": path, "line": line_at(source, match.start())})
    language = sources["src/workspace/language.rs"]
    for match in re.finditer(r"^[ \t]*(language_\w+),\s*(Language\w+),", language, re.M):
        definitions[match[1]].append({"name": match[1], "path": "src/workspace/language.rs", "line": line_at(language, match.start()), "actions": [match[2]], "body": "macro expansion calls set_language", "start": match.start(), "end": match.end()})
    handlers = defaultdict(list)
    render = sources["src/workspace/render.rs"]
    for match in re.finditer(r"\.on_action\(cx\.listener\(Self::(\w+)\)\)", render):
        candidates = definitions.get(match[1], [])
        for fn in candidates:
            for action in fn["actions"]:
                if action in action_defs:
                    handlers[action].append({"method": match[1], "path": fn["path"], "line": fn["line"], "wire_path": "src/workspace/render.rs", "wire_line": line_at(render, match.start())})
    globals_source = sources["src/app/menus.rs"]
    for match in re.finditer(r"cx\.on_action\(\|[^|]*:\s*&([A-Z]\w+),", globals_source):
        handlers[match[1]].append({"method": "global action closure", "path": "src/app/menus.rs", "line": line_at(globals_source, match.start()), "wire_path": "src/app/menus.rs", "wire_line": line_at(globals_source, match.start())})
    menus = defaultdict(list)
    for path, source in sources.items():
        if not path.startswith("src/app/menus/") or "test" in path or "fixture" in path:
            continue
        source = re.split(r"#\[cfg\(test\)\]\s*mod tests", source)[0]
        top = MENU_NAMES.get(Path(path).stem, Path(path).stem.title())
        masked = mask(source)
        menu_blocks = []
        for menu_match in re.finditer(r"\bMenu\s*\{", masked):
            if masked[:menu_match.start()].rstrip().endswith("->"):
                continue
            menu_end = matching_end(masked, menu_match.end() - 1, "{", "}")
            fields = split_args(source[menu_match.end():menu_end])
            name = next((match for field in fields if (match := re.match(rf"\s*name:\s*({STRING})", field))), None)
            if name:
                menu_blocks.append((menu_match.start(), menu_end, strip_mnemonics(rust_string(name[1]))))
        for match in re.finditer(r"MenuItem::action\s*\(", masked):
            end = matching_end(masked, match.end() - 1)
            args = split_args(source[match.end():end])
            if len(args) < 2:
                raise ValueError(f"Menu action arguments missing at {path}:{line_at(source, match.start())}")
            action_match = re.match(r"(?:\w+::)*(\w+)", args[1])
            if action_match is None:
                raise ValueError(f"Menu action type unparsed at {path}:{line_at(source, match.start())}")
            action = action_match[1]
            label = rust_string(args[0]) if re.fullmatch(STRING, args[0]) else "<computed label>"
            submenus = [name for start, end, name in menu_blocks if start <= match.start() < end and name not in top.split(" > ")]
            menu_path = " > ".join([top] + submenus + [strip_mnemonics(label)])
            if action == "FileOpenContainingFolder":
                menu_path = "File > Open Containing Folder > Explorer / Finder / File Manager"
            menus[action].append({"path": path, "line": line_at(source, match.start()), "menu_path": menu_path, "label": label})
    return catalog, action_defs, handlers, menus, definitions, source_hashes


def build_inventory(root, oracle):
    rc = read(oracle / "Notepad_plus.rc")
    entries, separators, start, end = oracle_menu(rc)
    ids = numeric_ids(read(oracle / "menuCmdID.h"))
    parameters = read(oracle / "Parameters.cpp")
    defaults = oracle_keys(parameters)
    commands = read(oracle / "NppCommands.cpp")
    dispatches = defaultdict(list)
    for match in re.finditer(r"\bcase\s+(IDM_\w+)\s*:", mask(commands)):
        dispatches[match[1]].append(line_at(commands, match.start()))
    catalog, action_defs, handlers, menus, definitions, source_hashes = studio_routes(root)
    by_npp = {row["npp"]: action for action, row in catalog.items() if row["npp"]}
    for entry in entries:
        command = entry["id"]
        if command not in rc.splitlines()[entry["rc_line"] - 1]:
            raise ValueError(f"Oracle resource line reference is wrong: {command}")
        entry["numeric_id"] = ids(command)
        entry["default_keys"] = defaults.get(command, [])
        for key in entry["default_keys"]:
            if key["source_id"] not in parameters.splitlines()[key["line"] - 1]:
                raise ValueError(f"Oracle shortcut line reference is wrong: {command}")
        entry["npp_dispatch_lines"] = dispatches.get(command, [])
        key = ALIASES.get(command, command)
        action = by_npp.get(key, RELATED_ACTIONS.get(command))
        entry["mapping"] = "Scintilla redirected command" if command in ALIASES else "catalog command ID"
        if command in RELATED_ACTIONS:
            entry["mapping"] = "manually reviewed related action"
        entry["studio_action"] = action
        entry["studio_declaration"] = action_defs.get(action)
        entry["studio_catalog"] = catalog.get(action)
        entry["studio_menu"] = menus.get(action, [])
        entry["studio_handlers"] = handlers.get(action, [])
        entry["alternate"] = []
        if command in ALTERNATE_FUNCTIONS:
            entry["alternate"] = [{"path": fn["path"], "line": fn["line"], "method": fn["name"]} for fn in definitions[ALTERNATE_FUNCTIONS[command]]]
        if action and entry["studio_declaration"] and entry["studio_catalog"] and entry["studio_menu"] and entry["studio_handlers"]:
            entry["status"] = "routed"
            entry["note"] = "Connected production route; full oracle behavior and native visuals are unverified by this static audit."
        elif action:
            entry["status"] = "incomplete"
            entry["note"] = "Mapped action lacks at least one declaration, catalog, static-menu or registered-handler link."
        else:
            entry["status"] = "missing"
            entry["note"] = "No mapped static main-menu/catalog action found; this does not prove the behavior is absent from every other UI."
        if command in REVIEWED:
            entry["status"], entry["note"] = REVIEWED[command]
    counts = Counter(entry["id"] for entry in entries)
    unique = list({entry["id"]: entry for entry in entries}.values())
    result = {
        "studio_source_sha256": hashlib.sha256(json.dumps(source_hashes, sort_keys=True).encode("utf-8")).hexdigest(),
        "studio_sources": source_hashes,
        "oracle_sources": {name: hashlib.sha256((oracle / name).read_bytes()).hexdigest() for name in ["Notepad_plus.rc", "menuCmdID.h", "Parameters.cpp", "NppCommands.cpp", "Notepad_plus.cpp", "MISC/Common/Common.cpp"]},
        "rc_start": start, "rc_end": end,
        "summary": {"appearances": len(entries), "unique_ids": len(counts), "duplicate_appearances": len(entries) - len(counts), "separators": len(separators), "catalog_rows": len(catalog), "conditional_directives_in_resource": 0, "appearance_statuses": dict(sorted(Counter(x["status"] for x in entries).items())), "unique_statuses": dict(sorted(Counter(x["status"] for x in unique).items())), "top_menus": dict(Counter(x["top"] for x in entries)), "duplicates": {key: value for key, value in counts.items() if value > 1}},
        "entries": entries,
        "separators": separators,
    }
    if sum(result["summary"]["appearance_statuses"].values()) != len(entries):
        raise ValueError("Status count does not cover every appearance")
    if any(entry["status"] == "incomplete" for entry in entries):
        raise ValueError("A mapped route was not fully classified")
    return result


def source_link(path, line, label):
    return f"[{label}](../{path}#L{line})"


def cell(value):
    return str(value).replace("|", "\\|").replace("\n", " ")


def render_table(inventory):
    lines = []
    top = None
    for entry in inventory["entries"]:
        if entry["top"] != top:
            top = entry["top"]
            lines += [f"### {top}", "", "| # | Oracle ID / number | Full oracle path / RC line | Default Windows chord / Parameters line | Notepad Studio route evidence | Class |", "| --- | --- | --- | --- | --- | --- |"]
        defaults = entry["default_keys"]
        chord_values = [f"`{x['chord']}` @{x['line']}" for x in defaults if x["chord"]]
        if not chord_values:
            chord_values = ["Unassigned" + (" @" + ",".join(str(x["line"]) for x in defaults) if defaults else "; no key-table row")]
        route = []
        if entry["studio_action"]:
            action = entry["studio_action"]
            declaration = entry["studio_declaration"]
            route.append(source_link(declaration["path"], declaration["line"], action) if declaration else action)
            for menu in entry["studio_menu"]:
                route.append(source_link(menu["path"], menu["line"], menu["menu_path"]))
            catalog = entry["studio_catalog"]
            if catalog:
                route.append(source_link("src/shortcuts.rs", catalog["line"], "CATALOG") + " Win " + (", ".join(f"`{x}`" for x in catalog["windows"]) or "none") + "; Mac " + (", ".join(f"`{x}`" for x in catalog["mac"]) or "none"))
            for handler in entry["studio_handlers"]:
                route.append(source_link(handler["path"], handler["line"], handler["method"]) + " / " + source_link(handler["wire_path"], handler["wire_line"], "wire"))
        for alternate in entry["alternate"]:
            route.append(source_link(alternate["path"], alternate["line"], alternate["method"]))
        if entry["npp_dispatch_lines"]:
            route.append("NppCommands case @" + ",".join(map(str, entry["npp_dispatch_lines"])))
        route.append(entry["note"])
        values = [entry["index"], f"`{entry['id']}` / {entry['numeric_id']}", f"{entry['path']} @{entry['rc_line']}" + (f" [{entry['flags']}]" if entry["flags"] else ""), "<br>".join(chord_values), "<br>".join(route), entry["status"]]
        lines.append("| " + " | ".join(cell(value) for value in values) + " |")
        next_index = entry["index"]
        if next_index == len(inventory["entries"]) or inventory["entries"][next_index]["top"] != top:
            lines.append("")
    return "\n".join(lines).rstrip()


def render_intro(inventory):
    summary = inventory["summary"]
    count_lines = [f"Audited Notepad Studio Rust source SHA-256 `{inventory['studio_source_sha256']}`, covering {len(inventory['studio_sources'])} source files. The factual JSON records every file hash. The resource spans RC lines {inventory['rc_start']}-{inventory['rc_end']}.", "", f"The resource contains **{summary['appearances']} command appearances, {summary['unique_ids']} unique command IDs and {summary['separators']} separators**. The {summary['duplicate_appearances']} extra appearances are the second Language layout's 98 IDs and the menu-bar New/Close controls. The catalog has {summary['catalog_rows']} rows, including commands outside this static resource and Notepad Studio extras.", "", "| Classification | Static appearances | Unique command IDs |", "| --- | ---: | ---: |"]
    for status, count in summary["appearance_statuses"].items():
        count_lines.append(f"| {status} | {count} | {summary['unique_statuses'][status]} |")
    count_lines += ["", "| Static menu | Command appearances |", "| --- | ---: |"]
    count_lines.extend(f"| {name} | {count} |" for name, count in summary["top_menus"].items())
    source_lines = ["The factual JSON records every row, separators, duplicate IDs and source SHA-256 hashes. The following hashes pin the oracle used here.", "", "| Oracle file | SHA-256 |", "| --- | --- |"]
    source_lines.extend(f"| `{name}` | `{value}` |" for name, value in inventory["oracle_sources"].items())
    return read(ROOT / "tools/parity_inventory/report_intro.md").replace("<!-- INVENTORY_COUNTS -->", "\n".join(count_lines)).replace("<!-- INVENTORY_SOURCES -->", "\n".join(source_lines))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--oracle", type=Path, required=True, help="Notepad++ PowerEditor/src directory")
    parser.add_argument("--check", action="store_true", help="Compare generated factual outputs with checked-in files")
    args = parser.parse_args()
    inventory = build_inventory(ROOT, args.oracle)
    outputs = {ROOT / "tools/parity_inventory/inventory.json": json.dumps(inventory, indent=2, ensure_ascii=False) + "\n", ROOT / "docs/notepadpp-parity-inventory.md": render_intro(inventory) + "\n" + render_table(inventory) + "\n"}
    for path, expected in outputs.items():
        if args.check:
            if read(path) != expected:
                raise SystemExit(f"Generated inventory differs: {path}")
        else:
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(expected, encoding="utf-8", newline="\n")
    print(json.dumps(inventory["summary"], indent=2))


if __name__ == "__main__":
    main()
