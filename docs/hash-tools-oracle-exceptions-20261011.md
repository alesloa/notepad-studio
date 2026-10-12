# Hash dialog caption audit

The SHA-512 file dialog uses **Generate SHA-512 digest from files**. Its selected
algorithm and output remain SHA-512. This corrects a misleading upstream caption;
it is an explicit exception to exact default-English caption parity.

The inspected Notepad++ source is the local `notepad-plus-plus-master` checkout:

- `PowerEditor/src/MISC/md5/md5Dlgs.rc:24–61` supplies the shared MD5 resource.
- `md5Dlgs.cpp:282–327` creates that resource, then sets algorithm-specific titles
  and chooser text. The SHA-512 branch at 316–319 sets a SHA-1 title and a SHA-512
  chooser label. `WM_INITDIALOG` does not supply a later algorithm title.
- `PowerEditor/src/NppCommands.cpp:3667–3670` opens the SHA-512 file dialog and
  then calls `changeDlgLang` with `SHA512FromFilesDlg` on its first creation.
- `PowerEditor/src/localization.cpp:1228–1248` returns when no language is loaded,
  otherwise replacing the caption from the matching dialog's `title` attribute.
- `localization.h` defaults `loadIfEnglish` to false. Startup calls `init` in
  `Notepad_plus.cpp:145`; `localization.cpp:170–175` disables ordinary
  `english.xml` in that case. The erroneous runtime title therefore survives in
  default English. A loaded custom or translated language can replace it.
- `PowerEditor/installer/nativeLang/english.xml:605–609` names the SHA-512 file
  dialog correctly. Notepad Studio uses this title, as approved in review.

The other seven captions follow the effective runtime strings in
`md5Dlgs.cpp:295–312,650–689`. A registered SHA-512 file-dialog control checks the
actual window caption, selected algorithm and the independently known digest of
`abc`; caption agreement does not stand in for a digest test.

Dialog Copy controls are tested with fresh clipboard sentinels. The initial
output keyboard-Copy fixture had already placed the expected digest on the
clipboard and could miss a routing failure. The stronger fixture reproduced that
failure before field focus registration was fixed. Both local evidence packets
are retained; this was a candidate implementation defect, not a baseline or
native Notepad++ defect.

A retained registered GPUI input callback also reproduced a candidate defect
when a real hash result published before the next paint: updating the output
retired an unchanged live input composition. Fields now have separate input
generations, while Copy retains the dialog-wide version guard. Controls preserve
live composition across output publication and reject input edit/Undo ABA,
selection/focus/window ABA and output queries after replacement. This tests the
installed input protocol; native OS IME interaction remains an audit gap.
