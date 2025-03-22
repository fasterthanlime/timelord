# timelord

[![Crates.io](https://img.shields.io/crates/v/timelord.svg)](https://crates.io/crates/timelord)
[![Documentation](https://docs.rs/timelord/badge.svg)](https://docs.rs/timelord)
[![License: MIT OR Apache-2.0](https://img.shields.io/crates/l/timelord.svg)](LICENSE)

A Rust tool to preserve file timestamps (mtime) between CI builds, even with fresh git checkouts.

## Usage

Timelord preserves file timestamps between CI builds, even with fresh git checkouts. It achieves this by storing a database of file sizes and hashes, and restoring old timestamps if file contents remain unchanged.

```bash
timelord --source-dir <SOURCE_DIR> --cache-dir <CACHE_DIR>
```

- `<SOURCE_DIR>`: Directory containing files to preserve timestamps for
- `<CACHE_DIR>`: Persistent directory to store the timestamp database across CI builds

Timelord essentially implements the functionality of Cargo's unstable "checksum-freshness" feature (https://doc.rust-lang.org/cargo/reference/unstable.html#checksum-freshness), but for stable Rust.

The cache file (`timelord.db`) is stored in the specified cache directory and should be preserved between runs for optimal functionality.

## Additional Configuration

To ensure Timelord works properly, especially in CI environments, it's important to use the `-Zremap-cwd-prefix` rustc flag (https://doc.rust-lang.org/beta/unstable-book/compiler-flags/remap-cwd-prefix.html). This flag helps maintain consistent paths across different build environments.

## License

Licensed under either of

 * Apache License, Version 2.0
   ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
 * MIT license
   ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.
