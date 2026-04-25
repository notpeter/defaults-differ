# defaults-differ

Detect changes in macOS `defaults`

## Usage

Is there a macOS system setting that you'd like to be able to
automatically configure via the `defaults` command? We got you.

```shell
% cargo -q run
Dumping current defaults...
Make your settings change now, then press Enter to dump defaults again.

Dumping updated defaults...

Generated defaults commands:
defaults write NSGlobalDomain com.apple.swipescrolldirection -bool true
```

What you do with that information is up to you.
