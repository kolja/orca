
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
alice = "468a286ae97d67f84b56:94Gxd6BCmgkBAtMEIxjW"
bob = "9a8692aeabe66ebfa609:iK4ODmrJ6RsD8CYRjcY6"

[calibre.libraries]
library = "/Volumes/library"
nonfiction = "/Volumes/nonfiction"
```

## Authentication

The server supports basic authentication: You can generate a password hash like so:
```bash
orca --hash <login>:<password> # e.g. orca --hash alice:secretpassword
```
The server will print the hash which you have to copy to the `[authentication]` section of your config file.
