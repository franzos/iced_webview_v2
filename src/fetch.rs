use std::collections::HashMap;
use std::sync::LazyLock;
use url::Url;

/// Max response size for the main page (10 MB).
const MAX_PAGE_SIZE: u64 = 10 * 1024 * 1024;
/// Max response size for a single image (10 MB).
const MAX_IMAGE_SIZE: u64 = 10 * 1024 * 1024;
/// Max response size for a single stylesheet (5 MB).
const MAX_CSS_SIZE: u64 = 5 * 1024 * 1024;
/// Max number of external stylesheets to fetch.
const MAX_STYLESHEETS: usize = 50;
/// Max depth for @import chains to prevent infinite loops.
const MAX_IMPORT_DEPTH: usize = 3;

static HTTP_CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .expect("failed to build HTTP client")
});

/// Fetch a URL and return raw HTML plus a pre-fetched CSS cache.
///
/// The CSS cache maps resolved stylesheet URLs to their CSS text.
/// The HTML is returned unmodified — no inlining. The engine's
/// `import_css` callback looks up stylesheets from the cache instead.
pub(crate) async fn fetch_html(
    page_url: String,
) -> Result<(String, HashMap<String, String>), String> {
    let client = &*HTTP_CLIENT;
    let base = Url::parse(&page_url).map_err(|e| e.to_string())?;

    let response = client
        .get(&page_url)
        .send()
        .await
        .map_err(|e| e.to_string())?;

    if let Some(len) = response.content_length() {
        if len > MAX_PAGE_SIZE {
            return Err(format!(
                "page too large: {len} bytes exceeds {MAX_PAGE_SIZE} byte limit"
            ));
        }
    }

    let body = response.bytes().await.map_err(|e| e.to_string())?;
    if body.len() as u64 > MAX_PAGE_SIZE {
        return Err(format!(
            "page too large: {} bytes exceeds {MAX_PAGE_SIZE} byte limit",
            body.len()
        ));
    }
    let html = String::from_utf8_lossy(&body).into_owned();

    // Pre-fetch external stylesheets into a cache keyed by resolved URL.
    let mut css_cache = HashMap::new();
    let links = extract_stylesheet_links(&html, &base);
    let capped = if links.len() > MAX_STYLESHEETS {
        &links[..MAX_STYLESHEETS]
    } else {
        &links
    };
    for css_url in capped {
        fetch_css_recursive(client, &css_url, &mut css_cache, 0).await;
    }

    Ok((html, css_cache))
}

/// Fetch a single CSS file and follow @import directives up to MAX_IMPORT_DEPTH.
async fn fetch_css_recursive(
    client: &reqwest::Client,
    url: &Url,
    cache: &mut HashMap<String, String>,
    depth: usize,
) {
    let key = url.to_string();
    if cache.contains_key(&key) || depth > MAX_IMPORT_DEPTH {
        return;
    }

    let css = match fetch_css(client, url).await {
        Some(text) => text,
        None => return,
    };

    // Scan for @import before inserting (we need the text)
    let imports = extract_css_imports(&css, url);
    cache.insert(key, css);

    for import_url in imports {
        if cache.len() >= MAX_STYLESHEETS {
            break;
        }
        Box::pin(fetch_css_recursive(client, &import_url, cache, depth + 1)).await;
    }
}

/// Fetch a single CSS URL with size limits. Returns None on failure.
async fn fetch_css(client: &reqwest::Client, url: &Url) -> Option<String> {
    let resp = client.get(url.clone()).send().await.ok()?;
    if resp
        .content_length()
        .map_or(false, |len| len > MAX_CSS_SIZE)
    {
        return None;
    }
    let bytes = resp.bytes().await.ok()?;
    if bytes.len() as u64 > MAX_CSS_SIZE {
        return None;
    }
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

/// Scan CSS text for `@import url(...)` or `@import "..."` directives.
/// Returns resolved URLs.
fn extract_css_imports(css: &str, base: &Url) -> Vec<Url> {
    let mut results = Vec::new();
    let lower = css.to_ascii_lowercase();
    let mut pos = 0;

    while let Some(offset) = lower[pos..].find("@import") {
        let start = pos + offset + 7; // skip "@import"

        // Skip whitespace
        let remaining = &css[start..];
        let trimmed = remaining.trim_start();
        let after_ws = start + (remaining.len() - trimmed.len());

        let href = if trimmed.starts_with("url(") {
            // @import url("...") or @import url(...)
            let inner = &trimmed[4..];
            extract_url_value(inner)
        } else if trimmed.starts_with('"') || trimmed.starts_with('\'') {
            // @import "..." or @import '...'
            let quote = trimmed.as_bytes()[0] as char;
            let rest = &trimmed[1..];
            rest.find(quote).map(|end| rest[..end].to_string())
        } else {
            None
        };

        if let Some(href) = href {
            if let Ok(resolved) = base.join(&href) {
                results.push(resolved);
            }
        }

        pos = after_ws + 1;
    }

    results
}

/// Extract a URL value from inside `url(...)`.
fn extract_url_value(s: &str) -> Option<String> {
    let trimmed = s.trim_start();
    if trimmed.starts_with('"') || trimmed.starts_with('\'') {
        let quote = trimmed.as_bytes()[0] as char;
        let rest = &trimmed[1..];
        let end = rest.find(quote)?;
        Some(rest[..end].to_string())
    } else {
        let end = trimmed.find(')')?;
        Some(trimmed[..end].trim().to_string())
    }
}

/// Scan HTML for `<link rel="stylesheet" href="...">` tags.
/// Returns resolved URLs.
fn extract_stylesheet_links(html: &str, base: &Url) -> Vec<Url> {
    let mut results = Vec::new();
    let lower = html.to_ascii_lowercase();
    let mut pos = 0;

    while let Some(offset) = lower[pos..].find("<link") {
        let start = pos + offset;
        let Some(end_offset) = lower[start..].find('>') else {
            break;
        };
        let end = start + end_offset + 1;
        let tag_lower = &lower[start..end];
        pos = end;

        if !tag_lower.contains("stylesheet") {
            continue;
        }

        // Extract href from the original (case-preserving) tag
        let Some(href) = extract_attr(&html[start..end], "href") else {
            continue;
        };

        if let Ok(resolved) = base.join(&href) {
            results.push(resolved);
        }
    }

    results
}

/// Fetch an image URL and return the raw bytes.
pub(crate) async fn fetch_image(url: String) -> Result<Vec<u8>, String> {
    let client = &*HTTP_CLIENT;
    let response = client.get(&url).send().await.map_err(|e| e.to_string())?;

    if let Some(len) = response.content_length() {
        if len > MAX_IMAGE_SIZE {
            return Err(format!(
                "image too large: {len} bytes exceeds {MAX_IMAGE_SIZE} byte limit"
            ));
        }
    }

    let bytes = response.bytes().await.map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_IMAGE_SIZE {
        return Err(format!(
            "image too large: {} bytes exceeds {MAX_IMAGE_SIZE} byte limit",
            bytes.len()
        ));
    }

    Ok(bytes.to_vec())
}

/// Pull the value of a named attribute out of a single HTML tag string.
fn extract_attr(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let needle = format!("{name}=");
    let idx = lower.find(&needle)?;
    let after = &tag[idx + needle.len()..];

    if after.starts_with('"') || after.starts_with('\'') {
        let quote = after.as_bytes()[0] as char;
        let inner = &after[1..];
        let end = inner.find(quote)?;
        Some(inner[..end].to_string())
    } else {
        let end = after
            .find(|c: char| c.is_whitespace() || c == '>' || c == '/')
            .unwrap_or(after.len());
        Some(after[..end].to_string())
    }
}
