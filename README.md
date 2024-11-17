
# ORCA

A simple OPDS Server for Calibre written in Rust

## Configuration

If you point the environment variable `ORCA_CONFIG` to a `.toml` file, that's where the configuration will be read from. Otherwise, it will look for a file named `orca.toml` or `orca/config.toml` in `$HOME/.config/`.
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
