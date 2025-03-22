// Always use eprintln! instead of println! for output
use camino::Utf8PathBuf;
use ignore::DirEntry;
use ignore::WalkBuilder;
use log::*;
use rayon::iter::IntoParallelRefIterator;
use rayon::iter::ParallelIterator;
use serde::Deserialize;
use serde::Serialize;
use std::collections::BTreeMap;
use std::fs::{self, File};
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
pub struct Workspace {
    pub source_dir: Utf8PathBuf,
}

/// Represents a relative path within the workspace
#[derive(Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Clone)]
#[serde(transparent)]
pub struct RelativePath(Utf8PathBuf);

impl RelativePath {
    /// Converts the relative path to an absolute path within the workspace
    pub fn to_absolute_path(&self, workspace: &Workspace) -> Utf8PathBuf {
        workspace.source_dir.join(&self.0)
    }
}

#[derive(Serialize, Deserialize)]
pub struct HashedFile {
    /// The relative path of the file within the workspace
    pub path: RelativePath,
    /// A hash of the file's contents
    pub hash: Hash,
    /// The size of the file in bytes
    pub size: u64,
    /// The mtime of the file (last we checked)
    pub timestamp: std::time::SystemTime,
}

/// The seahash of a file
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(transparent)]
pub struct Hash(pub u64);

impl std::fmt::Display for Hash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:016x}", self.0)
    }
}

pub const TIMELORD_CACHE_VERSION: u32 = 3;

#[derive(Serialize, Deserialize)]
pub struct Cache {
    pub entries: BTreeMap<RelativePath, HashedFile>,
    pub version: u32,
    pub crawl_time: std::time::SystemTime,
    pub absolute_path: Utf8PathBuf,
    pub hostname: String,
}

impl Cache {
    pub fn new(absolute_path: Utf8PathBuf) -> Self {
        Cache {
            entries: BTreeMap::new(),
            version: TIMELORD_CACHE_VERSION,
            crawl_time: std::time::SystemTime::now(),
            absolute_path,
            hostname: hostname::get().unwrap().to_string_lossy().into_owned(),
        }
    }
}

pub fn walk_source_dir(workspace: &Workspace) -> Cache {
    let entries = Arc::new(Mutex::new(BTreeMap::new()));

    WalkBuilder::new(&workspace.source_dir)
        .standard_filters(false)
        .build_parallel()
        .run(|| {
            let entries_clone = Arc::clone(&entries);
            let workspace = workspace.clone();
            Box::new(move |entry: Result<DirEntry, ignore::Error>| {
                let entry = entry.unwrap();
                if entry.file_type().is_some_and(|ft| ft.is_file()) {
                    let path =
                        Utf8PathBuf::try_from(entry.path().to_owned()).unwrap_or_else(|_| {
                            panic!("Non-UTF-8 filepath encountered: {}", entry.path().display())
                        });
                    let relative_path =
                        RelativePath(path.strip_prefix(&workspace.source_dir).unwrap().to_owned());
                    let mut file = File::open(&path).unwrap();
                    let mut contents = Vec::new();
                    file.read_to_end(&mut contents).unwrap();
                    let hash = Hash(seahash::hash(&contents));

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

    let mut source_dir = Cache::new(workspace.source_dir.clone());
    source_dir.entries = entries;
    source_dir
}

use owo_colors::OwoColorize;
use std::thread;

pub fn bad_cache_disclaimer(message: &str) {
    warn!("{}", "=".repeat(80).red());
    warn!("⚠️  {} ⚠️", message.bold().red());
    warn!("{}", "=".repeat(80).red());
}

pub fn read_cache(cache_file: &Utf8PathBuf) -> Option<Cache> {
    if !cache_file.exists() {
        debug!("🆕 No cache file found at {}, starting fresh!", cache_file);
        return None;
    }
    debug!("🔍 Reading cache file: {}", cache_file);

    let contents = match fs::read(cache_file) {
        Ok(c) => c,
        Err(e) => {
            bad_cache_disclaimer(&format!("Failed to read cache file: {}", e));
            return None;
        }
    };

    let (source_dir, _) =
        match bincode::serde::decode_from_slice::<Cache, _>(&contents, bincode::config::standard())
        {
            Ok(result) => result,
            Err(e) => {
                bad_cache_disclaimer(&format!("Failed to deserialize cache: {}", e));
                return None;
            }
        };

    if source_dir.version != TIMELORD_CACHE_VERSION {
        bad_cache_disclaimer("Cache file has wrong version, starting fresh!");
        return None;
    }

    Some(source_dir)
}

pub fn read_or_create_cache(cache_file: &Utf8PathBuf) -> Cache {
    let start = Instant::now();
    let old_source_dir = match read_cache(cache_file) {
        Some(cache) => cache,
        None => {
            debug!("⚠️ Falling back to empty cache");
            Cache::new(Utf8PathBuf::new())
        }
    };
    let deserialize_time = start.elapsed();
    debug!("⏰ Deserialization took: {:?}", deserialize_time);
    old_source_dir
}

fn scan_source_directory(workspace: &Workspace) -> Cache {
    debug!("🔍 Scanning source directory: {}", workspace.source_dir);
    let scan_start = Instant::now();
    let new_source_dir = walk_source_dir(workspace);
    let scan_time = scan_start.elapsed();
    debug!("⏰ Directory scan took: {:?}", scan_time);
    new_source_dir
}

fn update_timestamps(old_source_dir: &Cache, new_source_dir: &Cache, workspace: &Workspace) {
    debug!("⏰ Updating file timestamps...");
    let update_start = Instant::now();

    let fresh_count = AtomicUsize::new(0);
    let dirty_count = AtomicUsize::new(0);
    new_source_dir
        .entries
        .par_iter()
        .for_each(|(path, new_entry)| {
            #[derive(Debug)]
            enum DirtyReason {
                New,
                HashChanged,
                SizeChanged,
            }

            let old_entry = old_source_dir.entries.get(path);
            let cause = if let Some(old_entry) = old_entry {
                if new_entry.hash != old_entry.hash {
                    Some(DirtyReason::HashChanged)
                } else if new_entry.size != old_entry.size {
                    Some(DirtyReason::SizeChanged)
                } else {
                    None
                }
            } else {
                Some(DirtyReason::New)
            };

            if let Some(cause) = cause {
                dirty_count.fetch_add(1, Ordering::Relaxed);
                let dirty_count_so_far = dirty_count.load(Ordering::Relaxed);
                if dirty_count_so_far <= 5 {
                    debug!(
                        "  {} {} ({}, {}) - {:?}",
                        "[dirty]".red(),
                        path.0,
                        new_entry.hash,
                        human_bytes::human_bytes(new_entry.size as f64),
                        cause
                    );
                } else if dirty_count_so_far == 5 {
                    debug!("  {}", "(other dirty files ignored)");
                }
            } else {
                let old_entry = old_entry.unwrap();
                if new_entry.timestamp != old_entry.timestamp {
                    let absolute_path = path.to_absolute_path(workspace);
                    std::fs::File::open(&absolute_path)
                        .and_then(|f| f.set_modified(old_entry.timestamp))
                        .unwrap_or_else(|e| {
                            warn!("❌ Failed to set mtime for {}: {}", absolute_path, e);
                        });
                }
                let fresh_count_so_far = fresh_count.fetch_add(1, Ordering::Relaxed);
                #[allow(clippy::comparison_chain)]
                if fresh_count_so_far < 5 {
                    debug!(
                        "  {} {} ({}, {}, {} => {})",
                        "[fresh]".green(),
                        path.0,
                        new_entry.hash,
                        human_bytes::human_bytes(new_entry.size as f64),
                        format_timestamp(old_entry.timestamp),
                        format_timestamp_diff(old_entry.timestamp, new_entry.timestamp)
                    );
                } else if fresh_count_so_far == 5 {
                    debug!("  {}", "(other fresh files ignored)");
                }
            }
        });

    let fresh_count = fresh_count.load(Ordering::Relaxed);
    let dirty_count = dirty_count.load(Ordering::Relaxed);
    let update_time = update_start.elapsed();
    debug!(
        "⏰ Spent {:?} syncing ({} fresh, {} dirty)",
        update_time, fresh_count, dirty_count
    );
}

fn save_new_cache(new_source_dir: &Cache, cache_file: &Utf8PathBuf) {
    debug!("💾 Saving new cache to {}", cache_file);
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
    debug!("⏰ Cache serialization took: {:?}", serialize_time);
}

pub fn sync(source_dir: Utf8PathBuf, cache_dir: Utf8PathBuf) {
    let cache_file = cache_dir.join("timelord.db");
    let start = Instant::now();

    let workspace = Workspace { source_dir };

    let (old_source_dir, new_source_dir) = {
        let cache_file = cache_file.clone();
        let workspace = workspace.clone();
        let cache_reader_handle = thread::spawn(move || {
            let sd = read_or_create_cache(&cache_file);
            print_cache_info(&sd, &cache_file);
            sd
        });
        let source_scanner_handle = thread::spawn(move || scan_source_directory(&workspace));
        (
            cache_reader_handle.join().unwrap(),
            source_scanner_handle.join().unwrap(),
        )
    };

    let old_source_dir = Arc::new(old_source_dir);
    let new_source_dir = Arc::new(new_source_dir);

    let (timestamp_updater_handle, cache_saver_handle) = {
        let old_source_dir = Arc::clone(&old_source_dir);
        let new_source_dir1 = Arc::clone(&new_source_dir);
        let new_source_dir2 = Arc::clone(&new_source_dir);
        let cache_file = cache_file.clone();
        let workspace = workspace.clone();
        let timestamp_updater_handle =
            thread::spawn(move || update_timestamps(&old_source_dir, &new_source_dir1, &workspace));
        let cache_saver_handle =
            thread::spawn(move || save_new_cache(&new_source_dir2, &cache_file));
        (timestamp_updater_handle, cache_saver_handle)
    };

    timestamp_updater_handle.join().unwrap();
    cache_saver_handle.join().unwrap();

    let total_time = start.elapsed();
    info!(
        "🎉 All done! Restored {} files in {:?}",
        new_source_dir.entries.len(),
        total_time
    );
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
                debug!(
                    "{}{}/  ({} files, {})",
                    prefix,
                    name.blue(),
                    self.files,
                    human_bytes::human_bytes(self.total_size as f64)
                );
            } else {
                debug!("{}{}/ ({})", prefix, name.blue(), "empty".red());
            }
        } else {
            debug!(
                "{}{}/  ({} files, {})",
                prefix,
                name.blue(),
                self.files,
                human_bytes::human_bytes(self.total_size as f64)
            );
            for (subdir_name, subdir_info) in &self.subdirectories {
                subdir_info.print(&format!("{}  ", prefix), subdir_name);
            }
        }
    }
}

pub fn cache_info(cache_dir: Utf8PathBuf) {
    let cache_file = cache_dir.join("timelord.db");
    if !cache_file.exists() {
        warn!("❌ Cache file not found: {}", cache_file);
        return;
    }

    let source_dir = match read_cache(&cache_file) {
        Some(source_dir) => source_dir,
        None => {
            warn!("❌ Failed to read cache file: {}", cache_file);
            return;
        }
    };
    print_cache_info(&source_dir, &cache_file);
}

fn print_cache_info(cache: &Cache, cache_file: &Utf8PathBuf) {
    let cache_size = match fs::metadata(cache_file) {
        Ok(metadata) => metadata.len(),
        Err(_) => {
            debug!("Cache not created yet");
            return;
        }
    };
    debug!(
        "   Cache is {}, tracking {} entries (version {})",
        human_bytes::human_bytes(cache_size as f64),
        cache.entries.len(),
        cache.version,
    );
    debug!(
        "   Crawled {} ago ({}) on {} from source dir {}",
        humantime::format_duration(
            std::time::SystemTime::now()
                .duration_since(cache.crawl_time)
                .unwrap()
        ),
        format_timestamp(cache.crawl_time),
        cache.hostname,
        cache.absolute_path
    );

    let mut root = DirectoryInfo::new();
    for (path, file) in &cache.entries {
        let mut current = &mut root;
        for name in path.0.components().take(path.0.components().count() - 1) {
            current = current
                .subdirectories
                .entry(name.to_string())
                .or_insert_with(DirectoryInfo::new);
        }
        current.add_file(file.size);
    }

    debug!("📁 Directory Structure:");
    root.print("  ", ".");
}

fn format_timestamp(timestamp: std::time::SystemTime) -> String {
    jiff::Timestamp::try_from(timestamp)
        .unwrap()
        .strftime("%Y-%m-%d %H:%M:%S")
        .to_string()
}

fn format_timestamp_diff(old: std::time::SystemTime, new: std::time::SystemTime) -> String {
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
