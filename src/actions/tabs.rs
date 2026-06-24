// Tab cycling and direct tab-N jump actions. No dedicated Tabs menu today;
// these all wire from the keymap, while the per-tab UI is rendered in the
// tab strip.

gpui::actions!(
    notepad_studio,
    [
        TabNext,
        TabPrev,
        TabNextSequential,
        TabPrevSequential,
        TabGoto1,
        TabGoto2,
        TabGoto3,
        TabGoto4,
        TabGoto5,
        TabGoto6,
        TabGoto7,
        TabGoto8,
        TabGoto9,
    ]
);
