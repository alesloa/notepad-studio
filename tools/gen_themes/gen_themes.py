#!/usr/bin/env python3
# Apache-2.0. Generates Notepad Studio's bundled theme XML files from the structural
# skeleton of an upstream Scintilla-style stylers file. Reads the upstream
# file ONLY for the lexer/style technical-identifier list (not colours) and
# emits NPP-compatible XML files filled with each theme's canonical palette.
#
# Usage:
#   python3 tools/gen_themes/gen_themes.py
#
# Reads:  the upstream stylers.model.xml — path from the $NPP_STYLERS env
#         var, else ../notepad-plus-plus/PowerEditor/src/stylers.model.xml
#         relative to this repo's parent directory.
# Writes: assets/themes/NotepadStudioLight.xml
#         assets/themes/NotepadStudioDark.xml
#         assets/themes/Solarized.xml
#         assets/themes/Dracula.xml
#         assets/themes/Nord.xml
#         assets/themes/Gruvbox.xml
#         assets/themes/Monokai.xml

from __future__ import annotations
import os
import sys
import xml.etree.ElementTree as ET
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
NPP_STYLERS = Path(
    os.environ.get(
        "NPP_STYLERS",
        REPO.parent / "notepad-plus-plus/PowerEditor/src/stylers.model.xml",
    )
)
OUT_DIR = REPO / "assets/themes"

# ----------------------------------------------------------------------------
# Notepad Studio-original palettes (NOT derived from any other product).
# Designed for this fork on 2026-05-10.
# ----------------------------------------------------------------------------

LIGHT = {
    # base
    "bg":          "FAF9F6",  # soft warm off-white
    "fg":          "232629",  # dark slate (not pure black)
    "current":     "F2F0EA",  # current line bg, slightly warmer
    "selection":   "D8E5F2",  # selection bg, soft blue
    "caret":       "232629",
    "caret_multi": "646A72",
    "edge":        "C9CFD6",  # edge line / margins / indent guides
    "margin_bg":   "F1EEE7",  # gutter / line-number margin
    "margin_fg":   "9AA1A8",
    "ws_symbol":   "C9CFD6",  # whitespace symbol
    # syntax
    "comment":     "7A828B",  # muted slate-grey
    "doc_comment": "5C8B5C",  # muted forest green
    "keyword":     "0F4080",  # deep navy
    "keyword2":    "5A2D85",  # plum
    "type":        "21566B",  # slate teal
    "string":      "8C5500",  # warm umber
    "char":        "8C5500",
    "regex":       "9C3B5A",  # muted rose
    "number":      "8B0046",  # deep magenta
    "operator":    "484848",
    "preprocessor":"6E2D85",
    "label":       "9C5C18",
    "identifier":  "232629",
    "tag":         "0F4080",
    "attribute":   "6E2D85",
    "value":       "8C5500",
    "function":    "21566B",
    "variable":    "21566B",
    "macro":       "9C3B5A",
    "command":     "0F4080",
    "verbatim":    "8C5500",
    "url":         "0066B8",
    "constant":    "8B0046",
    "todo":        "C13030",
    "warning":     "DBA600",
    "deletion":    "C13030",
    "addition":    "468847",
    "header":      "0F4080",
    "error":       "C13030",
    # accents / change history / marks / tabs
    "fold":        "9AA1A8",
    "fold_active": "3C8CCC",
    "smart_hl":    "FBE7AD",
    "find_mark":   "FFCC74",
    "find_err":    "C13030",
    "find_msg":    "232629",
    "find_eor":    "DBA600",
    "mark1":       "C7E3FB",
    "mark2":       "D7F1C9",
    "mark3":       "FFEAC2",
    "mark4":       "F2D0E1",
    "mark5":       "E2D5F5",
    "incr_hl":     "C7E3FB",
    "tag_match":   "D7E3F4",
    "tag_attr":    "EAE0EE",
    "ch_modified": "DBA600",
    "ch_revert_m": "6FB346",
    "ch_revert_o": "3C8CCC",
    "ch_saved":    "6FB346",
    "ch_margin":   "EFEFEF",
    "tab_focus":   "3C8CCC",
    "tab_unfocus": "9AA1A8",
    "tab_text":    "232629",
    "tab_inact_fg":"6E6E6E",
    "tab_inact_bg":"EFEFEF",
    "tab1":        "D7E3F4",
    "tab2":        "D7F1C9",
    "tab3":        "FFEAC2",
    "tab4":        "F2D0E1",
    "tab5":        "E2D5F5",
    "tab1_dark":   "2D4870",
    "tab2_dark":   "2D5048",
    "tab3_dark":   "55411B",
    "tab4_dark":   "55253E",
    "tab5_dark":   "402C5C",
    "url_hover":   "0066B8",
    "doc_map":     "9AA1A8",
    "doc_map_bg":  "F1EEE7",
    "default_font":"Menlo",
    "default_size":"11",
}

DARK = {
    "bg":          "1C1E22",
    "fg":          "E4E4E2",
    "current":     "262A30",
    "selection":   "2E3F55",
    "caret":       "E4E4E2",
    "caret_multi": "9AA0A6",
    "edge":        "3C414A",
    "margin_bg":   "232629",
    "margin_fg":   "707683",
    "ws_symbol":   "3C414A",
    "comment":     "8E939B",
    "doc_comment": "8FB28F",
    "keyword":     "8AB7E8",
    "keyword2":    "C99CE6",
    "type":        "7DC8C8",
    "string":      "E5B27D",
    "char":        "E5B27D",
    "regex":       "E2799C",
    "number":      "E29299",
    "operator":    "B7BCC2",
    "preprocessor":"C99CE6",
    "label":       "E5B27D",
    "identifier":  "E4E4E2",
    "tag":         "8AB7E8",
    "attribute":   "C99CE6",
    "value":       "E5B27D",
    "function":    "7DC8C8",
    "variable":    "7DC8C8",
    "macro":       "E2799C",
    "command":     "8AB7E8",
    "verbatim":    "E5B27D",
    "url":         "82B6F2",
    "constant":    "E29299",
    "todo":        "E5807F",
    "warning":     "E2C77E",
    "deletion":    "E5807F",
    "addition":    "8FCC74",
    "header":      "8AB7E8",
    "error":       "E5807F",
    "fold":        "707683",
    "fold_active": "82B6F2",
    "smart_hl":    "584518",
    "find_mark":   "7A5F23",
    "find_err":    "E5807F",
    "find_msg":    "E4E4E2",
    "find_eor":    "E2C77E",
    "mark1":       "203E55",
    "mark2":       "1F4030",
    "mark3":       "4A3814",
    "mark4":       "4A1F33",
    "mark5":       "33235C",
    "incr_hl":     "203E55",
    "tag_match":   "2E3F55",
    "tag_attr":    "3A2C42",
    "ch_modified": "C68C00",
    "ch_revert_m": "5C8F36",
    "ch_revert_o": "2E78B0",
    "ch_saved":    "5C8F36",
    "ch_margin":   "232629",
    "tab_focus":   "82B6F2",
    "tab_unfocus": "707683",
    "tab_text":    "E4E4E2",
    "tab_inact_fg":"9AA0A6",
    "tab_inact_bg":"232629",
    "tab1":        "2D4870",
    "tab2":        "2D5048",
    "tab3":        "55411B",
    "tab4":        "55253E",
    "tab5":        "402C5C",
    "tab1_dark":   "2D4870",
    "tab2_dark":   "2D5048",
    "tab3_dark":   "55411B",
    "tab4_dark":   "55253E",
    "tab5_dark":   "402C5C",
    "url_hover":   "82B6F2",
    "doc_map":     "707683",
    "doc_map_bg":  "232629",
    "default_font":"Menlo",
    "default_size":"11",
}

# ----------------------------------------------------------------------------
# Solarized Dark (Ethan Schoonover, MIT) — https://github.com/altercation/solarized
# ----------------------------------------------------------------------------

SOLARIZED = {
    "bg":          "002B36",  # base03
    "fg":          "839496",  # base0
    "current":     "073642",  # base02
    "selection":   "073642",
    "caret":       "93A1A1",  # base1
    "caret_multi": "586E75",
    "edge":        "586E75",  # base01
    "margin_bg":   "073642",
    "margin_fg":   "586E75",
    "ws_symbol":   "586E75",
    "comment":     "586E75",  # base01
    "doc_comment": "586E75",
    "keyword":     "859900",  # green
    "keyword2":    "6C71C4",  # violet
    "type":        "B58900",  # yellow
    "string":      "2AA198",  # cyan
    "char":        "2AA198",
    "regex":       "DC322F",  # red
    "number":      "D33682",  # magenta
    "operator":    "839496",
    "preprocessor":"DC322F",  # red
    "label":       "CB4B16",  # orange
    "identifier":  "839496",
    "tag":         "268BD2",  # blue
    "attribute":   "B58900",
    "value":       "2AA198",
    "function":    "268BD2",
    "variable":    "268BD2",
    "macro":       "DC322F",
    "command":     "859900",
    "verbatim":    "2AA198",
    "url":         "268BD2",
    "constant":    "D33682",
    "todo":        "DC322F",
    "warning":     "B58900",
    "deletion":    "DC322F",
    "addition":    "859900",
    "header":      "268BD2",
    "error":       "DC322F",
    "fold":        "586E75",
    "fold_active": "268BD2",
    "smart_hl":    "55432F",
    "find_mark":   "806020",
    "find_err":    "DC322F",
    "find_msg":    "839496",
    "find_eor":    "B58900",
    "mark1":       "1A4F5A",
    "mark2":       "2A4F1A",
    "mark3":       "5A4F1A",
    "mark4":       "5A1A4F",
    "mark5":       "1A1A5A",
    "incr_hl":     "1A4F5A",
    "tag_match":   "073642",
    "tag_attr":    "1A2A35",
    "ch_modified": "B58900",
    "ch_revert_m": "859900",
    "ch_revert_o": "268BD2",
    "ch_saved":    "859900",
    "ch_margin":   "073642",
    "tab_focus":   "268BD2",
    "tab_unfocus": "586E75",
    "tab_text":    "839496",
    "tab_inact_fg":"586E75",
    "tab_inact_bg":"073642",
    "tab1":        "0A3F4A",
    "tab2":        "1A3F0A",
    "tab3":        "3F2F0A",
    "tab4":        "3F0A2F",
    "tab5":        "0A0A3F",
    "tab1_dark":   "0A3F4A",
    "tab2_dark":   "1A3F0A",
    "tab3_dark":   "3F2F0A",
    "tab4_dark":   "3F0A2F",
    "tab5_dark":   "0A0A3F",
    "url_hover":   "2AA198",
    "doc_map":     "586E75",
    "doc_map_bg":  "073642",
    "default_font":"Menlo",
    "default_size":"11",
}

# ----------------------------------------------------------------------------
# Dracula (Zeno Rocha, MIT) — https://draculatheme.com
# ----------------------------------------------------------------------------

DRACULA = {
    "bg":          "282A36",
    "fg":          "F8F8F2",
    "current":     "44475A",
    "selection":   "44475A",
    "caret":       "F8F8F0",
    "caret_multi": "BD93F9",
    "edge":        "44475A",
    "margin_bg":   "282A36",
    "margin_fg":   "6272A4",
    "ws_symbol":   "44475A",
    "comment":     "6272A4",
    "doc_comment": "50FA7B",
    "keyword":     "FF79C6",  # pink
    "keyword2":    "BD93F9",  # purple
    "type":        "8BE9FD",  # cyan
    "string":      "F1FA8C",  # yellow
    "char":        "F1FA8C",
    "regex":       "FF5555",  # red
    "number":      "BD93F9",  # purple
    "operator":    "FF79C6",
    "preprocessor":"FFB86C",  # orange
    "label":       "FFB86C",
    "identifier":  "F8F8F2",
    "tag":         "FF79C6",
    "attribute":   "50FA7B",  # green
    "value":       "F1FA8C",
    "function":    "50FA7B",
    "variable":    "F8F8F2",
    "macro":       "FF5555",
    "command":     "FF79C6",
    "verbatim":    "F1FA8C",
    "url":         "8BE9FD",
    "constant":    "BD93F9",
    "todo":        "FF5555",
    "warning":     "FFB86C",
    "deletion":    "FF5555",
    "addition":    "50FA7B",
    "header":      "BD93F9",
    "error":       "FF5555",
    "fold":        "6272A4",
    "fold_active": "8BE9FD",
    "smart_hl":    "5C4D2F",
    "find_mark":   "7A6024",
    "find_err":    "FF5555",
    "find_msg":    "F8F8F2",
    "find_eor":    "FFB86C",
    "mark1":       "2D3F5C",
    "mark2":       "2D5C3F",
    "mark3":       "5C4D2D",
    "mark4":       "5C2D4D",
    "mark5":       "3F2D5C",
    "incr_hl":     "2D3F5C",
    "tag_match":   "44475A",
    "tag_attr":    "3A3344",
    "ch_modified": "FFB86C",
    "ch_revert_m": "50FA7B",
    "ch_revert_o": "8BE9FD",
    "ch_saved":    "50FA7B",
    "ch_margin":   "282A36",
    "tab_focus":   "BD93F9",
    "tab_unfocus": "6272A4",
    "tab_text":    "F8F8F2",
    "tab_inact_fg":"6272A4",
    "tab_inact_bg":"282A36",
    "tab1":        "3D4D75",
    "tab2":        "3D5C44",
    "tab3":        "5C5024",
    "tab4":        "5C2D4D",
    "tab5":        "44345C",
    "tab1_dark":   "3D4D75",
    "tab2_dark":   "3D5C44",
    "tab3_dark":   "5C5024",
    "tab4_dark":   "5C2D4D",
    "tab5_dark":   "44345C",
    "url_hover":   "8BE9FD",
    "doc_map":     "6272A4",
    "doc_map_bg":  "282A36",
    "default_font":"Menlo",
    "default_size":"11",
}

# ----------------------------------------------------------------------------
# Nord (Arctic Ice Studio, MIT) — https://www.nordtheme.com
# ----------------------------------------------------------------------------

NORD = {
    "bg":          "2E3440",  # nord0
    "fg":          "D8DEE9",  # nord4
    "current":     "3B4252",  # nord1
    "selection":   "434C5E",  # nord2
    "caret":       "D8DEE9",
    "caret_multi": "88C0D0",
    "edge":        "4C566A",  # nord3
    "margin_bg":   "3B4252",
    "margin_fg":   "616E88",
    "ws_symbol":   "4C566A",
    "comment":     "616E88",
    "doc_comment": "A3BE8C",
    "keyword":     "81A1C1",  # nord9
    "keyword2":    "B48EAD",  # nord15
    "type":        "8FBCBB",  # nord7
    "string":      "A3BE8C",  # nord14
    "char":        "EBCB8B",  # nord13
    "regex":       "BF616A",  # nord11
    "number":      "B48EAD",
    "operator":    "ECEFF4",  # nord6
    "preprocessor":"5E81AC",  # nord10
    "label":       "D08770",  # nord12
    "identifier":  "D8DEE9",
    "tag":         "81A1C1",
    "attribute":   "8FBCBB",
    "value":       "A3BE8C",
    "function":    "88C0D0",  # nord8
    "variable":    "D8DEE9",
    "macro":       "B48EAD",
    "command":     "81A1C1",
    "verbatim":    "A3BE8C",
    "url":         "88C0D0",
    "constant":    "B48EAD",
    "todo":        "BF616A",
    "warning":     "EBCB8B",
    "deletion":    "BF616A",
    "addition":    "A3BE8C",
    "header":      "5E81AC",
    "error":       "BF616A",
    "fold":        "616E88",
    "fold_active": "88C0D0",
    "smart_hl":    "4C4426",
    "find_mark":   "5C5429",
    "find_err":    "BF616A",
    "find_msg":    "D8DEE9",
    "find_eor":    "EBCB8B",
    "mark1":       "2A3851",
    "mark2":       "324A3D",
    "mark3":       "504526",
    "mark4":       "4A3148",
    "mark5":       "353045",
    "incr_hl":     "2A3851",
    "tag_match":   "434C5E",
    "tag_attr":    "3F3543",
    "ch_modified": "EBCB8B",
    "ch_revert_m": "A3BE8C",
    "ch_revert_o": "5E81AC",
    "ch_saved":    "A3BE8C",
    "ch_margin":   "3B4252",
    "tab_focus":   "88C0D0",
    "tab_unfocus": "616E88",
    "tab_text":    "D8DEE9",
    "tab_inact_fg":"616E88",
    "tab_inact_bg":"3B4252",
    "tab1":        "3F4D66",
    "tab2":        "415C49",
    "tab3":        "5C5026",
    "tab4":        "5C3045",
    "tab5":        "443A5C",
    "tab1_dark":   "3F4D66",
    "tab2_dark":   "415C49",
    "tab3_dark":   "5C5026",
    "tab4_dark":   "5C3045",
    "tab5_dark":   "443A5C",
    "url_hover":   "88C0D0",
    "doc_map":     "616E88",
    "doc_map_bg":  "3B4252",
    "default_font":"Menlo",
    "default_size":"11",
}

# ----------------------------------------------------------------------------
# Gruvbox Dark (Pavel Pertsev, MIT) — https://github.com/morhetz/gruvbox
# ----------------------------------------------------------------------------

GRUVBOX = {
    "bg":          "282828",  # bg0
    "fg":          "EBDBB2",  # fg1
    "current":     "3C3836",  # bg1
    "selection":   "504945",  # bg2
    "caret":       "EBDBB2",
    "caret_multi": "BDAE93",
    "edge":        "504945",
    "margin_bg":   "3C3836",
    "margin_fg":   "928374",  # gray
    "ws_symbol":   "504945",
    "comment":     "928374",
    "doc_comment": "B8BB26",
    "keyword":     "FB4934",  # red
    "keyword2":    "D3869B",  # purple
    "type":        "FABD2F",  # yellow
    "string":      "B8BB26",  # green
    "char":        "B8BB26",
    "regex":       "FE8019",  # orange
    "number":      "D3869B",  # purple
    "operator":    "FE8019",
    "preprocessor":"83A598",  # blue
    "label":       "FE8019",
    "identifier":  "EBDBB2",
    "tag":         "8EC07C",  # aqua
    "attribute":   "FABD2F",
    "value":       "B8BB26",
    "function":    "8EC07C",
    "variable":    "EBDBB2",
    "macro":       "D3869B",
    "command":     "FB4934",
    "verbatim":    "B8BB26",
    "url":         "83A598",
    "constant":    "D3869B",
    "todo":        "FB4934",
    "warning":     "FABD2F",
    "deletion":    "FB4934",
    "addition":    "B8BB26",
    "header":      "FABD2F",
    "error":       "FB4934",
    "fold":        "928374",
    "fold_active": "83A598",
    "smart_hl":    "5C4A1F",
    "find_mark":   "7A5F23",
    "find_err":    "FB4934",
    "find_msg":    "EBDBB2",
    "find_eor":    "FABD2F",
    "mark1":       "2A3D4D",
    "mark2":       "354729",
    "mark3":       "4A3920",
    "mark4":       "4A2A38",
    "mark5":       "352A4A",
    "incr_hl":     "2A3D4D",
    "tag_match":   "504945",
    "tag_attr":    "3D3340",
    "ch_modified": "FABD2F",
    "ch_revert_m": "B8BB26",
    "ch_revert_o": "83A598",
    "ch_saved":    "B8BB26",
    "ch_margin":   "3C3836",
    "tab_focus":   "FABD2F",
    "tab_unfocus": "928374",
    "tab_text":    "EBDBB2",
    "tab_inact_fg":"928374",
    "tab_inact_bg":"3C3836",
    "tab1":        "3D4D5A",
    "tab2":        "3F4F2D",
    "tab3":        "5A4727",
    "tab4":        "5A2D38",
    "tab5":        "3F345C",
    "tab1_dark":   "3D4D5A",
    "tab2_dark":   "3F4F2D",
    "tab3_dark":   "5A4727",
    "tab4_dark":   "5A2D38",
    "tab5_dark":   "3F345C",
    "url_hover":   "83A598",
    "doc_map":     "928374",
    "doc_map_bg":  "3C3836",
    "default_font":"Menlo",
    "default_size":"11",
}

# ----------------------------------------------------------------------------
# Monokai (Wimer Hazenberg, original 2006 palette) — https://monokai.pro
# ----------------------------------------------------------------------------

MONOKAI = {
    "bg":          "272822",
    "fg":          "F8F8F2",
    "current":     "3E3D32",
    "selection":   "49483E",
    "caret":       "F8F8F0",
    "caret_multi": "AE81FF",
    "edge":        "49483E",
    "margin_bg":   "272822",
    "margin_fg":   "75715E",
    "ws_symbol":   "49483E",
    "comment":     "75715E",
    "doc_comment": "A6E22E",
    "keyword":     "F92672",  # pink
    "keyword2":    "AE81FF",  # purple
    "type":        "66D9EF",  # cyan
    "string":      "E6DB74",  # yellow
    "char":        "E6DB74",
    "regex":       "FD971F",  # orange
    "number":      "AE81FF",
    "operator":    "F92672",
    "preprocessor":"A6E22E",  # green
    "label":       "FD971F",
    "identifier":  "F8F8F2",
    "tag":         "F92672",
    "attribute":   "A6E22E",
    "value":       "E6DB74",
    "function":    "A6E22E",
    "variable":    "FD971F",
    "macro":       "F92672",
    "command":     "F92672",
    "verbatim":    "E6DB74",
    "url":         "66D9EF",
    "constant":    "AE81FF",
    "todo":        "F92672",
    "warning":     "FD971F",
    "deletion":    "F92672",
    "addition":    "A6E22E",
    "header":      "AE81FF",
    "error":       "F92672",
    "fold":        "75715E",
    "fold_active": "66D9EF",
    "smart_hl":    "5C4F2A",
    "find_mark":   "7A6224",
    "find_err":    "F92672",
    "find_msg":    "F8F8F2",
    "find_eor":    "FD971F",
    "mark1":       "2A3D55",
    "mark2":       "354A29",
    "mark3":       "4A3F20",
    "mark4":       "4A2A38",
    "mark5":       "3F2A5A",
    "incr_hl":     "2A3D55",
    "tag_match":   "49483E",
    "tag_attr":    "3D3344",
    "ch_modified": "FD971F",
    "ch_revert_m": "A6E22E",
    "ch_revert_o": "66D9EF",
    "ch_saved":    "A6E22E",
    "ch_margin":   "272822",
    "tab_focus":   "66D9EF",
    "tab_unfocus": "75715E",
    "tab_text":    "F8F8F2",
    "tab_inact_fg":"75715E",
    "tab_inact_bg":"272822",
    "tab1":        "2D4D6B",
    "tab2":        "3F5A2D",
    "tab3":        "5A4929",
    "tab4":        "5A2D44",
    "tab5":        "44295C",
    "tab1_dark":   "2D4D6B",
    "tab2_dark":   "3F5A2D",
    "tab3_dark":   "5A4929",
    "tab4_dark":   "5A2D44",
    "tab5_dark":   "44295C",
    "url_hover":   "66D9EF",
    "doc_map":     "75715E",
    "doc_map_bg":  "272822",
    "default_font":"Menlo",
    "default_size":"11",
}

# ----------------------------------------------------------------------------
# Style-name → role classifier. Maps NPP style identifiers (technical
# strings shared across Scintilla-based editors) to a Notepad Studio palette role.
# ----------------------------------------------------------------------------

def classify(style_name: str) -> str:
    n = style_name.upper()
    # Common patterns first.
    if "DEFAULT" in n or n == "IDENTIFIER":
        return "identifier"
    if "DOC COMMENT" in n or "DOCCOMMENT" in n or "DOC_COMMENT" in n:
        return "doc_comment"
    if "JAVADOC" in n or "DOXY" in n:
        return "doc_comment"
    if "COMMENT" in n:
        return "comment"
    if "PREPROCESSOR" in n or "PREPROC" in n or "DIRECTIVE" in n:
        return "preprocessor"
    if "REGEX" in n or "REGULAR EXPRESSION" in n or "PATTERN" in n:
        return "regex"
    if "NUMBER" in n or "NUMERIC" in n or "DIGIT" in n:
        return "number"
    if "OPERATOR" in n or "PUNCTUATION" in n or "SYMBOL" in n:
        return "operator"
    if "STRING" in n or "TEXT BETWEEN" in n or "DOUBLEQUOTED" in n or "SINGLEQUOTED" in n:
        return "string"
    if "CHARACTER" in n or n == "CHAR":
        return "char"
    if "ATTRIBUTE" in n or "ATTRIB" in n:
        return "attribute"
    if "TAG" in n and "MATCH" not in n:
        return "tag"
    if "VALUE" in n:
        return "value"
    if "VARIABLE" in n or "VAR " in n or n.endswith("VAR"):
        return "variable"
    if "FUNCTION" in n or "FUNC" in n or "METHOD" in n:
        return "function"
    if "MACRO" in n:
        return "macro"
    if "TYPE" in n or "CLASS" in n:
        return "type"
    if "INSTRUCTION WORD" in n or "KEY WORD" in n or "KEYWORD" in n or n.startswith("WORD"):
        if any(s in n for s in ("2", "3", "4", "5", "6", "7", "8", "SECONDARY")):
            return "keyword2"
        return "keyword"
    if "INSTRUCTION" in n:
        return "keyword"
    if "LABEL" in n:
        return "label"
    if "COMMAND" in n:
        return "command"
    if "URL" in n or "LINK" in n or "HREF" in n:
        return "url"
    if "VERBATIM" in n or "BACKTICK" in n:
        return "verbatim"
    if "CONSTANT" in n:
        return "constant"
    if "ERROR" in n:
        return "error"
    if "TODO" in n:
        return "todo"
    if "WARNING" in n:
        return "warning"
    if "DELETION" in n or "REMOVED" in n:
        return "deletion"
    if "ADDITION" in n or "ADDED" in n or "INSERTED" in n:
        return "addition"
    if "HEADER" in n or "TITLE" in n or "SECTION" in n:
        return "header"
    # Fallback: identifier (uses fg).
    return "identifier"


# ----------------------------------------------------------------------------
# Per-LexerStyles role overrides. NPP's style names aren't always self-
# describing. Keyed by (lexer_name, style_name) tuple. Empty by default;
# fill if a specific lexer needs a tweak.
# ----------------------------------------------------------------------------

OVERRIDES: dict[tuple[str, str], str] = {
    # Markdown headers
    ("markdown", "HEADER 1"): "header",
    ("markdown", "HEADER 2"): "header",
    ("markdown", "HEADER 3"): "header",
    ("markdown", "HEADER 4"): "header",
    ("markdown", "HEADER 5"): "header",
    ("markdown", "HEADER 6"): "header",
    ("markdown", "STRONG 1"): "keyword",
    ("markdown", "STRONG 2"): "keyword",
    ("markdown", "EM 1"): "keyword2",
    ("markdown", "EM 2"): "keyword2",
    ("markdown", "BLOCKQUOTE"): "comment",
    ("markdown", "CODE"): "verbatim",
    ("markdown", "CODE2"): "verbatim",
    ("markdown", "CODEBK"): "verbatim",
    ("markdown", "LINE BEGIN"): "identifier",
    ("markdown", "PRECHAR"): "operator",
    ("markdown", "HRULE"): "operator",
    ("markdown", "LINK"): "url",
    ("markdown", "ULIST_ITEM"): "operator",
    ("markdown", "OLIST_ITEM"): "operator",
    # Diff
    ("diff", "ADDED"): "addition",
    ("diff", "DELETED"): "deletion",
    ("diff", "CHANGED"): "warning",
    ("diff", "POSITION"): "header",
    ("diff", "HEADER"): "header",
    ("diff", "COMMAND"): "command",
}


# ----------------------------------------------------------------------------
# Reader
# ----------------------------------------------------------------------------

def parse_skeleton(path: Path) -> tuple[list[dict], list[dict]]:
    """Returns (lexer_types, global_widget_styles_skeleton).
    Each lexer entry: {name, desc, ext, excluded, styles: [{name, styleID, keywordClass?}]}
    Each global entry: {name, styleID}.
    Colours are NEVER read from the upstream file.
    """
    if not path.exists():
        print(f"ERROR: NPP stylers.model.xml not found at {path}", file=sys.stderr)
        sys.exit(2)
    tree = ET.parse(path)
    root = tree.getroot()

    lexers = []
    for lex in root.findall(".//LexerStyles/LexerType"):
        entry = {
            "name": lex.attrib.get("name", ""),
            "desc": lex.attrib.get("desc", lex.attrib.get("name", "")),
            "ext":  lex.attrib.get("ext", ""),
            "excluded": lex.attrib.get("excluded", "no"),
            "styles": [],
        }
        for ws in lex.findall("WordsStyle"):
            s = {
                "name": ws.attrib.get("name", ""),
                "styleID": ws.attrib.get("styleID", "0"),
            }
            if "keywordClass" in ws.attrib and ws.attrib["keywordClass"]:
                s["keywordClass"] = ws.attrib["keywordClass"]
            entry["styles"].append(s)
        lexers.append(entry)

    # Global skeleton: name + styleID only.
    globals_ = []
    for ws in root.findall(".//GlobalStyles/WidgetStyle"):
        globals_.append({
            "name": ws.attrib.get("name", ""),
            "styleID": ws.attrib.get("styleID", "0"),
        })
    return lexers, globals_


# ----------------------------------------------------------------------------
# Palette resolver: returns (fg, bg, font_style) for a given role.
# ----------------------------------------------------------------------------

def role_to_attrs(role: str, palette: dict) -> tuple[str, str, str]:
    bg = palette["bg"]
    fontStyle = "0"
    fg_map = {
        "comment": (palette["comment"], "2"),       # italic
        "doc_comment": (palette["doc_comment"], "2"),
        "keyword": (palette["keyword"], "1"),       # bold
        "keyword2": (palette["keyword2"], "1"),
        "type":     (palette["type"], "1"),
        "string":   (palette["string"], "0"),
        "char":     (palette["char"], "0"),
        "regex":    (palette["regex"], "0"),
        "number":   (palette["number"], "0"),
        "operator": (palette["operator"], "0"),
        "preprocessor": (palette["preprocessor"], "0"),
        "label":    (palette["label"], "0"),
        "identifier": (palette["fg"], "0"),
        "tag":      (palette["tag"], "1"),
        "attribute":(palette["attribute"], "0"),
        "value":    (palette["value"], "0"),
        "function": (palette["function"], "0"),
        "variable": (palette["variable"], "0"),
        "macro":    (palette["macro"], "0"),
        "command":  (palette["command"], "1"),
        "verbatim": (palette["verbatim"], "0"),
        "url":      (palette["url"], "4"),          # underline
        "constant": (palette["constant"], "0"),
        "todo":     (palette["todo"], "1"),
        "warning":  (palette["warning"], "1"),
        "deletion": (palette["deletion"], "0"),
        "addition": (palette["addition"], "0"),
        "header":   (palette["header"], "1"),
        "error":    (palette["error"], "1"),
    }
    fg, fontStyle = fg_map.get(role, (palette["fg"], "0"))
    return fg, bg, fontStyle


# ----------------------------------------------------------------------------
# GlobalStyles renderer — fills colours per known global name.
# Anything not explicitly mapped uses Default Style fg/bg.
# ----------------------------------------------------------------------------

def render_global(name: str, style_id: str, palette: dict) -> str:
    p = palette
    none = ("", "", "", "", "")
    table = {
        "Global override":           (p["fg"], p["bg"], p["default_font"], "0", p["default_size"]),
        "Default Style":             (p["fg"], p["bg"], p["default_font"], "0", p["default_size"]),
        "Indent guideline style":    (p["edge"], p["bg"], "", "0", ""),
        "Brace highlight style":     (p["fg"], p["selection"], "", "1", ""),
        "Bad brace colour":          ("FFFFFF", p["error"], "", "1", ""),
        "Current line background colour": ("", p["current"], "", "", ""),
        "Selected text colour":      ("", p["selection"], "", "", ""),
        "Multi-selected text color": ("", p["selection"], "", "", ""),
        "Caret colour":              (p["caret"], "", "", "", ""),
        "Multi-edit carets color":   (p["caret_multi"], "", "", "", ""),
        "Edge colour":               (p["edge"], "", "", "", ""),
        "Line number margin":        (p["margin_fg"], p["margin_bg"], "", "0", ""),
        "Bookmark margin":           ("", p["ch_margin"], "", "", ""),
        "Change History margin":     ("", p["ch_margin"], "", "", ""),
        "Change History modified":   (p["ch_modified"], p["ch_margin"], "", "", ""),
        "Change History revert modified": (p["ch_revert_m"], p["ch_margin"], "", "", ""),
        "Change History revert origin":   (p["ch_revert_o"], p["ch_margin"], "", "", ""),
        "Change History saved":      (p["ch_saved"], p["ch_margin"], "", "", ""),
        "Fold":                      (p["fold"], p["margin_bg"], "", "", ""),
        "Fold active":               (p["fold_active"], p["margin_bg"], "", "", ""),
        "Fold margin":               (p["edge"], p["margin_bg"], "", "", ""),
        "White space symbol":        (p["ws_symbol"], p["bg"], "", "", ""),
        "Smart Highlighting":        ("", p["smart_hl"], "", "", ""),
        "Find Mark Style":           ("", p["find_mark"], "", "", ""),
        "Find status: Not found":    (p["find_err"], p["bg"], "", "", ""),
        "Find status: Message":      (p["find_msg"], p["bg"], "", "", ""),
        "Find status: Search end reached": (p["find_eor"], p["bg"], "", "", ""),
        "Mark Style 1":              ("", p["mark1"], "", "", ""),
        "Mark Style 2":              ("", p["mark2"], "", "", ""),
        "Mark Style 3":              ("", p["mark3"], "", "", ""),
        "Mark Style 4":              ("", p["mark4"], "", "", ""),
        "Mark Style 5":              ("", p["mark5"], "", "", ""),
        "Incremental highlight all": ("", p["incr_hl"], "", "", ""),
        "Tags match highlighting":   ("", p["tag_match"], "", "", ""),
        "Tags attribute":            ("", p["tag_attr"], "", "", ""),
        "Active tab focused indicator":   (p["tab_focus"], "", "", "", ""),
        "Active tab unfocused indicator": (p["tab_unfocus"], "", "", "", ""),
        "Active tab text":           (p["tab_text"], "", "", "", ""),
        "Inactive tabs":             (p["tab_inact_fg"], p["tab_inact_bg"], "", "", ""),
        "Tab color 1":               ("", p["tab1"], "", "", ""),
        "Tab color 2":               ("", p["tab2"], "", "", ""),
        "Tab color 3":               ("", p["tab3"], "", "", ""),
        "Tab color 4":               ("", p["tab4"], "", "", ""),
        "Tab color 5":               ("", p["tab5"], "", "", ""),
        "Tab color dark mode 1":     ("", p["tab1_dark"], "", "", ""),
        "Tab color dark mode 2":     ("", p["tab2_dark"], "", "", ""),
        "Tab color dark mode 3":     ("", p["tab3_dark"], "", "", ""),
        "Tab color dark mode 4":     ("", p["tab4_dark"], "", "", ""),
        "Tab color dark mode 5":     ("", p["tab5_dark"], "", "", ""),
        "URL hovered":               (p["url_hover"], "", "", "", ""),
        "Document map":              (p["doc_map"], p["doc_map_bg"], "", "", ""),
        "EOL custom color":          (p["edge"], "", "", "", ""),
        "Non-printing characters custom color": (p["edge"], "", "", "", ""),
        "Caret line frame":          (p["edge"], "", "", "", ""),
        "Tab marker":                (p["edge"], "", "", "", ""),
    }
    fg, bg, fname, fstyle, fsize = table.get(name, none)
    parts = [f'name="{xml_escape(name)}"', f'styleID="{style_id}"']
    if fg:
        parts.append(f'fgColor="{fg}"')
    if bg:
        parts.append(f'bgColor="{bg}"')
    if fname or fstyle or fsize:
        parts.append(f'fontName="{fname}"')
        parts.append(f'fontStyle="{fstyle}"')
        parts.append(f'fontSize="{fsize}"')
    return "        <WidgetStyle " + " ".join(parts) + " />"


def xml_escape(s: str) -> str:
    return (s.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")
             .replace('"', "&quot;").replace("'", "&apos;"))


def render_word_style(lexer_name: str, style: dict, palette: dict) -> str:
    role = OVERRIDES.get((lexer_name, style["name"].upper()),
            OVERRIDES.get((lexer_name, style["name"]), classify(style["name"])))
    fg, bg, font_style = role_to_attrs(role, palette)
    parts = [
        f'name="{xml_escape(style["name"])}"',
        f'styleID="{style["styleID"]}"',
        f'fgColor="{fg}"',
        f'bgColor="{bg}"',
        'fontName=""',
        f'fontStyle="{font_style}"',
        'fontSize=""',
    ]
    if "keywordClass" in style:
        parts.append(f'keywordClass="{style["keywordClass"]}"')
    return "            <WordsStyle " + " ".join(parts) + " />"


def render_lexer(lex: dict, palette: dict) -> str:
    attrs = (f'name="{xml_escape(lex["name"])}" desc="{xml_escape(lex["desc"])}" '
             f'ext="{xml_escape(lex["ext"])}"')
    if lex.get("excluded", "no") not in ("no", ""):
        attrs += f' excluded="{xml_escape(lex["excluded"])}"'
    if not lex["styles"]:
        return f'        <LexerType {attrs} />'
    body = "\n".join(render_word_style(lex["name"], s, palette) for s in lex["styles"])
    return f'        <LexerType {attrs}>\n{body}\n        </LexerType>'


def render_xml(palette: dict, header: str, lexers: list[dict], globals_: list[dict]) -> str:
    lex_blocks = "\n".join(render_lexer(l, palette) for l in lexers)
    glob_blocks = "\n".join(render_global(g["name"], g["styleID"], palette) for g in globals_)
    return f"""<?xml version="1.0" encoding="UTF-8" ?>
{header}
<NotepadPlus>
    <LexerStyles>
{lex_blocks}
    </LexerStyles>
    <GlobalStyles>
{glob_blocks}
    </GlobalStyles>
</NotepadPlus>
"""


# ----------------------------------------------------------------------------
# Theme registry — drives the emit loop in main().
# ----------------------------------------------------------------------------

HEADER_LIGHT = """<!--
  Notepad Studio stylers - NotepadStudioLight
  License: Apache-2.0. Original colour palette authored for Notepad Studio.
  Schema structure (lexer/style names) reflects shared technical identifiers
  used by Scintilla-based editors and is not derived from any GPL'd content.

  Palette: warm off-white background (#FAF9F6), dark slate text (#232629),
  navy keywords (#0F4080), umber strings (#8C5500), magenta numbers (#8B0046),
  forest doc-comments (#5C8B5C), muted slate comments (#7A828B).
-->"""

HEADER_DARK = """<!--
  Notepad Studio stylers - NotepadStudioDark
  License: Apache-2.0. Original colour palette authored for Notepad Studio.
  Schema structure (lexer/style names) reflects shared technical identifiers
  used by Scintilla-based editors and is not derived from any GPL'd content.

  Palette: tinted near-black background (#1C1E22), warm off-white text
  (#E4E4E2), pastel-blue keywords (#8AB7E8), warm peach strings (#E5B27D),
  rose numbers (#E29299), teal types (#7DC8C8), pastel doc-comments (#8FB28F).
-->"""

HEADER_SOLARIZED = """<!--
  Theme: Solarized Dark
  Original author: Ethan Schoonover (2011)
  Source: https://github.com/altercation/solarized
  Licence: MIT — palette colour values reused under MIT terms.
  Notepad Studio adaptation: Apache-2.0. Schema (lexer/style names) is a
  compatibility surface for Scintilla-style editors and is not
  derived from any GPL'd source.
-->"""

HEADER_DRACULA = """<!--
  Theme: Dracula
  Original author: Zeno Rocha (2013)
  Source: https://draculatheme.com — https://github.com/dracula/dracula-theme
  Licence: MIT — palette colour values reused under MIT terms.
  Notepad Studio adaptation: Apache-2.0. Schema (lexer/style names) is a
  compatibility surface for Scintilla-style editors and is not
  derived from any GPL'd source.
-->"""

HEADER_NORD = """<!--
  Theme: Nord
  Original author: Arctic Ice Studio / Sven Greb (2016)
  Source: https://www.nordtheme.com — https://github.com/nordtheme/nord
  Licence: MIT — palette colour values reused under MIT terms.
  Notepad Studio adaptation: Apache-2.0. Schema (lexer/style names) is a
  compatibility surface for Scintilla-style editors and is not
  derived from any GPL'd source.
-->"""

HEADER_GRUVBOX = """<!--
  Theme: Gruvbox Dark
  Original author: Pavel Pertsev / morhetz (2016)
  Source: https://github.com/morhetz/gruvbox
  Licence: MIT — palette colour values reused under MIT terms.
  Notepad Studio adaptation: Apache-2.0. Schema (lexer/style names) is a
  compatibility surface for Scintilla-style editors and is not
  derived from any GPL'd source.
-->"""

HEADER_MONOKAI = """<!--
  Theme: Monokai
  Inspired by Wimer Hazenberg's original Monokai (2006, TextMate).
  Source: https://monokai.pro — original palette is widely
  reimplemented across editors; colour values used here are the
  canonical 2006 set, not derived from any specific port.
  Notepad Studio adaptation: Apache-2.0. Schema (lexer/style names) is a
  compatibility surface for Scintilla-style editors and is not
  derived from any GPL'd source.
-->"""

THEMES: list[tuple[str, dict, str]] = [
    ("NotepadStudioLight", LIGHT, HEADER_LIGHT),
    ("NotepadStudioDark", DARK, HEADER_DARK),
    ("Solarized", SOLARIZED, HEADER_SOLARIZED),
    ("Dracula", DRACULA, HEADER_DRACULA),
    ("Nord", NORD, HEADER_NORD),
    ("Gruvbox", GRUVBOX, HEADER_GRUVBOX),
    ("Monokai", MONOKAI, HEADER_MONOKAI),
]


def main() -> int:
    lexers, globals_ = parse_skeleton(NPP_STYLERS)
    print(f"Parsed {len(lexers)} lexers + {len(globals_)} global styles "
          f"from {NPP_STYLERS}", file=sys.stderr)

    OUT_DIR.mkdir(parents=True, exist_ok=True)
    for name, palette, header in THEMES:
        out = OUT_DIR / f"{name}.xml"
        out.write_text(render_xml(palette, header, lexers, globals_))
        print(f"Wrote {out} ({out.stat().st_size} bytes)", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
