# reccmp-gui

maybe objdiff-style gui wrapper for reccmp.

This was primarily developed for and tested with the [Touhou 7 decompilation project](https://github.com/some100/th07) as well as the [Touhou 8 decompilation project](https://github.com/GensokyoClub/th08). There may be edge cases unaccounted for in programs outside of those projects. If there happen to be any, please [open an issue](https://github.com/some100/reccmp-gui/issues/new).

This is only for use with 32-bit x86 applications with old versions of MSVC. If you aren't using that, then you probably aren't using reccmp in the first place.

## Dependencies

- [reccmp](https://github.com/isledecomp/reccmp)

## Building

Just use `cargo build -r` ❓

## Usage

1. Go into `File -> New`. Select the project directory in the folder picker (that is the directory containing `reccmp-project.yml` and `reccmp-user.yml`), fill out the build command, and click `Finish`. Or load the `reccmp-gui.yml` in `File -> Open -> Browse...`.
2. If they're not already auto-detected, browse for the locations of `reccmp-reccmp` and co. Then, select your target. It'll automatically start building after doing that.
3. Select a function in the listing to view the assembly diffs.

## Usage (continued)

This tool integrates 3 of the reccmp tools.

- `reccmp-reccmp`, which is the listing that appears after a successful build.
- `reccmp-stackcmp`, which is available on the top left of a function diff tab, and
- `reccmp-roadmap`, which can be called through the Tool menu on the menu bar.

Each of these tools are opened and used in the form of tabs. These tabs can be navigated between using Ctrl+Tab or Ctrl+Shift+Tab, and can be closed with Ctrl+W.

`reccmp-datacmp` also exists but might as well not be there. Anyways,

`reccmp-reccmp` is automatically called after a build. It shows up as the listing tab once it completes. This listing tab is basically the equivalent to reccmp's webui listing.

You can search by offset or function name, hide fully matched functions, hide stubs, and show recomp addresses. You can click on the column headers to sort them ascending or descending as well.

The function names are clickable, and clicking them will disassemble the function and lead to a function diff tab that updates after every rebuild.

To the top left is the `Run reccmp-stackcmp` button. Clicking this will trigger stackcmp for the current function and display the result in a new stackcmp tab. To the top right is the matching percentage of the function as well as the current hunk (for navigating with N/Shift+N).

If the function contains jump tables or data tables, you can find a button on the top left that says either "Tables: MATCH (numtables)" or "Tables: DIFF (numtables)" depending on if your tables match. Clicking on this button will open a window containing the tables.

For each table, there will be:

- An Index column to show the indices of the jump table,
- A Status column (MATCH or DIFF),
- An Orig Target column to show where that index of the jump table resolves to in the original binary,
- A Recomp Target column, and
- An Offset column, which shows the offset of the jump tables from the start of the jump table (and if they differ, it shows both of the offsets as orig_offset vs. recomp_offset)

You can jump to the orig targets and the recomp targets as well, which will scroll to and highlight the row in the disassembly.

The main part of the tab is the actual disassembly. Should be self explanatory for the most part. However, if a row is highlighted in green, that is an "Advisory" diff. This means that it doesn't show up in normal reccmp, and can usually be ignored. There will be `~>` arrows to the right of jump instructions (which may or may not have been inspired/ripped off from objdiff) and to the left of jump targets. Clicking on the jump arrows will scroll to the target instruction, and clicking on the target arrows will scroll to the jump instruction.

Pressing N in the disassembly diff will scroll to the next hunk (which is indicated by the top right), and Shift+N will scroll to the previous hunk.

# Licensing

This project is dual-licensed under your choice of the [GPLv3](./LICENSE-GPL) or [LICENSE](./LICENSE).
