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

#[derive(Parser, Debug, Clone)]
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
    // Check for self-test flag
    if std::env::args().any(|arg| arg == "--self-test") {
        self_test();
        std::process::exit(0);
    }

    let args = Args::parse();
    main_with_args(args);
}

fn main_with_args(args: Args) {
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

fn self_test() {
    use std::fs::{self, File};
    use std::io::Write;
    use std::time::SystemTime;

    eprintln!(
        "{}",
        "===============================================".blue()
    );
    eprintln!("{}", "Starting Timelord Self-Test".green());
    eprintln!(
        "{}",
        "===============================================".blue()
    );

    // Create temporary directories for source and cache
    let temp_dir = tempfile::tempdir().unwrap();
    let source_dir = temp_dir.path().join("source");
    let cache_dir = temp_dir.path().join("cache");
    fs::create_dir_all(&source_dir).unwrap();
    fs::create_dir_all(&cache_dir).unwrap();
    eprintln!("{}", "Created temporary directories:".yellow());
    eprintln!("  Source: {}", source_dir.display().blue());
    eprintln!("  Cache: {}", cache_dir.display().blue());

    // Create some test files
    let file1_path = source_dir.join("file1.txt");
    let file2_path = source_dir.join("file2.txt");
    let mut file1 = File::create(&file1_path).unwrap();
    let mut file2 = File::create(&file2_path).unwrap();
    file1.write_all(b"Hello, World!").unwrap();
    file2.write_all(b"Timelord test").unwrap();
    eprintln!("{}", "Created test files:".yellow());
    eprintln!("  {}: 'Hello, World!'", file1_path.display().blue());
    eprintln!("  {}: 'Timelord test'", file2_path.display().blue());

    eprintln!(
        "\n{}",
        "===============================================".blue()
    );
    eprintln!("{}", "Scenario 1: First Run - Creating Cache".green());
    eprintln!(
        "{}",
        "===============================================".blue()
    );
    // Run Timelord for the first time
    let args = Args {
        source_dir: Utf8PathBuf::from_path_buf(source_dir.clone()).unwrap(),
        cache_dir: Utf8PathBuf::from_path_buf(cache_dir.clone()).unwrap(),
    };
    main_with_args(args.clone());

    // Check if the database was created
    let cache_file = cache_dir.join("timelord.db");
    assert!(cache_file.exists(), "Database file was not created");
    eprintln!(
        "Cache file created successfully: {}",
        cache_file.display().green()
    );

    eprintln!(
        "\n{}",
        "===============================================".blue()
    );
    eprintln!("{}", "Scenario 2: Modifying Timestamps".green());
    eprintln!(
        "{}",
        "===============================================".blue()
    );
    // Change all timestamps
    let new_time = SystemTime::now() - std::time::Duration::from_secs(3600);
    File::open(&file1_path)
        .unwrap()
        .set_modified(new_time)
        .unwrap();
    File::open(&file2_path)
        .unwrap()
        .set_modified(new_time)
        .unwrap();
    eprintln!(
        "{}",
        "Modified timestamps of both files to 1 hour ago".yellow()
    );

    // Run Timelord again
    eprintln!("{}", "Running Timelord to restore timestamps...".cyan());
    main_with_args(args.clone());

    // Check if timestamps were restored
    let file1_time = fs::metadata(&file1_path).unwrap().modified().unwrap();
    let file2_time = fs::metadata(&file2_path).unwrap().modified().unwrap();
    assert!(file1_time != new_time, "File1 timestamp was not restored");
    assert!(file2_time != new_time, "File2 timestamp was not restored");
    eprintln!(
        "{}",
        "Timestamps successfully restored for both files".green()
    );

    eprintln!(
        "\n{}",
        "===============================================".blue()
    );
    eprintln!("{}", "Scenario 3: Modifying Content and Timestamps".green());
    eprintln!(
        "{}",
        "===============================================".blue()
    );
    // Change timestamps again and modify one file
    let another_new_time = SystemTime::now() - std::time::Duration::from_secs(7200);
    File::open(&file1_path)
        .unwrap()
        .set_modified(another_new_time)
        .unwrap();
    let mut file2 = File::create(&file2_path).unwrap();
    file2.write_all(b"Modified content").unwrap();
    file2.set_modified(another_new_time).unwrap();
    eprintln!(
        "{}",
        "Modified timestamps of both files to 2 hours ago".yellow()
    );
    eprintln!(
        "{}",
        "Changed content of file2 to 'Modified content'".yellow()
    );

    // Run Timelord one more time
    eprintln!(
        "{}",
        "Running Timelord to selectively restore timestamps...".cyan()
    );
    main_with_args(args);

    // Check if timestamps were restored correctly
    let file1_final_time = fs::metadata(&file1_path).unwrap().modified().unwrap();
    let file2_final_time = fs::metadata(&file2_path).unwrap().modified().unwrap();
    assert!(
        file1_final_time != another_new_time,
        "File1 timestamp was not restored after content remained unchanged"
    );
    assert!(
        file2_final_time == another_new_time,
        "File2 timestamp was correctly not restored after content change"
    );
    eprintln!("{}", "File1 timestamp restored (content unchanged)".green());
    eprintln!(
        "{}",
        "File2 timestamp not restored (content changed)".green()
    );

    eprintln!(
        "\n{}",
        "===============================================".blue()
    );
    eprintln!("{}", "Self-test completed successfully!".green());
    eprintln!(
        "{}",
        "===============================================".blue()
    );
}
