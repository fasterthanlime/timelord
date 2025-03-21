use camino::Utf8PathBuf;
use log::{debug, info, warn};
use owo_colors::OwoColorize;

#[test]
fn self_test() {
    use std::fs::{self, File};
    use std::io::Write;
    use std::time::SystemTime;

    env_logger::builder()
        .filter_level(log::LevelFilter::Debug)
        .init();

    debug!(
        "{}",
        "===============================================".blue()
    );
    info!("Starting {}", "Timelord Self-Test".green());
    debug!(
        "{}",
        "===============================================".blue()
    );

    // Create temporary directories for source and cache
    let temp_dir = tempfile::tempdir().unwrap();
    let source_dir = temp_dir.path().join("source");
    let cache_dir = temp_dir.path().join("cache");
    fs::create_dir_all(&source_dir).unwrap();
    fs::create_dir_all(source_dir.join("src")).unwrap();
    fs::create_dir_all(source_dir.join("tests")).unwrap();
    info!("Created temporary directories: {}", "".yellow());
    debug!("  Source: {}", source_dir.display().blue());
    debug!("  Cache: {}", cache_dir.display().blue());

    // Create some test files
    let file1_path = source_dir.join("src/main.rs");
    let file2_path = source_dir.join("tests/integration-test.rs");
    let file3_path = source_dir.join("README.md");
    let mut file1 = File::create(&file1_path).unwrap();
    let mut file2 = File::create(&file2_path).unwrap();
    let mut file3 = File::create(&file3_path).unwrap();
    file1.write_all(b"Hello, World!").unwrap();
    file2.write_all(b"Timelord test").unwrap();
    file3.write_all(b"README content").unwrap();
    info!("Created test files: {}", "".yellow());
    debug!("  {}: 'Hello, World!'", file1_path.display().blue());
    debug!("  {}: 'Timelord test'", file2_path.display().blue());
    debug!("  {}: 'README content'", file3_path.display().blue());

    debug!(
        "{}",
        "===============================================".blue()
    );
    info!("Scenario 1: First Run - {}", "Creating Cache".green());
    debug!(
        "{}",
        "===============================================".blue()
    );
    // Run Timelord for the first time
    super::sync(
        Utf8PathBuf::from_path_buf(source_dir.clone()).unwrap(),
        Utf8PathBuf::from_path_buf(cache_dir.clone()).unwrap(),
    );

    // Check if the database was created
    let cache_file = cache_dir.join("timelord.db");
    assert!(cache_file.exists(), "Database file was not created");
    info!(
        "Cache file created successfully: {}",
        cache_file.display().green()
    );

    // Run cache-info command
    debug!("Running cache-info command: {}", "".cyan());
    super::cache_info(Utf8PathBuf::from_path_buf(cache_dir.clone()).unwrap());

    debug!(
        "{}",
        "===============================================".blue()
    );
    info!("Scenario 2: {}", "Modifying Timestamps".green());
    debug!(
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
    info!(
        "Modified timestamps of src/main.rs and tests/integration-test.rs to {}",
        "1 hour ago".yellow()
    );

    // Run Timelord again
    debug!("Running Timelord to restore timestamps: {}", "".cyan());
    super::sync(
        Utf8PathBuf::from_path_buf(source_dir.clone()).unwrap(),
        Utf8PathBuf::from_path_buf(cache_dir.clone()).unwrap(),
    );

    // Check if timestamps were restored
    let file1_time = fs::metadata(&file1_path).unwrap().modified().unwrap();
    let file2_time = fs::metadata(&file2_path).unwrap().modified().unwrap();
    assert!(
        file1_time != new_time,
        "src/main.rs timestamp was not restored"
    );
    assert!(
        file2_time != new_time,
        "tests/integration-test.rs timestamp was not restored"
    );
    info!(
        "Timestamps successfully restored for {}",
        "both files".green()
    );

    // Run cache-info command
    debug!("Running cache-info command: {}", "".cyan());
    super::cache_info(Utf8PathBuf::from_path_buf(cache_dir.clone()).unwrap());

    debug!(
        "{}",
        "===============================================".blue()
    );
    info!("Scenario 3: {}", "Modifying Content and Timestamps".green());
    debug!(
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
    info!(
        "Modified timestamps of src/main.rs and tests/integration-test.rs to {}",
        "2 hours ago".yellow()
    );
    info!(
        "Changed content of tests/integration-test.rs to {}",
        "'Modified content'".yellow()
    );

    // Run Timelord one more time
    debug!(
        "Running Timelord to selectively restore timestamps: {}",
        "".cyan()
    );
    super::sync(
        Utf8PathBuf::from_path_buf(source_dir.clone()).unwrap(),
        Utf8PathBuf::from_path_buf(cache_dir.clone()).unwrap(),
    );

    // Check if timestamps were restored correctly
    let file1_final_time = fs::metadata(&file1_path).unwrap().modified().unwrap();
    let file2_final_time = fs::metadata(&file2_path).unwrap().modified().unwrap();
    assert!(
        file1_final_time != another_new_time,
        "src/main.rs timestamp was not restored after content remained unchanged"
    );
    assert!(
        file2_final_time == another_new_time,
        "tests/integration-test.rs timestamp was correctly not restored after content change"
    );
    info!(
        "src/main.rs timestamp restored ({})",
        "content unchanged".green()
    );
    info!(
        "tests/integration-test.rs timestamp not restored ({})",
        "content changed".green()
    );

    // Run cache-info command
    debug!("Running cache-info command: {}", "".cyan());
    super::cache_info(Utf8PathBuf::from_path_buf(cache_dir.clone()).unwrap());

    debug!(
        "{}",
        "===============================================".blue()
    );
    info!("Scenario 4: {}", "Different Source Directory Base".green());
    debug!(
        "{}",
        "===============================================".blue()
    );
    // Create a new source directory with the same structure
    let new_source_dir = temp_dir.path().join("new_source");
    fs::create_dir_all(new_source_dir.join("src")).unwrap();
    fs::create_dir_all(new_source_dir.join("tests")).unwrap();
    let new_file1_path = new_source_dir.join("src/main.rs");
    let new_file2_path = new_source_dir.join("tests/integration-test.rs");
    let new_file3_path = new_source_dir.join("README.md");
    fs::copy(&file1_path, &new_file1_path).unwrap();
    fs::copy(&file2_path, &new_file2_path).unwrap();
    fs::copy(&file3_path, &new_file3_path).unwrap();

    // Modify timestamps of the new files
    let new_time = SystemTime::now() - std::time::Duration::from_secs(3600);
    File::open(&new_file1_path)
        .unwrap()
        .set_modified(new_time)
        .unwrap();
    File::open(&new_file2_path)
        .unwrap()
        .set_modified(new_time)
        .unwrap();

    info!(
        "Created new source directory with {}",
        "same files".yellow()
    );
    debug!("  New Source: {}", new_source_dir.display().blue());

    // Run Timelord with the new source directory
    debug!("Running Timelord with new source directory: {}", "".cyan());
    super::sync(
        Utf8PathBuf::from_path_buf(new_source_dir.clone()).unwrap(),
        Utf8PathBuf::from_path_buf(cache_dir.clone()).unwrap(),
    );

    // Check if timestamps were restored in the new location
    let new_file1_time = fs::metadata(&new_file1_path).unwrap().modified().unwrap();
    let new_file2_time = fs::metadata(&new_file2_path).unwrap().modified().unwrap();
    assert!(
        new_file1_time != new_time,
        "New src/main.rs timestamp was not restored"
    );
    assert!(
        new_file2_time != new_time,
        "New tests/integration-test.rs timestamp was not restored"
    );
    info!(
        "Timestamps successfully restored in {}",
        "new location".green()
    );

    // Run cache-info command
    debug!("Running cache-info command: {}", "".cyan());
    super::cache_info(Utf8PathBuf::from_path_buf(cache_dir.clone()).unwrap());

    debug!(
        "{}",
        "===============================================".blue()
    );
    info!("Scenario 5: {}", "Corrupted Cache File".green());
    debug!(
        "{}",
        "===============================================".blue()
    );

    // Corrupt the cache file
    let mut cache_file = File::options()
        .write(true)
        .open(&cache_file)
        .expect("Failed to open cache file");
    cache_file
        .write_all(&[0xBA, 0xDB, 0xAD, 0xFF])
        .expect("Failed to write corrupt data");
    cache_file.flush().expect("Failed to flush cache file");

    warn!("Corrupted cache file with {}", "0xBADBADFF".yellow());

    // Run Timelord with corrupted cache
    debug!("Running Timelord with corrupted cache: {}", "".cyan());
    super::sync(
        Utf8PathBuf::from_path_buf(source_dir.clone()).unwrap(),
        Utf8PathBuf::from_path_buf(cache_dir.clone()).unwrap(),
    );

    // Check if a new cache file was created
    assert!(
        cache_file.metadata().unwrap().len() > 0,
        "New cache file was not created or is empty after corruption"
    );
    info!(
        "Timelord handled corrupted cache and {}",
        "created a new one".green()
    );

    // Run cache-info command
    debug!("Running cache-info command: {}", "".cyan());
    super::cache_info(Utf8PathBuf::from_path_buf(cache_dir.clone()).unwrap());

    debug!(
        "{}",
        "===============================================".blue()
    );
    info!("All scenarios completed {}", "successfully!".green());
    debug!(
        "{}",
        "===============================================".blue()
    );
}
