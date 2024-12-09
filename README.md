
<img src="./orca-logo.svg" alt="an orca whale reading a book" width="200px" height="200px">

# ORCA

![latest](https://img.shields.io/github/v/tag/kolja/orca)
![build](https://github.com/kolja/orca/actions/workflows/rust.yml/badge.svg)
[![dependency status](https://deps.rs/repo/github/kolja/orca/status.svg?path=%2F)](https://deps.rs/repo/github/kolja/orca?path=%2F)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

A simple OPDS Server for Calibre written in Rust

## Configuration

If you point the environment variable `ORCA_CONFIG` to a `.toml` file, that's where the configuration will be read from. Otherwise, it will look for a file named `orca.toml` or `orca/config.toml` in `$HOME/.config/`.
```toml
[server]
ip = "<your_ip>"
port = 8080

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

## Development

There are a couple of tasks you can run with `cargo make`:

- `cargo make docker-build <image/name>` - Build Docker image and push it to the registry
- `cargo make git-tag` - Create and push a new git tag. The Version number is read from `Cargo.toml`

## License

MIT
