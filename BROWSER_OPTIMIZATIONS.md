# Revised Browser-Based Optimization Plan

This plan focuses on maximizing speed while strictly adhering to the requirement of using **Headless Chrome** and avoiding bot detection.

## 1. Browser Instance Persistence (The "Singleton" Pattern)
The current code (before my last change) was creating a browser manager but we can optimize this further.
- **The Strategy:** Use a single persistent `Browser` instance for the entire session. Starting and stopping the Chrome process is the single most expensive operation.

## 2. Advanced Tab Pooling
- **The Strategy:** Instead of creating and closing a tab for every item, maintain a pool of warm tabs.
- **Benefit:** Navigating an existing tab is significantly faster than the handshake and initialization of a new one.

## 3. Resource Interception & Blocking
- **The Strategy:** Use the Chrome DevTools Protocol (CDP) to block non-essential resources *within the browser*.
- **Implementation:**
    - Block `fonts`, `media`, and `stylesheets`.
    - **Crucially:** Block `images` from loading in the browser to save bandwidth and CPU, but still extract the `src` URL from the HTML to download them separately via `reqwest`.

## 4. Intelligent Concurrency (The "Security Sweet Spot")
- **The Strategy:** Implement a hard cap on concurrent tabs (e.g., 2-4).
- **Rationale:** High concurrency from a single IP is the most common trigger for security measures (CAPTCHAs). By keeping concurrency low but the browser extremely efficient, we maintain a steady, safe, and fast ingestion rate.

## 5. Stealth & Fingerprinting
- **The Strategy:** 
    - Use a fixed, realistic `User-Agent`.
    - Set the `window.navigator.webdriver` property to `false` via a startup script to evade basic bot detection.
    - Implement randomized "human-like" delays (jitter) between navigations.

## 6. Hybrid Extraction
- **The Strategy:** Use `headless_chrome` for the initial page load and to execute any required JS, then immediately extract the `DOM` and pass it to the fast `scraper` crate for parsing. This offloads CPU work from the browser process to the Rust process.
