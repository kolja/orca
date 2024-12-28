
<img src="./orca-logo.svg" alt="an orca whale reading a book" width="200px" height="200px">

# ORCA

![latest](https://img.shields.io/github/v/tag/kolja/orca)
[![build](https://github.com/kolja/orca/actions/workflows/rust.yml/badge.svg)](https://github.com/kolja/orca/actions)
[![Coverage Status](https://coveralls.io/repos/github/kolja/orca/badge.svg?branch=main)](https://coveralls.io/github/kolja/orca?branch=main)
[![dependency status](https://deps.rs/repo/github/kolja/orca/status.svg?path=%2F)](https://deps.rs/repo/github/kolja/orca?path=%2F)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

A simple OPDS Server for Calibre written in Rust.
It supports http and https, basic authentication and multiple libraries.

## Installation

### Homebrew
```bash
brew tap kolja/orca
brew install orca-server
```
### Docker
```bash
docker pull koljaw/orca
docker run -p 8080:8080 \
-e ORCA_CONFIG=/app/orca.toml \
-v /path/to/your/config.toml:/app/orca.toml \
-v /path/to/your/library:/app/library \
koljaw/orca:latest
```

## Configuration

If you point the environment variable `ORCA_CONFIG` to a `.toml` file, that's where the configuration will be read from. Otherwise, it will look for a file named `orca.toml` or `orca/config.toml` in `$HOME/.config/`.

The server will either start as HTTP or HTTPS server depending on the value of 'protocol'. If you set it to 'https', you have to provide a path to a certificate and a key file.
```toml
[server]
ip = "<your_ip>"
port = 8080
protocol = "Https" # or "Http"
cert = "/path/to/cert.pem"
key = "/path/to/key.pem"

[authentication.login]
alice = "$argon2id$v=19$m=19456,t=2,p=1$bK0qYfzAokhthFP0fKBQvg$QPPf54SN74dT2YX4aGoN+KxoWD+xV+c6OBrrPnvxj24"
bob = "$argon2id$v=19$m=19456,t=2,p=1$FMnONzRzIAkaIuy3c+A9cg$DE3+UC62d/f+L0jqEWgz9GAfNWQkKfugeZFSL/FG5XQ"

[authentication]
public = ["/", "/library/**"] # the root endpoint is publicly accessible, so is everything under /library

[calibre.libraries]
library = "/Volumes/library"
nonfiction = "/Volumes/nonfiction"
```

## Authentication

The server supports basic authentication: You can generate a password hash like so:
```bash
orca --hash <login>:<password> # e.g. orca --hash alice:secretpassword
```
The server will print the hash which you have to copy to the `[authentication.login]` section of your config file.

Under the `public` array in the `[authentication]` section you can specify which paths should be accessible without authentication. You can use wildcards like `*` and `**` to match multiple paths.

## Development

There are a couple of tasks you can run with `cargo make`:

- `cargo make docker-build <image/name>` - Build Docker image and push it to the registry
- `cargo make git-tag` - Create and push a new git tag. The Version number is read from `Cargo.toml`
- `cargo make list-sha` - List all the sha256 hashes for the Assets in the Release (for use with the the homebrew formula)

## License

MIT
