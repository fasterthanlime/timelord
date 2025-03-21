use camino::Utf8PathBuf;
use clap::Parser;
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

#[derive(Serialize, Deserialize)]
struct HashedFile {
    path: Utf8PathBuf,
    hash: u64,
    size: u64,
    timestamp: std::time::SystemTime,
}

#[derive(Serialize, Deserialize)]
struct SourceDir {
    entries: BTreeMap<Utf8PathBuf, HashedFile>,
}

fn walk_source_dir(path: &Utf8PathBuf) -> SourceDir {
    let entries = Arc::new(Mutex::new(BTreeMap::new()));

    WalkBuilder::new(path).build_parallel().run(|| {
        let entries_clone = Arc::clone(&entries);
        Box::new(move |entry: Result<DirEntry, ignore::Error>| {
            let entry = entry.unwrap();
            if entry.file_type().is_some_and(|ft| ft.is_file()) {
                let path = Utf8PathBuf::try_from(entry.into_path())
                    .expect("timelord only abides by utf-8 files");
                let mut file = File::open(&path).unwrap();
                let mut contents = Vec::new();
                file.read_to_end(&mut contents).unwrap();

                let mut hasher = DefaultHasher::new();
                contents.hash(&mut hasher);
                let hash = hasher.finish();

                let size = contents.len() as u64;
                let timestamp = file.metadata().unwrap().modified().unwrap();

                entries_clone.lock().unwrap().insert(
                    path.clone(),
                    HashedFile {
                        path,
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

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
/// A tool to preserve file timestamps (mtime) between CI builds, even with fresh git checkouts.
///
/// This tool works by storing a database of file sizes and hashes. It requires two directories:
/// 1. A source directory where it will restore old timestamps if file contents remain the same.
/// 2. A cache directory that should be persistent across CI builds to store the timestamp database.
struct Args {
    /// The source directory containing files to preserve timestamps for.
    #[arg(long)]
    source_dir: Utf8PathBuf,

    /// The cache directory to store the timestamp database, should be persistent across CI builds.
    /// The file will be written in the cache directory as `timelord.db`.
    #[arg(long)]
    cache_dir: Utf8PathBuf,
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

fn scan_source_directory(source_dir: &Utf8PathBuf) -> SourceDir {
    eprintln!(
        "🔍 Scanning source directory: {}",
        source_dir.to_string().blue()
    );
    let scan_start = Instant::now();
    let new_source_dir = walk_source_dir(source_dir);
    let scan_time = scan_start.elapsed();
    eprintln!("⏱️  Directory scan took: {:?}", scan_time.blue());
    new_source_dir
}

fn update_timestamps(old_source_dir: &SourceDir, new_source_dir: &SourceDir) {
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
                    std::fs::File::open(path)
                        .and_then(|f| f.set_modified(old_entry.timestamp))
                        .unwrap_or_else(|e| {
                            eprintln!("❌ Failed to set mtime for {}: {}", path.red(), e);
                        });
                    updated_count.fetch_add(1, Ordering::Relaxed);
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
        "💾 Cache file size: {} bytes",
        cache_size.to_string().yellow()
    );
}

fn main() {
    let args = Args::parse();

    let cache_file = args.cache_dir.join("timelord.db");
    let start = Instant::now();

    let (old_source_dir, new_source_dir) = {
        let cache_file_clone = cache_file.clone();
        let source_dir_clone = args.source_dir.clone();
        let cache_reader_handle = thread::spawn(move || read_or_create_cache(&cache_file_clone));
        let source_scanner_handle = thread::spawn(move || scan_source_directory(&source_dir_clone));
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
        let timestamp_updater_handle =
            thread::spawn(move || update_timestamps(&old_source_dir_clone, &new_source_dir_clone1));
        let cache_saver_handle =
            thread::spawn(move || save_new_cache(&new_source_dir_clone2, &cache_file_clone));
        (timestamp_updater_handle, cache_saver_handle)
    };

    timestamp_updater_handle.join().unwrap();
    cache_saver_handle.join().unwrap();

    let total_time = start.elapsed();
    eprintln!("🎉 All done! Total time: {:?}", total_time.green());
}
