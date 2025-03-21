use camino::Utf8PathBuf;
use owo_colors::OwoColorize;

use super::{Args, Command, main_with_args};

#[test]
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
    main_with_args(Args {
        command: Command::Sync {
            source_dir: Utf8PathBuf::from_path_buf(source_dir.clone()).unwrap(),
            cache_dir: Utf8PathBuf::from_path_buf(cache_dir.clone()).unwrap(),
        },
    });

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
    main_with_args(Args {
        command: Command::Sync {
            source_dir: Utf8PathBuf::from_path_buf(source_dir.clone()).unwrap(),
            cache_dir: Utf8PathBuf::from_path_buf(cache_dir.clone()).unwrap(),
        },
    });

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
    main_with_args(Args {
        command: Command::Sync {
            source_dir: Utf8PathBuf::from_path_buf(source_dir.clone()).unwrap(),
            cache_dir: Utf8PathBuf::from_path_buf(cache_dir.clone()).unwrap(),
        },
    });

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
    eprintln!("{}", "Scenario 4: Different Source Directory Base".green());
    eprintln!(
        "{}",
        "===============================================".blue()
    );
    // Create a new source directory with the same structure
    let new_source_dir = temp_dir.path().join("new_source");
    fs::create_dir_all(&new_source_dir).unwrap();
    let new_file1_path = new_source_dir.join("file1.txt");
    let new_file2_path = new_source_dir.join("file2.txt");
    fs::copy(&file1_path, &new_file1_path).unwrap();
    fs::copy(&file2_path, &new_file2_path).unwrap();

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

    eprintln!(
        "{}",
        "Created new source directory with same files".yellow()
    );
    eprintln!("  New Source: {}", new_source_dir.display().blue());

    // Run Timelord with the new source directory
    eprintln!("{}", "Running Timelord with new source directory...".cyan());
    main_with_args(Args {
        command: Command::Sync {
            source_dir: Utf8PathBuf::from_path_buf(new_source_dir).unwrap(),
            cache_dir: Utf8PathBuf::from_path_buf(cache_dir.clone()).unwrap(),
        },
    });

    // Check if timestamps were restored in the new location
    let new_file1_time = fs::metadata(&new_file1_path).unwrap().modified().unwrap();
    let new_file2_time = fs::metadata(&new_file2_path).unwrap().modified().unwrap();
    assert!(
        new_file1_time != new_time,
        "New file1 timestamp was not restored"
    );
    assert!(
        new_file2_time != new_time,
        "New file2 timestamp was not restored"
    );
    eprintln!(
        "{}",
        "Timestamps successfully restored in new location".green()
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
