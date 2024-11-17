
# ORCA

A simple OPDS Server for Calibre written in Rust

## Configuration

If you set the environment variable `ORCA_CONFIG` to the path of a TOML file, that's where the configuration will be read from. Otherwise, it will look for a file named `orca.toml` in `$HOME/.config` or `/$HOME/.config/orca/config.toml` (in that order).
```toml
[server]
ip = "<your_ip>"
port = 8080
templates = "/path/to/templates"

[authentication]
credentials = [ "bob:password", "alice:secret" ]

[calibre.libraries]
library = "/Volumes/library"
nonfiction = "/Volumes/nonfiction"
```
