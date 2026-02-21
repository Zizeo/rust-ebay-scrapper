#![allow(dead_code)]

use headless_chrome::Browser;
use reqwest::blocking::Client;
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::Path;
use std::thread;
use std::time::Duration;

use chrono::Utc;
use rand::Rng;

/// Struct matching the Python dict: title, description, image_link, commentaire, price
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct EbayItem {
    pub title: String,
    pub description: String,
    pub image_link: Vec<String>,
    pub commentaire: String,
    pub price: String,
}

/// Remove ASCII punctuation from a string (mirrors Python's `str.translate(table)` with `string.punctuation`).
fn remove_punctuation(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_ascii_punctuation())
        .collect::<String>()
        .trim()
        .to_string()
}

/// Detect image extension from URL, default to ".jpg".
fn detect_extension(link: &str) -> &str {
    let lower = link.to_lowercase();
    if lower.contains(".webp") {
        ".webp"
    } else if lower.contains(".png") {
        ".png"
    } else if lower.contains(".jpeg") {
        ".jpeg"
    } else {
        ".jpg"
    }
}

/// Equivalent of `get_ebay_data(driver, url)` in Python.
///
/// Uses headless_chrome to navigate to the URL, gets the page source,
/// then parses with `scraper`. Retries up to 3 times if the description iframe is missing.
pub fn get_ebay_data(browser: &Browser, url: &str) -> Option<EbayItem> {
    let max_retries = 5;

    for attempt in 0..max_retries {
        match _try_get_ebay_data(browser, url) {
            Ok(item) => return Some(item),
            Err(RetryOrFail::Retry(msg)) => {
                println!("Retry {}/{}: {}", attempt + 1, max_retries, msg);
                thread::sleep(Duration::from_secs(2));
                continue;
            }
            Err(RetryOrFail::Fail(msg)) => {
                println!(
                    "Error: {} in get_ebay_data (attempt {}/{})",
                    msg,
                    attempt + 1,
                    max_retries
                );
                if attempt < max_retries - 1 {
                    thread::sleep(Duration::from_millis(100));
                    continue;
                }
                return None;
            }
        }
    }

    None
}

enum RetryOrFail {
    Retry(String),
    Fail(String),
}

fn _try_get_ebay_data(browser: &Browser, url: &str) -> Result<EbayItem, RetryOrFail> {
    // Navigate to the page
    let tab = browser
        .new_tab()
        .map_err(|e| RetryOrFail::Fail(format!("Failed to create tab: {}", e)))?;

    tab.navigate_to(url)
        .map_err(|e| RetryOrFail::Fail(format!("Failed to navigate: {}", e)))?;

    tab.wait_until_navigated()
        .map_err(|e| RetryOrFail::Fail(format!("Navigation timeout: {}", e)))?;

    // Small wait to let the page render
    thread::sleep(Duration::from_secs(2));

    let page_source = tab
        .get_content()
        .map_err(|e| RetryOrFail::Fail(format!("Failed to get page source: {}", e)))?;

    let soup = Html::parse_document(&page_source);

    // --- Title ---
    let title = {
        let sel = Selector::parse("h1.x-item-title__mainTitle").unwrap();
        match soup.select(&sel).next() {
            Some(el) => {
                let raw = el.text().collect::<String>().trim().to_string();
                remove_punctuation(&raw)
            }
            None => {
                println!("Error: title element not found");
                "None".to_string()
            }
        }
    };

    // --- Iframe for description ---
    let iframe_sel = Selector::parse("iframe#desc_ifr").unwrap();
    let iframe_el = soup
        .select(&iframe_sel)
        .next()
        .ok_or_else(|| RetryOrFail::Retry("Iframe not found.".to_string()))?;

    // Optionally save soup to file (only if ebay_soup.html doesn't already exist)
    if !Path::new("ebay_soup.html").exists() {
        if let Ok(mut f) = fs::File::create("ebay_soup.html") {
            let _ = f.write_all(page_source.as_bytes());
        }
    }

    let url_dsc = iframe_el
        .value()
        .attr("src")
        .ok_or_else(|| RetryOrFail::Fail("Iframe has no src attribute".to_string()))?;

    // --- Description from iframe src ---
    let description = {
        let client = Client::new();
        match client.get(url_dsc).send() {
            Ok(resp) => match resp.text() {
                Ok(text) => {
                    let soup_dsc = Html::parse_document(&text);
                    // Get all text, split into words, skip the first word, rejoin
                    let page_text: String = soup_dsc
                        .root_element()
                        .text()
                        .collect::<String>()
                        .trim()
                        .to_string();
                    let words: Vec<&str> = page_text.split_whitespace().collect();
                    if words.len() > 1 {
                        words[1..].join(" ")
                    } else {
                        String::new()
                    }
                }
                Err(e) => {
                    println!("Error: {} in description", e);
                    String::new()
                }
            },
            Err(e) => {
                println!("Error: {} in description", e);
                String::new()
            }
        }
    };

    // --- Image links ---
    let image_link = {
        let container_sel = Selector::parse("div.ux-image-carousel-container img").unwrap();
        let mut links: Vec<String> = Vec::new();

        for img in soup.select(&container_sel) {
            let el = img.value();

            // 1. Try data-zoom-src
            let mut link: Option<String> = el.attr("data-zoom-src").map(|s| s.to_string());

            // 2. Try data-srcset → take last candidate
            if link.is_none() {
                if let Some(srcset) = el.attr("data-srcset") {
                    let candidates: Vec<&str> = srcset
                        .split(',')
                        .map(|c| c.trim())
                        .filter(|c| !c.is_empty())
                        .collect();
                    if let Some(last) = candidates.last() {
                        // Each candidate is "url size", take the url part
                        if let Some(url_part) = last.split_whitespace().next() {
                            link = Some(url_part.to_string());
                        }
                    }
                }
            }

            // 3. Fallback: data-src or src
            if link.is_none() {
                link = el
                    .attr("data-src")
                    .or_else(|| el.attr("src"))
                    .map(|s| s.to_string());
            }

            if let Some(l) = link {
                links.push(l);
            }
        }

        links
    };

    // --- Price ---
    let price = {
        let sel = Selector::parse("div.x-price-primary").unwrap();
        match soup.select(&sel).next() {
            Some(el) => el.text().collect::<String>().trim().to_string(),
            None => {
                println!("Error: price element not found");
                "None".to_string()
            }
        }
    };

    // --- Commentaire (condition/features) ---
    let commentaire = {
        let sel = Selector::parse("#viTabs_0_is > div > div > div.ux-layout-section-evo.ux-layout-section--features > div > div:nth-child(1) > div:nth-child(2) > dl > dd > div > div").unwrap();
        match soup.select(&sel).next() {
            Some(el) => el.text().collect::<String>().trim().to_string(),
            None => String::new(),
        }
    };

    // Close the tab to free resources
    let _ = tab.close(true);

    Ok(EbayItem {
        title,
        description,
        image_link,
        commentaire,
        price,
    })
}

/// Equivalent of `get_item_link(driver, url)` in Python.
///
/// Parses a listing/grid page and returns all item links.
pub fn get_item_link(browser: &Browser, url: &str) -> Vec<String> {
    let tab = browser.new_tab().expect("Failed to create tab");
    tab.navigate_to(url).expect("Failed to navigate");
    tab.wait_until_navigated().expect("Navigation timeout");

    thread::sleep(Duration::from_secs(2));

    let page_source = tab.get_content().expect("Failed to get page source");
    let soup = Html::parse_document(&page_source);

    let link_sel =
        Selector::parse("section.str-items-grid__container a.str-item-card__link").unwrap();
    let mut links = Vec::new();

    for el in soup.select(&link_sel) {
        if let Some(href) = el.value().attr("href") {
            links.push(href.to_string());
        }
    }

    println!("{}", links.len());

    let _ = tab.close(true);
    links
}

/// Equivalent of `get_all_data(driver, url)` in Python.
///
/// Fetches all items from a listing page and writes each one.
pub fn get_all_data(browser: &Browser, url: &str) -> Result<Vec<Option<EbayItem>>, String> {
    let links = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        get_item_link(browser, url)
    })) {
        Ok(l) => l,
        Err(e) => return Err(format!("{:?}", e)),
    };

    let mut item_data: Vec<Option<EbayItem>> = Vec::new();

    for link in &links {
        let data = get_ebay_data(browser, link);
        if let Some(ref item) = data {
            write_data(item);
        }
        item_data.push(data);
        thread::sleep(Duration::from_millis(10));
    }

    Ok(item_data)
}

/// Equivalent of `write_data(item)` in Python.
///
/// Writes item data to `./test/{title}{random_number}/` folder.
pub fn write_data(item: &EbayItem) -> Option<i32> {
    let now = Utc::now();
    let seed_micro = now.timestamp_subsec_micros();
    let mut rng = rand::thread_rng();
    // Seed is used in Python but in Rust we just use thread_rng
    let _ = seed_micro; // mimic the Python pattern
    let nombre_aleatoire: u32 = rng.gen_range(1..=100000);

    let sanitized_title = item.title.trim().replace('"', "").replace(' ', "_");

    let folder = format!("./test/{}{}", sanitized_title, nombre_aleatoire);

    if let Err(e) = fs::create_dir_all(&folder) {
        println!("{}", item.title);
        println!("{}", e);
        return Some(404);
    }

    // Write data.json
    let json_path = format!("{}/data.json", folder);
    match fs::File::create(&json_path) {
        Ok(mut f) => {
            let json_str = serde_json::to_string_pretty(item).unwrap_or_else(|_| "{}".to_string());
            if let Err(e) = f.write_all(json_str.as_bytes()) {
                println!("Error writing JSON: {}", e);
                return None;
            }
        }
        Err(e) => {
            println!("Error creating JSON file: {}", e);
            return None;
        }
    }

    // Download images
    let client = Client::new();
    for (i, link) in item.image_link.iter().enumerate() {
        let idx = i + 1;
        match client.get(link).send() {
            Ok(resp) => {
                if resp.status().is_success() {
                    let ext = detect_extension(link);
                    let filename =
                        format!("{}{}_{}{}", sanitized_title, nombre_aleatoire, idx, ext);
                    let filepath = format!("{}/{}", folder, filename);
                    match resp.bytes() {
                        Ok(bytes) => {
                            if let Err(e) = fs::write(&filepath, &bytes) {
                                println!("Error writing image: {}", e);
                            }
                        }
                        Err(e) => println!("Error reading image bytes: {}", e),
                    }
                } else {
                    println!("La requête a échoué avec le code d'état {}", resp.status());
                }
            }
            Err(e) => println!("Error downloading image {}: {}", idx, e),
        }
    }

    Some(0)
}

/// Equivalent of `write_item_data(item, item_id)` in Python.
///
/// Writes item data to `./download/{item_id}/` folder.
/// Returns true on success, false on failure.
pub fn write_item_data(item: &EbayItem, item_id: &str) -> bool {
    let folder_path = format!("./download/{}", item_id);

    if let Err(e) = fs::create_dir_all(&folder_path) {
        println!("Error creating directory: {}", e);
        return false;
    }

    // Save JSON data
    let json_path = format!("{}/data.json", folder_path);
    match fs::File::create(&json_path) {
        Ok(mut f) => {
            let json_str = serde_json::to_string_pretty(item).unwrap_or_else(|_| "{}".to_string());
            if let Err(e) = f.write_all(json_str.as_bytes()) {
                println!("Error writing JSON: {}", e);
                return false;
            }
        }
        Err(e) => {
            println!("Error creating JSON file: {}", e);
            return false;
        }
    }

    // Download images
    let client = Client::new();
    for (i, link) in item.image_link.iter().enumerate() {
        let idx = i + 1;
        let mut target_link = link.clone();
        if target_link.starts_with("//") {
            target_link = format!("https:{}", target_link);
        }

        match client.get(&target_link).send() {
            Ok(resp) => {
                if resp.status().is_success() {
                    let ext = detect_extension(&target_link);
                    let filepath = format!("{}/{}{}", folder_path, idx, ext);
                    match resp.bytes() {
                        Ok(bytes) => {
                            if let Err(e) = fs::write(&filepath, &bytes) {
                                println!("  \u{2717} Error writing image {}: {}", idx, e);
                            }
                        }
                        Err(e) => {
                            println!("  \u{2717} Error reading bytes for image {}: {}", idx, e)
                        }
                    }
                } else {
                    println!(
                        "  \u{2717} Image {} download failed (status: {})",
                        idx,
                        resp.status()
                    );
                }
            }
            Err(e) => println!("  \u{2717} Error requesting image {}: {}", idx, e),
        }
    }

    true
}

/// Equivalent of `write_item_data(item, item_id)` but with a custom base path.
pub fn write_item_data_to_path(item: &EbayItem, item_id: &str, base_path: &str) -> bool {
    let folder_path = format!("{}/{}", base_path, item_id);

    if let Err(e) = fs::create_dir_all(&folder_path) {
        println!("Error creating directory: {}", e);
        return false;
    }

    // Save JSON data
    let json_path = format!("{}/data.json", folder_path);
    match fs::File::create(&json_path) {
        Ok(mut f) => {
            let json_str = serde_json::to_string_pretty(item).unwrap_or_else(|_| "{}".to_string());
            if let Err(e) = f.write_all(json_str.as_bytes()) {
                println!("Error writing JSON: {}", e);
                return false;
            }
        }
        Err(e) => {
            println!("Error creating JSON file: {}", e);
            return false;
        }
    }

    // Download images
    let client = Client::new();
    for (i, link) in item.image_link.iter().enumerate() {
        let idx = i + 1;
        let mut target_link = link.clone();
        if target_link.starts_with("//") {
            target_link = format!("https:{}", target_link);
        }

        match client.get(&target_link).send() {
            Ok(resp) => {
                if resp.status().is_success() {
                    let ext = detect_extension(&target_link);
                    let filepath = format!("{}/{}{}", folder_path, idx, ext);
                    match resp.bytes() {
                        Ok(bytes) => {
                            if let Err(e) = fs::write(&filepath, &bytes) {
                                println!("  \u{2717} Error writing image {}: {}", idx, e);
                            }
                        }
                        Err(e) => {
                            println!("  \u{2717} Error reading bytes for image {}: {}", idx, e)
                        }
                    }
                } else {
                    println!(
                        "  \u{2717} Image {} download failed (status: {})",
                        idx,
                        resp.status()
                    );
                }
            }
            Err(e) => println!("  \u{2717} Error requesting image {}: {}", idx, e),
        }
    }

    true
}
