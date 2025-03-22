#![doc = include_str!("../README.md")]

use camino::Utf8PathBuf;
use clap::{Parser, Subcommand};

#[derive(Parser, Debug, Clone)]
#[command(author, version, about, long_about = None)]
/// A tool to preserve file timestamps (mtime) between CI builds, even with fresh git checkouts.
struct Args {
    #[command(subcommand)]
    command: TlCommand,
}

#[derive(Subcommand, Debug, Clone)]
enum TlCommand {
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

fn main() {
    if std::env::var("RUST_LOG").is_err() {
        unsafe { std::env::set_var("RUST_LOG", "info") };
    }
    env_logger::init();

    let args = Args::parse();
    main_with_args(args);
}

fn main_with_args(args: Args) {
    match args.command {
        TlCommand::Sync {
            source_dir,
            cache_dir,
        } => {
            timelord::sync(source_dir, cache_dir);
        }
        TlCommand::CacheInfo { cache_dir } => {
            timelord::cache_info(cache_dir);
        }
    }
}
