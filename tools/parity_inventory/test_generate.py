import json
from pathlib import Path
import tempfile
import unittest

import generate


class ResourceTests(unittest.TestCase):
    def test_whitespace_comments_unicode_duplicates_and_line_numbers(self):
        source = '\nIDR_M30_MENU MENU\nBEGIN\n POPUP "&File"\n BEGIN\n MENUITEM "&New" , IDM_FILE_NEW\n MENUITEM SEPARATOR\n //MENUITEM "Disabled", IDM_DISABLED\n END\n MENUITEM "＋", IDM_FILE_NEW, HELP\nEND\nOTHER MENU\n'
        entries, separators, start, end = generate.oracle_menu(source)
        self.assertEqual((start, end), (2, 11))
        self.assertEqual([(x["id"], x["path"], x["rc_line"]) for x in entries], [("IDM_FILE_NEW", "File > New", 6), ("IDM_FILE_NEW", "＋", 10)])
        self.assertEqual(separators, [{"line": 7, "path": "File"}])

    def test_unparsed_resource_line_fails(self):
        with self.assertRaisesRegex(ValueError, "Unparsed"):
            generate.oracle_menu('IDR_M30_MENU MENU\nBEGIN\nNOT_A_COMMAND\nEND\n')

    def test_numeric_id_arithmetic_and_aliases(self):
        resolve = generate.numeric_ids('#define BASE 40000\n#define FILE (BASE + 1000)\n#define NEW (FILE + 1)\n#define ALIAS NEW\n')
        self.assertEqual(resolve("NEW"), 41001)
        self.assertEqual(resolve("ALIAS"), 41001)

    def test_scintilla_redirect_keeps_every_alternative(self):
        source = '{ VK_N, IDM_NEW, true, false, false, nullptr },\n//{ VK_P, IDM_COMMENT, true, false, false, nullptr },\n{L"SCI_UNDO", SCI_UNDO, true, false, false, VK_Z, IDM_UNDO},\n{L"", SCI_UNDO, false, true, false, VK_BACK, 0},\n{ VK_NULL, IDM_EMPTY, false, false, false, nullptr },\n'
        keys = generate.oracle_keys(source)
        self.assertEqual([x["chord"] for x in keys["IDM_UNDO"]], ["ctrl-z", "alt-backspace"])
        self.assertEqual(keys["IDM_EMPTY"][0]["chord"], None)
        self.assertNotIn("IDM_COMMENT", keys)


class RustScannerTests(unittest.TestCase):
    def test_menu_return_types_do_not_reparent_sibling_items(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            for path in ["src/shortcuts.rs", "src/workspace/render.rs", "src/workspace/language.rs", "src/app/menus.rs"]:
                file = root / path
                file.parent.mkdir(parents=True, exist_ok=True)
                file.write_text("", encoding="utf-8")
            menu = root / "src/app/menus/file.rs"
            menu.parent.mkdir(parents=True)
            menu.write_text('''fn build() -> Menu {
 let items = vec![
  MenuItem::action("New", FileNew),
  MenuItem::submenu(Menu {
   name: "&Nested".into(),
   items: vec![MenuItem::action("Open", FileOpen), MenuItem::submenu(Menu {
    name: "Deep".into(), items: vec![MenuItem::action("Close", FileClose)],
   })],
  }),
  MenuItem::action("Save", FileSave),
 ];
 Menu { name: "&File".into(), items }
}
fn computed() -> Menu {
 Menu { name: dynamic_name(), items: vec![MenuItem::submenu(Menu {
  name: "Other".into(), items: vec![MenuItem::action("Exit", Quit)],
 })] }
}
''', encoding="utf-8")
            routes = generate.studio_routes(root)[3]
            self.assertEqual({action: values[0]["menu_path"] for action, values in routes.items()}, {
                "FileNew": "File > New", "FileOpen": "File > Nested > Open",
                "FileClose": "File > Nested > Deep > Close", "FileSave": "File > Save",
                "Quit": "File > Other > Exit",
            })

    def test_source_identity_is_independent_of_docs_and_git_metadata(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "src").mkdir()
            source = root / "src/main.rs"
            source.write_text("fn main() {}\n", encoding="utf-8")
            before = generate.audited_source_hashes(root)
            (root / "docs").mkdir()
            (root / "docs/report.md").write_text("New documentation\n", encoding="utf-8")
            (root / ".git").mkdir()
            (root / ".git/HEAD").write_text("An unrelated docs commit\n", encoding="utf-8")
            self.assertEqual(generate.audited_source_hashes(root), before)
            source.write_text("fn main() { changed(); }\n", encoding="utf-8")
            self.assertNotEqual(generate.audited_source_hashes(root), before)

    def test_literals_comments_and_escaped_newline_preserve_offsets(self):
        source = '''fn real(_: &MyAction) {
 let raw = r###" } " quote ( "###;
 let byte = br#" ) { "#;
 let brace = '}'; let paren = '(';
 let quoted = "escaped \\
 continuation }";
 /* } */ // )
 run();
}
'''
        masked = generate.mask(source)
        self.assertEqual(len(masked), len(source))
        self.assertEqual(masked.count("\n"), source.count("\n"))
        functions = generate.functions("sample.rs", source)
        self.assertEqual([(x["name"], x["actions"], x["line"]) for x in functions], [("real", ["MyAction"], 1)])
        self.assertIn("run();", functions[0]["body"])

    def test_test_items_do_not_count_but_parameters_and_fields_stay_parseable(self):
        source = '''#[cfg(test)]
mod tests { fn fake(_: &MyAction) {} }
#[cfg(test)]
#[allow(dead_code)]
pub(crate) fn fake2(_: &MyAction) {}
struct S { #[cfg(test)] extra: bool }
fn real(_: &MyAction, #[cfg(test)] probe: bool) {}
'''
        production = generate.production_source(source)
        self.assertEqual(len(production), len(source))
        self.assertEqual([x["name"] for x in generate.functions("sample.rs", production)], ["real"])


class InventoryTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.inventory = json.loads(Path(__file__).with_name("inventory.json").read_text(encoding="utf-8"))

    def test_resource_counts_and_duplicate_accounting(self):
        entries = self.inventory["entries"]
        ids = {x["id"] for x in entries}
        summary = self.inventory["summary"]
        self.assertEqual((len(entries), len(ids), len(self.inventory["separators"])), (581, 481, 45))
        self.assertEqual(sum(summary["top_menus"].values()), len(entries))
        self.assertEqual(sum(summary["appearance_statuses"].values()), len(entries))
        self.assertEqual(sum(summary["unique_statuses"].values()), len(ids))
        self.assertEqual(sum(x - 1 for x in summary["duplicates"].values()), 100)
        self.assertEqual(summary["top_menus"]["Language (flat)"], 98)
        self.assertEqual(summary["top_menus"]["Language (compact)"], 98)

    def test_literal_oracle_line_samples(self):
        samples = [(1, "IDM_FILE_NEW", 447), ("IDM_EDIT_COPY_ALL_NAMES", 511), ("IDM_SEARCH_TOGGLE_BOOKMARK", 731), ("IDM_EXECUTE", 1320)]
        entries = self.inventory["entries"]
        self.assertEqual((entries[0]["index"], entries[0]["id"], entries[0]["rc_line"]), samples[0])
        for command, line in samples[1:]:
            self.assertEqual(next(x["rc_line"] for x in entries if x["id"] == command), line)
        self.assertEqual(entries[-1]["rc_line"], 1371)

    def test_routed_requires_declaration_menu_catalog_and_production_handler(self):
        for entry in self.inventory["entries"]:
            if entry["status"] != "routed":
                continue
            with self.subTest(command=entry["id"]):
                self.assertTrue(entry["studio_declaration"])
                self.assertTrue(entry["studio_menu"])
                self.assertTrue(entry["studio_catalog"])
                self.assertTrue(entry["studio_handlers"])
                for handler in entry["studio_handlers"]:
                    self.assertNotIn("test", handler["path"])
                    self.assertNotIn("/keymap/", handler["path"])
                    self.assertIn(handler["wire_path"], ("src/workspace/render.rs", "src/app/menus.rs"))


if __name__ == "__main__":
    unittest.main()
