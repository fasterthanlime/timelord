# timelord

[![Crates.io](https://img.shields.io/crates/v/timelord.svg)](https://crates.io/crates/timelord)
[![Documentation](https://docs.rs/timelord/badge.svg)](https://docs.rs/timelord)
[![License: MIT OR Apache-2.0](https://img.shields.io/crates/l/timelord.svg)](LICENSE)

A Rust library to preserve file timestamps (mtime) between builds, even with fresh git checkouts.

## Usage

Timelord provides the `sync` function to preserve file timestamps between builds:

```rust
use timelord::sync;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    sync("path/to/source", "path/to/cache")?;
    Ok(())
}
```

The `sync` function takes two arguments:
- `source_dir`: Directory containing files to preserve timestamps for
- `cache_dir`: Persistent directory to store the timestamp database across builds

Timelord stores a database of file sizes and hashes, and restores old timestamps if file contents remain unchanged.

For CLI usage, see the [`timelord-cli`](https://crates.io/crates/timelord-cli) crate.

## Additional Configuration

To ensure Timelord works properly, especially in CI environments, it's important to use the `-Zremap-cwd-prefix` rustc flag. This flag helps maintain consistent paths across different build environments.

## License

Licensed under either of

 * Apache License, Version 2.0
   ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
 * MIT license
   ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.
