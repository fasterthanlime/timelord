// Always use eprintln! instead of println! for output
use camino::Utf8PathBuf;
use clap::{Parser, Subcommand};
use ignore::DirEntry;
use ignore::WalkBuilder;
use rayon::iter::IntoParallelRefIterator;
use rayon::iter::ParallelIterator;
use serde::Deserialize;
use serde::Serialize;
use std::collections::BTreeMap;
use std::collections::hash_map::DefaultHasher;
use std::fs::{self, File};
use std::hash::{Hash, Hasher};
use std::io::Read;
use std::io::Write;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::Instant;

#[cfg(test)]
mod tests;

/// Represents a workspace with a source directory
#[derive(Clone)]
struct Workspace {
    source_dir: Utf8PathBuf,
}

/// Represents a relative path within the workspace
#[derive(Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Clone)]
#[serde(transparent)]
struct RelativePath(Utf8PathBuf);

impl RelativePath {
    /// Converts the relative path to an absolute path within the workspace
    fn to_absolute_path(&self, workspace: &Workspace) -> Utf8PathBuf {
        workspace.source_dir.join(&self.0)
    }
}

#[derive(Serialize, Deserialize)]
struct HashedFile {
    path: RelativePath,
    hash: u64,
    size: u64,
    timestamp: std::time::SystemTime,
}

#[derive(Serialize, Deserialize)]
struct SourceDir {
    entries: BTreeMap<RelativePath, HashedFile>,
}

fn walk_source_dir(workspace: &Workspace) -> SourceDir {
    let entries = Arc::new(Mutex::new(BTreeMap::new()));

    WalkBuilder::new(&workspace.source_dir)
        .build_parallel()
        .run(|| {
            let entries_clone = Arc::clone(&entries);
            let workspace = workspace.clone();
            Box::new(move |entry: Result<DirEntry, ignore::Error>| {
                let entry = entry.unwrap();
                if entry.file_type().is_some_and(|ft| ft.is_file()) {
                    let path = Utf8PathBuf::try_from(entry.into_path())
                        .expect("timelord only abides by utf-8 files");
                    let relative_path =
                        RelativePath(path.strip_prefix(&workspace.source_dir).unwrap().to_owned());
                    let mut file = File::open(&path).unwrap();
                    let mut contents = Vec::new();
                    file.read_to_end(&mut contents).unwrap();

                    let mut hasher = DefaultHasher::new();
                    contents.hash(&mut hasher);
                    let hash = hasher.finish();

                    let size = contents.len() as u64;
                    let timestamp = file.metadata().unwrap().modified().unwrap();

                    entries_clone.lock().unwrap().insert(
                        relative_path.clone(),
                        HashedFile {
                            path: relative_path,
                            hash,
                            size,
                            timestamp,
                        },
                    );
                }
                ignore::WalkState::Continue
            })
        });

    let entries = Arc::try_unwrap(entries)
        .unwrap_or_else(|_| unreachable!())
        .into_inner()
        .expect("Failed to get inner value");

    SourceDir { entries }
}

#[derive(Parser, Debug, Clone)]
#[command(author, version, about, long_about = None)]
/// A tool to preserve file timestamps (mtime) between CI builds, even with fresh git checkouts.
struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug, Clone)]
enum Command {
    /// Synchronize timestamps between the source directory and cache
    Sync {
        /// The source directory containing files to preserve timestamps for.
        #[arg(long)]
        source_dir: Utf8PathBuf,

        /// The cache directory to store the timestamp database, should be persistent across CI builds.
        /// The file will be written in the cache directory as `timelord.db`.
        #[arg(long)]
        cache_dir: Utf8PathBuf,
    },
    /// Display information about the cache
    CacheInfo {
        /// The cache directory containing the timelord.db file
        #[arg(long)]
        cache_dir: Utf8PathBuf,
    },
}

use owo_colors::OwoColorize;
use std::thread;

fn read_or_create_cache(cache_file: &Utf8PathBuf) -> SourceDir {
    let start = Instant::now();
    let old_source_dir = if cache_file.exists() {
        eprintln!("🔍 Reading cache file: {}", cache_file.to_string().blue());
        let contents = fs::read(cache_file).expect("Failed to read cache file");
        bincode::serde::decode_from_slice(&contents, bincode::config::standard())
            .expect("Failed to deserialize cache")
            .0
    } else {
        eprintln!(
            "🆕 No cache file found at {}, starting fresh!",
            cache_file.to_string().blue()
        );
        SourceDir {
            entries: BTreeMap::new(),
        }
    };
    let deserialize_time = start.elapsed();
    eprintln!("⏱️  Deserialization took: {:?}", deserialize_time.blue());
    eprintln!(
        "📊 Old cache entries: {}",
        old_source_dir.entries.len().to_string().yellow()
    );
    old_source_dir
}

fn scan_source_directory(workspace: &Workspace) -> SourceDir {
    eprintln!(
        "🔍 Scanning source directory: {}",
        workspace.source_dir.to_string().blue()
    );
    let scan_start = Instant::now();
    let new_source_dir = walk_source_dir(workspace);
    let scan_time = scan_start.elapsed();
    eprintln!("⏱️  Directory scan took: {:?}", scan_time.blue());
    new_source_dir
}

fn update_timestamps(
    old_source_dir: &SourceDir,
    new_source_dir: &SourceDir,
    workspace: &Workspace,
) {
    eprintln!("🕰️  Updating file timestamps...");
    let update_start = Instant::now();

    let updated_count = AtomicUsize::new(0);
    let different_count = AtomicUsize::new(0);
    new_source_dir
        .entries
        .par_iter()
        .for_each(|(path, new_entry)| {
            if let Some(old_entry) = old_source_dir.entries.get(path) {
                if new_entry.hash == old_entry.hash && new_entry.size == old_entry.size {
                    let absolute_path = path.to_absolute_path(workspace);
                    std::fs::File::open(&absolute_path)
                        .and_then(|f| f.set_modified(old_entry.timestamp))
                        .unwrap_or_else(|e| {
                            eprintln!("❌ Failed to set mtime for {}: {}", absolute_path.red(), e);
                        });
                    let current_count = updated_count.fetch_add(1, Ordering::Relaxed);
                    #[allow(clippy::comparison_chain)]
                    if current_count < 5 {
                        fn format_timestamp(timestamp: std::time::SystemTime) -> String {
                            jiff::Timestamp::try_from(timestamp)
                                .unwrap()
                                .strftime("%Y-%m-%d %H:%M:%S")
                                .to_string()
                        }
                        fn format_timestamp_diff(
                            old: std::time::SystemTime,
                            new: std::time::SystemTime,
                        ) -> String {
                            let old_str = format_timestamp(old);
                            let new_str = format_timestamp(new);
                            let mut result = String::new();
                            for (old_char, new_char) in old_str.chars().zip(new_str.chars()) {
                                if old_char == new_char {
                                    result.push(new_char);
                                } else {
                                    result.push_str(&new_char.to_string().red().to_string());
                                }
                            }
                            result
                        }

                        eprintln!(
                            "🔄 {} ({:#016x}, {}, {} => {})",
                            path.0.to_string().green(),
                            new_entry.hash.blue(),
                            human_bytes::human_bytes(new_entry.size as f64).yellow(),
                            format_timestamp(old_entry.timestamp).red(),
                            format_timestamp_diff(old_entry.timestamp, new_entry.timestamp)
                        );
                    } else if current_count == 5 {
                        eprintln!("(not printing rest of paths, we were just giving examples)");
                    }
                } else {
                    different_count.fetch_add(1, Ordering::Relaxed);
                }
            } else {
                different_count.fetch_add(1, Ordering::Relaxed);
            }
        });

    let updated_count = updated_count.load(Ordering::Relaxed);
    let different_count = different_count.load(Ordering::Relaxed);
    let update_time = update_start.elapsed();
    eprintln!("⏱️  Timestamp update took: {:?}", update_time.blue());
    eprintln!("✅ Restored {} file timestamps", updated_count.green());
    eprintln!(
        "🔄 Found {} different or new files",
        different_count.yellow()
    );
}

fn save_new_cache(new_source_dir: &SourceDir, cache_file: &Utf8PathBuf) {
    eprintln!("💾 Saving new cache to {}", cache_file.to_string().blue());
    let serialize_start = Instant::now();
    let serialized = bincode::serde::encode_to_vec(new_source_dir, bincode::config::standard())
        .expect("Failed to serialize new source dir");

    // Create the directory if it doesn't exist
    if let Some(parent) = cache_file.parent() {
        fs::create_dir_all(parent).expect("Failed to create cache directory");
    }

    let mut file = File::create(cache_file).expect("Failed to create cache file");
    file.write_all(&serialized)
        .expect("Failed to write cache file");
    let serialize_time = serialize_start.elapsed();
    eprintln!("⏱️  Cache serialization took: {:?}", serialize_time.blue());

    let cache_size = fs::metadata(cache_file)
        .expect("Failed to get cache file metadata")
        .len();
    eprintln!(
        "📊 New cache entries: {}",
        new_source_dir.entries.len().to_string().yellow()
    );
    eprintln!(
        "💾 Cache file size: {}",
        human_bytes::human_bytes(cache_size as f64).yellow()
    );
}

fn main() {
    let args = Args::parse();
    main_with_args(args);
}

fn main_with_args(args: Args) {
    match args.command {
        Command::Sync {
            source_dir,
            cache_dir,
        } => {
            sync(source_dir, cache_dir);
        }
        Command::CacheInfo { cache_dir } => {
            cache_info(cache_dir);
        }
    }
}

fn sync(source_dir: Utf8PathBuf, cache_dir: Utf8PathBuf) {
    let cache_file = cache_dir.join("timelord.db");
    let start = Instant::now();

    let workspace = Workspace { source_dir };

    let (old_source_dir, new_source_dir) = {
        let cache_file_clone = cache_file.clone();
        let workspace_clone = workspace.clone();
        let cache_reader_handle = thread::spawn(move || read_or_create_cache(&cache_file_clone));
        let source_scanner_handle = thread::spawn(move || scan_source_directory(&workspace_clone));
        (
            cache_reader_handle.join().unwrap(),
            source_scanner_handle.join().unwrap(),
        )
    };

    let old_source_dir = Arc::new(old_source_dir);
    let new_source_dir = Arc::new(new_source_dir);

    let (timestamp_updater_handle, cache_saver_handle) = {
        let old_source_dir_clone = Arc::clone(&old_source_dir);
        let new_source_dir_clone1 = Arc::clone(&new_source_dir);
        let new_source_dir_clone2 = Arc::clone(&new_source_dir);
        let cache_file_clone = cache_file.clone();
        let workspace_clone = workspace.clone();
        let timestamp_updater_handle = thread::spawn(move || {
            update_timestamps(
                &old_source_dir_clone,
                &new_source_dir_clone1,
                &workspace_clone,
            )
        });
        let cache_saver_handle =
            thread::spawn(move || save_new_cache(&new_source_dir_clone2, &cache_file_clone));
        (timestamp_updater_handle, cache_saver_handle)
    };

    timestamp_updater_handle.join().unwrap();
    cache_saver_handle.join().unwrap();

    let total_time = start.elapsed();
    eprintln!("🎉 All done! Total time: {:?}", total_time.green());
}

#[derive(Debug, Clone)]
struct DirectoryInfo {
    files: usize,
    total_size: u64,
    subdirectories: BTreeMap<String, DirectoryInfo>,
}

impl DirectoryInfo {
    fn new() -> Self {
        DirectoryInfo {
            files: 0,
            total_size: 0,
            subdirectories: BTreeMap::new(),
        }
    }

    fn add_file(&mut self, size: u64) {
        self.files += 1;
        self.total_size += size;
    }

    fn print(&self, prefix: &str, name: &str) {
        use owo_colors::OwoColorize;
        if self.subdirectories.is_empty() {
            if self.files > 0 {
                eprintln!(
                    "{}{}/  ({} files, {})",
                    prefix,
                    name.blue(),
                    self.files.to_string().yellow(),
                    human_bytes::human_bytes(self.total_size as f64).green()
                );
            } else {
                eprintln!("{}{}/  ({})", prefix, name.blue(), "empty".red());
            }
        } else {
            eprintln!(
                "{}{}/  ({} files, {})",
                prefix,
                name.blue(),
                self.files.to_string().yellow(),
                human_bytes::human_bytes(self.total_size as f64).green()
            );
            for (subdir_name, subdir_info) in &self.subdirectories {
                subdir_info.print(&format!("{}  ", prefix), subdir_name);
            }
        }
    }
}

fn cache_info(cache_dir: Utf8PathBuf) {
    let cache_file = cache_dir.join("timelord.db");
    if !cache_file.exists() {
        eprintln!("❌ Cache file not found: {}", cache_file.to_string().red());
        return;
    }

    let source_dir = read_or_create_cache(&cache_file);
    let cache_size = fs::metadata(&cache_file).unwrap().len();

    eprintln!("{}", "📊 Cache Information:".blue());
    eprintln!("   Cache file: {}", cache_file.to_string().blue());
    eprintln!(
        "   Entries: {}",
        source_dir.entries.len().to_string().yellow()
    );
    eprintln!(
        "   Size: {}",
        human_bytes::human_bytes(cache_size as f64).green()
    );

    let mut root = DirectoryInfo::new();
    for (path, file) in &source_dir.entries {
        let mut current = &mut root;
        for name in path.0.components().take(path.0.components().count() - 1) {
            current = current
                .subdirectories
                .entry(name.to_string())
                .or_insert_with(DirectoryInfo::new);
        }
        current.add_file(file.size);
    }

    eprintln!("\n📁 Directory Structure:");
    root.print("", ".");
}
