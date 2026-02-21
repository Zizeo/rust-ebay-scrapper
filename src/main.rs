mod ebay_getter;

use ebay_getter as eg;
use headless_chrome::Browser;
use headless_chrome::LaunchOptions;
use std::collections::HashSet;
use std::fs;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

// --- Configuration ---
const ITEM_FILE: &str = "all_item.txt";
const DOWNLOAD_DIR: &str = "./download";
const EBAY_URL_TEMPLATE: &str = "https://www.ebay.fr/itm/";

/// Load item IDs from file, one per line.
/// Strips whitespace and ignores empty lines.
fn load_item_ids(filepath: &str) -> Vec<String> {
    let content = fs::read_to_string(filepath).expect("Failed to read item file");
    content
        .lines()
        .map(|line| line.trim().to_string())
        .filter(|line| !line.is_empty())
        .collect()
}

/// Return set of item IDs that already have folders in download directory.
/// This enables restart-safe behavior - we skip items already processed.
fn get_processed_ids(download_dir: &str) -> HashSet<String> {
    let path = std::path::Path::new(download_dir);
    if !path.exists() {
        return HashSet::new();
    }
    match fs::read_dir(path) {
        Ok(entries) => entries
            .filter_map(|entry| entry.ok().and_then(|e| e.file_name().into_string().ok()))
            .collect(),
        Err(_) => HashSet::new(),
    }
}

/// Build eBay item URL from ID.
fn build_url(item_id: &str) -> String {
    format!("{}{}", EBAY_URL_TEMPLATE, item_id)
}

fn main() {
    // Setup headless Chrome
    let launch_options = LaunchOptions {
        headless: true,
        ..LaunchOptions::default()
    };

    let browser = Browser::new(launch_options).expect("Failed to launch browser");

    // Create download directory if it doesn't exist
    fs::create_dir_all(DOWNLOAD_DIR).expect("Failed to create download directory");

    // Load all item IDs and find pending (unprocessed) ones
    let all_ids = load_item_ids(ITEM_FILE);
    let processed_ids = get_processed_ids(DOWNLOAD_DIR);
    let pending_ids: Vec<&String> = all_ids
        .iter()
        .filter(|id| !processed_ids.contains(id.as_str()))
        .collect();

    println!("{}", "=".repeat(50));
    println!("eBay Item Scraper - Restart Safe");
    println!("{}", "=".repeat(50));
    println!("Total items in file: {}", all_ids.len());
    println!("Already processed:   {}", processed_ids.len());
    println!("Pending to process:  {}", pending_ids.len());
    println!("{}", "=".repeat(50));

    if pending_ids.is_empty() {
        println!("All items have been processed. Nothing to do.");
        return;
    }

    let start_time = Instant::now();
    let mut success_count = 0u32;
    let mut fail_count = 0u32;

    // Setup Ctrl+C handler
    let interrupted = Arc::new(AtomicBool::new(false));
    let interrupted_clone = interrupted.clone();
    ctrlc::set_handler(move || {
        interrupted_clone.store(true, Ordering::SeqCst);
    })
    .expect("Error setting Ctrl-C handler");

    let total = pending_ids.len();
    for (idx, item_id) in pending_ids.iter().enumerate() {
        // Check for Ctrl+C
        if interrupted.load(Ordering::SeqCst) {
            println!("\n\n--- Interrupted by user ---");
            println!("Progress saved. Run again to resume from item {}.", idx + 1);
            break;
        }

        let url = build_url(item_id);
        println!("\n[{}/{}] Processing item {}...", idx + 1, total, item_id);

        // Fetch item data with retry logic built into get_ebay_data
        let item_data = eg::get_ebay_data(&browser, &url);

        match item_data {
            Some(ref item) => {
                let success = eg::write_item_data(item, item_id);
                if success {
                    println!("  \u{2713} Saved to download/{}/", item_id);
                    success_count += 1;
                } else {
                    println!("  \u{2717} Failed to save data");
                    fail_count += 1;
                }
            }
            None => {
                println!("  \u{2717} Failed to fetch item data");
                fail_count += 1;
            }
        }

        // Rate limiting to avoid being blocked
        thread::sleep(Duration::from_millis(10));
    }

    // Session summary
    let elapsed = start_time.elapsed().as_secs_f64();
    println!("\n{}", "=".repeat(50));
    println!("Session Summary");
    println!("{}", "=".repeat(50));
    println!("Time elapsed: {:.2} seconds", elapsed);
    println!("Successful:   {}", success_count);
    println!("Failed:       {}", fail_count);
    println!("{}", "=".repeat(50));
}
