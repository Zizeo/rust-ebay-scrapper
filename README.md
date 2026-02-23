# Rust eBay Scrapper / Listing Saver

A high-performance, multithreaded eBay listing scraper built with Rust, featuring a user-friendly GUI and optimized browser automation.
This project is aimed at ebay store owners who want to save all of their listings. The best way is to generate a store report, where all listings Ids are listed.

## AI Notice

This project was generated with the help of AI.
The core logic was manually written in python and then translated to rust.
The GUI and multithreading logic was generated with the help of AI.

## ✨ Features

- **Integrated GUI**: Easy-to-use interface built with `eframe` and `egui`.
- **High Concurrency**: Multithreaded processing using `rayon` for parallelizing listing fetches and image downloads.
- **Stealth Scaping**: Optimized `headless_chrome` configuration with custom User-Agents and automated stealth property injections to minimize detection.
- **Resource Optimization**: Blocks unnecessary browser resources (like images during page load) to increase speed and reduce bandwidth.
- **Robustness**: Automated browser instance recovery and detailed error logging.
- **Persistent Data**: Saves listing metadata in `data.json` and downloads high-resolution images.

## 🛠️ Tech Stack

- **[Rust](https://www.rust-lang.org/)**: The core language for memory safety and performance.
- **[headless_chrome](https://github.com/rust-browser/headless-chrome)**: High-level API to control Chrome/Chromium.
- **[eframe](https://github.com/emilk/egui/tree/master/crates/eframe)**: Framework for creating GUI applications.
- **[reqwest](https://github.com/seanmonstar/reqwest)**: HTTP client for fast internal description fetching and image downloads.
- **[rayon](https://github.com/rayon-rs/rayon)**: Data-parallelism library for multithreaded performance.
- **[scraper](https://github.com/causal-agent/scraper)**: HTML parsing and CSS selectors.

## 🚀 Getting Started

### Prerequisites

- [Rust and Cargo](https://rustup.rs/) (latest stable version recommended).
- Chrome or Chromium browser installed.

### Installation
Download the release or :

1. Clone the repository:
   ```bash
   git clone https://github.com/Zizeo/rust-ebay-scrapper.git
   cd rust-ebay-scrapper
   ```
2. Build and run:
   ```bash
   cargo run --release
   ```

## 📖 Usage

1. **Items File**: Create a `.txt` file containing eBay item IDs (one per line).
2. **Launch**: Open the application.
3. **Select File**: Use the "Select File" button to choose your item IDs list.
4. **Select Folder**: Choose a destination folder for the downloads.
5. **Configure**: Adjust the "Concurrent Tabs" slider based on your system's performance.
6. **Start**: Click "Start Download".

## 📁 Output Structure

Each listing is saved in its own folder named after the eBay Item ID:
```text
download/
└── 123456789012/
    ├── data.json     # Contains title, price, description, etc.
    ├── 1.jpg         # First listing image
    ├── 2.jpg         # Second listing image
    └── ...
```
