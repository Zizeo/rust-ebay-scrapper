mod ebay_getter;

use ebay_getter as eg;
use eframe::egui;
use headless_chrome::{Browser, LaunchOptions};
use rayon::prelude::*;
use reqwest::blocking::Client;
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::thread;
use std::time::Duration;

// --- Configuration Defaults ---
const EBAY_URL_TEMPLATE: &str = "https://www.ebay.fr/itm/";

struct BrowserManager {
    browser: Arc<RwLock<Arc<Browser>>>,
    launch_options: LaunchOptions<'static>,
}

impl BrowserManager {
    fn new(options: LaunchOptions<'static>) -> Result<Self, String> {
        let browser = Browser::new(options.clone())
            .map_err(|e| format!("Failed to launch browser: {}", e))?;
        Ok(Self {
            browser: Arc::new(RwLock::new(Arc::new(browser))),
            launch_options: options,
        })
    }

    fn get_browser(&self) -> Arc<Browser> {
        self.browser.read().unwrap().clone()
    }

    fn restart(&self) -> Result<(), String> {
        let mut lock = self.browser.write().unwrap();
        if lock.new_tab().is_ok() {
            return Ok(());
        }

        let new_browser = Browser::new(self.launch_options.clone())
            .map_err(|e| format!("Failed to restart browser: {}", e))?;
        *lock = Arc::new(new_browser);
        Ok(())
    }
}

struct ScraperApp {
    item_file: Option<PathBuf>,
    download_dir: Option<PathBuf>,
    is_running: Arc<AtomicBool>,
    progress: Arc<AtomicU32>,
    total_items: Arc<AtomicU32>,
    success_count: Arc<AtomicU32>,
    fail_count: Arc<AtomicU32>,
    status_msg: String,
    num_threads: usize,
    max_threads: usize,
    show_logs: bool,
    logs: Arc<Mutex<Vec<String>>>,
}

impl ScraperApp {
    fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        let max_threads = thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1);

        Self {
            item_file: None,
            download_dir: None,
            is_running: Arc::new(AtomicBool::new(false)),
            progress: Arc::new(AtomicU32::new(0)),
            total_items: Arc::new(AtomicU32::new(0)),
            success_count: Arc::new(AtomicU32::new(0)),
            fail_count: Arc::new(AtomicU32::new(0)),
            status_msg: "Ready".to_string(),
            num_threads: (max_threads / 2).max(1).min(4), // Default to low concurrency for browser safety
            max_threads: (max_threads).max(4),
            show_logs: false,
            logs: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn add_log(logs: &Arc<Mutex<Vec<String>>>, msg: String) {
        if let Ok(mut logs) = logs.lock() {
            logs.push(msg);
            if logs.len() > 1000 {
                logs.remove(0);
            }
        }
    }

    fn start_download(&mut self) {
        let item_file = match &self.item_file {
            Some(f) => f.clone(),
            None => {
                self.status_msg = "Error: Please select an items file.".to_string();
                return;
            }
        };

        let download_dir = match &self.download_dir {
            Some(d) => d.clone(),
            None => {
                self.status_msg = "Error: Please select a download folder.".to_string();
                return;
            }
        };

        let thread_count = self.num_threads;

        self.is_running.store(true, Ordering::SeqCst);
        self.progress.store(0, Ordering::SeqCst);
        self.success_count.store(0, Ordering::SeqCst);
        self.fail_count.store(0, Ordering::SeqCst);
        self.status_msg = "Initializing...".to_string();

        if let Ok(mut l) = self.logs.lock() {
            l.clear();
        }

        let is_running = self.is_running.clone();
        let progress = self.progress.clone();
        let total_items_atomic = self.total_items.clone();
        let success_count = self.success_count.clone();
        let fail_count = self.fail_count.clone();
        let logs_clone = self.logs.clone();

        thread::spawn(move || {
            Self::add_log(&logs_clone, "Starting Chrome...".to_string());

            let client = Arc::new(
                Client::builder()
                    .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/121.0.0.0 Safari/537.36")
                    .pool_idle_timeout(Duration::from_secs(90))
                    .build()
                    .unwrap_or_else(|_| Client::new()),
            );

            // Setup headless Chrome with resource blocking and increased timeouts
            let launch_options = LaunchOptions {
                headless: true,
                idle_browser_timeout: Duration::from_secs(120),
                args: vec![
                    std::ffi::OsStr::new("--blink-settings=imagesEnabled=false"), // Browser-level image blocking
                    std::ffi::OsStr::new("--disable-extensions"),
                    std::ffi::OsStr::new("--disable-gpu"),
                    std::ffi::OsStr::new("--disable-dev-shm-usage"),
                    std::ffi::OsStr::new("--no-sandbox"),
                    std::ffi::OsStr::new("--disable-blink-features=AutomationControlled"), // Stealth
                    std::ffi::OsStr::new("--user-agent=Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/121.0.0.0 Safari/537.36"),
                ],
                ..LaunchOptions::default()
            };

            let browser_manager = match BrowserManager::new(launch_options) {
                Ok(bm) => bm,
                Err(e) => {
                    Self::add_log(&logs_clone, format!("Error: {}", e));
                    is_running.store(false, Ordering::SeqCst);
                    return;
                }
            };

            let mut all_ids = load_item_ids(item_file.to_str().unwrap_or(""));
            let mut seen_ids = HashSet::new();
            all_ids.retain(|id| seen_ids.insert(id.clone()));

            let download_dir_str = download_dir.to_str().unwrap_or("./download");

            if let Err(_) = fs::create_dir_all(download_dir_str) {}

            let processed_ids = get_processed_ids(download_dir_str);
            let pending_ids: Vec<String> = all_ids
                .into_iter()
                .filter(|id| !processed_ids.contains(id.as_str()))
                .collect();

            let total = pending_ids.len();
            total_items_atomic.store(total as u32, Ordering::SeqCst);
            Self::add_log(&logs_clone, format!("Items to process: {}", total));

            if total == 0 {
                is_running.store(false, Ordering::SeqCst);
                return;
            }

            let pool = rayon::ThreadPoolBuilder::new()
                .num_threads(thread_count)
                .build()
                .unwrap_or_else(|_| rayon::ThreadPoolBuilder::new().build().unwrap());

            Self::add_log(
                &logs_clone,
                format!("Using pool with {} browser threads", thread_count),
            );

            pool.install(|| {
                pending_ids.par_iter().for_each(|item_id| {
                    if !is_running.load(Ordering::SeqCst) {
                        return;
                    }

                    Self::add_log(&logs_clone, format!("Processing item {}", item_id));
                    let url = format!("{}{}", EBAY_URL_TEMPLATE, item_id);

                    let mut attempt = 0;
                    let max_retry_browser = 3;

                    while attempt < max_retry_browser {
                        let browser = browser_manager.get_browser();
                        match eg::get_ebay_data(&client, &browser, &url) {
                            Ok(item) => {
                                let success = eg::write_item_data_to_path(
                                    &client,
                                    &item,
                                    item_id,
                                    download_dir_str,
                                );
                                if success {
                                    success_count.fetch_add(1, Ordering::SeqCst);
                                    Self::add_log(&logs_clone, format!("✓ Saved {}", item_id));
                                } else {
                                    fail_count.fetch_add(1, Ordering::SeqCst);
                                    Self::add_log(
                                        &logs_clone,
                                        format!("✗ Write failed for {}", item_id),
                                    );
                                }
                                break;
                            }
                            Err(e) => {
                                if e.contains("connection is closed")
                                    || e.contains("underlying connection is closed")
                                {
                                    Self::add_log(
                                        &logs_clone,
                                        format!("! Browser dead for {}, restarting...", item_id),
                                    );
                                    if let Err(restart_err) = browser_manager.restart() {
                                        Self::add_log(
                                            &logs_clone,
                                            format!("!! Restart failed: {}", restart_err),
                                        );
                                        fail_count.fetch_add(1, Ordering::SeqCst);
                                        break;
                                    }
                                    attempt += 1;
                                    continue;
                                } else {
                                    fail_count.fetch_add(1, Ordering::SeqCst);
                                    Self::add_log(
                                        &logs_clone,
                                        format!("✗ Fetch failed for {}: {}", item_id, e),
                                    );
                                    break;
                                }
                            }
                        }
                    }
                    progress.fetch_add(1, Ordering::SeqCst);
                });
            });

            Self::add_log(&logs_clone, "Session finished.".to_string());
            is_running.store(false, Ordering::SeqCst);
        });
    }
}

impl eframe::App for ScraperApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("eBay Scraper (Optimized Browser)");
            ui.add_space(10.0);

            // File Picker
            ui.horizontal(|ui| {
                ui.label("Items File (TXT):");
                if ui.button("Select File").clicked() {
                    if let Some(path) = rfd::FileDialog::new()
                        .add_filter("Text Files", &["txt"])
                        .pick_file()
                    {
                        self.item_file = Some(path);
                    }
                }
            });
            if let Some(path) = &self.item_file {
                let display_text = if path.to_string_lossy().len() > 50 {
                    format!(
                        "...{}",
                        &path.to_string_lossy()[path.to_string_lossy().len() - 47..]
                    )
                } else {
                    path.to_string_lossy().to_string()
                };
                ui.label(format!("Selected: {}", display_text));
            }
            ui.add_space(5.0);

            // Folder Picker
            ui.horizontal(|ui| {
                ui.label("Download Folder:");
                if ui.button("Select Folder").clicked() {
                    if let Some(path) = rfd::FileDialog::new().pick_folder() {
                        self.download_dir = Some(path);
                    }
                }
            });
            if let Some(path) = &self.download_dir {
                let display_text = if path.to_string_lossy().len() > 50 {
                    format!(
                        "...{}",
                        &path.to_string_lossy()[path.to_string_lossy().len() - 47..]
                    )
                } else {
                    path.to_string_lossy().to_string()
                };
                ui.label(format!("Selected: {}", display_text));
            }
            ui.add_space(10.0);

            // Thread Slider
            ui.horizontal(|ui| {
                ui.label("Concurrent Tabs:");
                ui.add(egui::Slider::new(
                    &mut self.num_threads,
                    1..=self.max_threads,
                ));
            });
            ui.add_space(10.0);

            // Logs toggle
            ui.checkbox(&mut self.show_logs, "Show Integrated Logs");
            ui.add_space(15.0);

            // Start Button
            let is_running = self.is_running.load(Ordering::SeqCst);
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(!is_running, egui::Button::new("Start Download"))
                    .clicked()
                {
                    self.start_download();
                }

                if ui
                    .add_enabled(is_running, egui::Button::new("Stop"))
                    .clicked()
                {
                    self.is_running.store(false, Ordering::SeqCst);
                }
            });

            ui.add_space(20.0);

            // Progress Bar
            let total = self.total_items.load(Ordering::SeqCst);
            let current = self.progress.load(Ordering::SeqCst);
            let progress_val = if total > 0 {
                current as f32 / total as f32
            } else {
                0.0
            };

            ui.add(egui::ProgressBar::new(progress_val).text(format!("{}/{}", current, total)));

            ui.add_space(10.0);
            ui.label(format!(
                "Success: {} | Failed: {}",
                self.success_count.load(Ordering::SeqCst),
                self.fail_count.load(Ordering::SeqCst)
            ));

            if !is_running && current > 0 && current == total {
                self.status_msg = "Completed!".to_string();
            } else if is_running {
                self.status_msg = "Running...".to_string();
            }

            ui.add_space(10.0);
            ui.label(format!("Status: {}", self.status_msg));

            // Log Area
            if self.show_logs {
                ui.add_space(10.0);
                ui.separator();
                ui.heading("Logs");
                egui::ScrollArea::vertical()
                    .max_height(200.0)
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        if let Ok(logs) = self.logs.lock() {
                            for log in logs.iter() {
                                ui.label(log);
                            }
                        }
                    });
            }
        });

        // Continuous update loop
        ctx.request_repaint();
    }
}

/// Load item IDs from file, one per line.
fn load_item_ids(filepath: &str) -> Vec<String> {
    let content = match fs::read_to_string(filepath) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    content
        .lines()
        .map(|line| line.trim().to_string())
        .filter(|line| !line.is_empty())
        .collect()
}

/// Return set of item IDs that already have folders in download directory.
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

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([450.0, 600.0]),
        ..Default::default()
    };

    eframe::run_native(
        "eBay Scraper",
        options,
        Box::new(|cc| Ok(Box::new(ScraperApp::new(cc)))),
    )
}
