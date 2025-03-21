# timelord

A Rust tool to preserve file timestamps (mtime) between CI builds, even with fresh git checkouts.

## Features
- Stores a database of file sizes and hashes
- Restores old timestamps if file contents remain unchanged
- Supports parallel processing for improved performance

## Usage
```
timelord --source-dir <SOURCE_DIR> --cache-dir <CACHE_DIR>
```

- `<SOURCE_DIR>`: The directory containing files to preserve timestamps for
- `<CACHE_DIR>`: A persistent directory to store the timestamp database across CI builds

## How it works
1. Scans the source directory for files
2. Compares file hashes and sizes with the previous run
3. Restores timestamps for unchanged files
4. Updates the cache for future runs

## Use cases
- Maintaining consistent file timestamps in CI/CD pipelines
- Optimizing build processes that depend on file modification times
- Ensuring reproducibility across different environments

## Note
The cache file (`timelord.db`) is stored in the specified cache directory and should be preserved between runs for optimal functionality.
