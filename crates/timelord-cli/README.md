# timelord-cli

[![Crates.io](https://img.shields.io/crates/v/timelord-cli.svg)](https://crates.io/crates/timelord-cli)
[![Documentation](https://docs.rs/timelord-cli/badge.svg)](https://docs.rs/timelord-cli)
[![License: MIT OR Apache-2.0](https://img.shields.io/crates/l/timelord-cli.svg)](LICENSE)

A command-line interface for [timelord](https://crates.io/crates/timelord), a Rust tool to preserve file timestamps (mtime) between CI builds, even with fresh git checkouts.

## Installation

```bash
cargo install timelord-cli
```

## Usage

```bash
timelord --source-dir <SOURCE_DIR> --cache-dir <CACHE_DIR>
```

- `<SOURCE_DIR>`: Directory containing files to preserve timestamps for
- `<CACHE_DIR>`: Persistent directory to store the timestamp database across CI builds

For more detailed information on how timelord works and additional configuration options, please refer to the [timelord library documentation](https://docs.rs/timelord).

## License

Licensed under either of

 * Apache License, Version 2.0
   ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
 * MIT license
   ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.
