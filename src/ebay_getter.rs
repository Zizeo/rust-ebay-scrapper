#![allow(dead_code)]

use headless_chrome::Browser;
use rand::Rng;
use reqwest::blocking::Client;
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::thread;
use std::time::Duration;

use rayon::prelude::*;

/// Struct matching the Python dict: title, description, image_link, commentaire, price
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct EbayItem {
    pub title: String,
    pub description: String,
    pub image_link: Vec<String>,
    pub commentaire: String,
    pub price: String,
}

/// Remove ASCII punctuation from a string
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
pub fn get_ebay_data(client: &Client, browser: &Browser, url: &str) -> Result<EbayItem, String> {
    let max_retries = 3;

    for attempt in 0..max_retries {
        // Add random jitter between 500ms and 1500ms to mimic human-like timing
        let jitter = rand::thread_rng().gen_range(500..1500);
        thread::sleep(Duration::from_millis(jitter));

        match _try_get_ebay_data(client, browser, url) {
            Ok(item) => return Ok(item),
            Err(RetryOrFail::Retry(msg)) => {
                if msg.contains("connection is closed") {
                    return Err(msg);
                }
                println!("Retry {}/{}: {}", attempt + 1, max_retries, msg);
                thread::sleep(Duration::from_secs(1));
                continue;
            }
            Err(RetryOrFail::Fail(msg)) => {
                if msg.contains("connection is closed") {
                    return Err(msg);
                }
                println!("Error: {} | Attempt {}/{}", msg, attempt + 1, max_retries);
                return Err(msg);
            }
        }
    }
    Err("Max retries reached".to_string())
}

enum RetryOrFail {
    Retry(String),
    Fail(String),
}

fn _try_get_ebay_data(
    client: &Client,
    browser: &Browser,
    url: &str,
) -> Result<EbayItem, RetryOrFail> {
    let tab = browser
        .new_tab()
        .map_err(|e| RetryOrFail::Fail(format!("Failed to create tab: {}", e)))?;

    // Enable stealth: Disable webdriver flag
    let _ = tab.evaluate(
        "Object.defineProperty(navigator, 'webdriver', {get: () => undefined})",
        false,
    );

    tab.navigate_to(url)
        .map_err(|e| RetryOrFail::Fail(format!("Failed to navigate: {}", e)))?;

    tab.wait_until_navigated()
        .map_err(|e| RetryOrFail::Fail(format!("Navigation timeout: {}", e)))?;

    // Wait for the title instead of a fixed sleep
    tab.wait_for_element("h1.x-item-title__mainTitle")
        .map_err(|_| RetryOrFail::Retry("Title element not found yet".to_string()))?;

    let page_source = tab
        .get_content()
        .map_err(|e| RetryOrFail::Fail(format!("Failed to get content: {}", e)))?;

    let soup = Html::parse_document(&page_source);

    let title = {
        let sel = Selector::parse("h1.x-item-title__mainTitle").unwrap();
        match soup.select(&sel).next() {
            Some(el) => remove_punctuation(&el.text().collect::<String>()),
            None => "None".to_string(),
        }
    };

    let iframe_sel = Selector::parse("iframe#desc_ifr").unwrap();
    let iframe_el = soup
        .select(&iframe_sel)
        .next()
        .ok_or_else(|| RetryOrFail::Retry("Description iframe not found.".to_string()))?;

    let url_dsc = iframe_el
        .value()
        .attr("src")
        .ok_or_else(|| RetryOrFail::Fail("Iframe has no src".to_string()))?;

    // Description from iframe src (fetched via reqwest for speed)
    let description = {
        client
            .get(url_dsc)
            .send()
            .ok()
            .and_then(|r| r.text().ok())
            .map(|text| {
                let soup_dsc = Html::parse_document(&text);

                let mut raw_text = String::new();
                let mut forbidden_depth = 0;

                // Traverse the tree to collect text while skipping <script> and <style> content
                for edge in soup_dsc.tree.root().traverse() {
                    match edge {
                        ego_tree::iter::Edge::Open(node) => {
                            if let Some(el) = node.value().as_element() {
                                if &*el.name.local == "script" || &*el.name.local == "style" {
                                    forbidden_depth += 1;
                                }
                            } else if let Some(t) = node.value().as_text() {
                                if forbidden_depth == 0 {
                                    raw_text.push_str(t);
                                    raw_text.push(' ');
                                }
                            }
                        }
                        ego_tree::iter::Edge::Close(node) => {
                            if let Some(el) = node.value().as_element() {
                                if &*el.name.local == "script" || &*el.name.local == "style" {
                                    forbidden_depth -= 1;
                                }
                            }
                        }
                    }
                }

                // Split by whitespace, skip the first word, and join back
                raw_text
                    .split_whitespace()
                    .skip(1)
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .unwrap_or_default()
    };

    let image_link = {
        let container_sel = Selector::parse("div.ux-image-carousel-container img").unwrap();
        let mut unique_links = std::collections::HashSet::new();
        soup.select(&container_sel)
            .filter_map(|img| {
                let el = img.value();
                el.attr("data-zoom-src")
                    .or_else(|| el.attr("data-src"))
                    .or_else(|| el.attr("src"))
                    .map(|s| s.to_string())
            })
            .filter(|link| unique_links.insert(link.clone()))
            .collect()
    };

    let price = {
        let sel = Selector::parse("div.x-price-primary").unwrap();
        soup.select(&sel)
            .next()
            .map(|el| el.text().collect::<String>().trim().to_string())
            .unwrap_or_else(|| "None".to_string())
    };

    let commentaire = {
        let sel = Selector::parse("#viTabs_0_is .ux-layout-section-evo").unwrap();
        soup.select(&sel)
            .next()
            .map(|el| el.text().collect::<String>().trim().to_string())
            .unwrap_or_default()
    };

    let _ = tab.close(true);

    Ok(EbayItem {
        title,
        description,
        image_link,
        commentaire,
        price,
    })
}

/// Sanitizes an image URL.
fn sanitize_image_url(link: &str) -> Option<String> {
    let target = link.trim();
    if target.is_empty() {
        return None;
    }
    if target.starts_with("//") {
        Some(format!("https:{}", target))
    } else if target.starts_with("http") {
        Some(target.to_string())
    } else {
        None
    }
}

/// Internal helper for writing item data and downloading images in parallel.
fn _write_item_data_internal(client: &Client, item: &EbayItem, folder_path: &str) -> bool {
    if let Err(e) = fs::create_dir_all(folder_path) {
        println!("Error creating dir {}: {}", folder_path, e);
        return false;
    }

    let json_path = format!("{}/data.json", folder_path);
    if let Ok(mut f) = fs::File::create(&json_path) {
        let json_str = serde_json::to_string_pretty(item).unwrap_or_default();
        let _ = f.write_all(json_str.as_bytes());
    }

    // Parallel image downloads within the item
    item.image_link
        .par_iter()
        .enumerate()
        .for_each(|(i, link)| {
            let idx = i + 1;
            if let Some(target_link) = sanitize_image_url(link) {
                if let Ok(resp) = client.get(&target_link).send() {
                    if resp.status().is_success() {
                        let ext = detect_extension(&target_link);
                        let filepath = format!("{}/{}{}", folder_path, idx, ext);
                        if let Ok(bytes) = resp.bytes() {
                            let _ = fs::write(&filepath, &bytes);
                        }
                    }
                }
            }
        });

    true
}

pub fn write_item_data_to_path(
    client: &Client,
    item: &EbayItem,
    item_id: &str,
    base_path: &str,
) -> bool {
    let folder_path = format!("{}/{}", base_path, item_id);
    _write_item_data_internal(client, item, &folder_path)
}
