# reccmp-gui

maybe objdiff-style gui wrapper for reccmp.

This was developed for and tested with the [Touhou 7 decompilation project](https://github.com/some100/th07). There may be edge cases unaccounted for in programs outside of that project. If there happen to be any, please [open an issue](https://github.com/some100/reccmp-gui/issues/new).

This is only for use with 32-bit x86 applications with old versions of MSVC. If you aren't using that, then you probably aren't using reccmp in the first place.

## Dependencies

- [reccmp](https://github.com/isledecomp/reccmp)

## Usage

1. Go into `File -> New`. Select the project directory in the folder picker (that is the directory containing `reccmp-project.yml` and `reccmp-user.yml`), fill out the build command, and click `Finish`. Or load the `reccmp-gui.yml` in `File -> Open -> Browse...`.
2. If they're not already auto-detected, browse for the locations of `reccmp-reccmp` and co. Then, select your target. It'll automatically start building after doing that.
3. Select a function in the listing to view the assembly diffs. Use `N` and `Shift+N` to jump to next and previous diff.

# Licensing

This project is dual-licensed under your choice of the [GPLv3](./LICENSE-GPL) or [LICENSE](./LICENSE).
