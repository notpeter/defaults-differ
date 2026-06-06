# defaults-differ

Detect changes in macOS `defaults`

## Usage

Is there a macOS system setting that you'd like to be able to
automatically configure via the `defaults` command? We got you.

```shell
% defaults-differ
Dumping current defaults...
Make your settings change now, then press Enter to dump defaults again.

Dumping updated defaults...

defaults write NSGlobalDomain com.apple.swipescrolldirection -bool true
```

Progress and warnings are written to stderr. Generated `defaults` commands are
written to stdout, so you can append them directly:

```shell
defaults-differ >> ~/.defaults
defaults-differ | tee -a ~/.defaults
```

You can also ask `defaults-differ` to append for you:

```shell
defaults-differ -a ~/.defaults
defaults-differ --out ~/.defaults --edit
defaults-differ --edit vim
defaults-differ --message "Trackpad" >> ~/.defaults
```

`--edit` opens `$EDITOR`. With `--append` or `--out`, it edits the file after
appending. Without an append target, it opens the generated commands on editor
stdin, equivalent to `defaults-differ | $EDITOR -`. `-e` is a short alias, and
both forms accept an optional editor name. If `$EDITOR` is unset and no editor is
specified, `--edit` prints a warning and leaves the generated output intact.
