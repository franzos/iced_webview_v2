//! Resource fetching for the litehtml/blitz engines.
//!
//! SSRF note: fetched URLs come from page content and may target localhost
//! or private address ranges; redirects are followed. Embedders rendering
//! untrusted content from privileged network positions should be aware.

use iced::futures::{stream, StreamExt};
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

    let body = read_body_limited(response, MAX_PAGE_SIZE)
        .await
        .map_err(|e| match e {
            ReadBodyError::TooLarge => {
                format!("page too large: exceeds {MAX_PAGE_SIZE} byte limit")
            }
            ReadBodyError::Network(e) => e.to_string(),
        })?;
    let html = String::from_utf8_lossy(&body).into_owned();

    // Pre-fetch external stylesheets into a cache keyed by resolved URL.
    let mut css_cache = HashMap::new();
    let links = extract_stylesheet_links(&html, &base);
    let capped = if links.len() > MAX_STYLESHEETS {
        &links[..MAX_STYLESHEETS]
    } else {
        &links
    };
    // Fetch top-level stylesheet bodies concurrently; @import recursion
    // needs &mut cache, so it stays sequential below.
    let bodies: Vec<(Url, Option<String>)> = stream::iter(capped.to_vec())
        .map(|css_url| async move {
            let css = fetch_css(client, &css_url).await;
            (css_url, css)
        })
        .buffered(8)
        .collect()
        .await;
    for (css_url, css) in bodies {
        if let Some(css) = css {
            if !css_cache.contains_key(css_url.as_str()) {
                process_css(client, &css_url, css, &mut css_cache, 0).await;
            }
        }
    }

    Ok((html, css_cache))
}

enum ReadBodyError {
    TooLarge,
    Network(reqwest::Error),
}

/// Read a response body in chunks, aborting once `limit` is exceeded.
async fn read_body_limited(
    mut response: reqwest::Response,
    limit: u64,
) -> Result<Vec<u8>, ReadBodyError> {
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(ReadBodyError::Network)? {
        if body.len() as u64 + chunk.len() as u64 > limit {
            return Err(ReadBodyError::TooLarge);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
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

    process_css(client, url, css, cache, depth).await;
}

/// Insert fetched CSS into the cache and follow its @import directives.
async fn process_css(
    client: &reqwest::Client,
    url: &Url,
    css: String,
    cache: &mut HashMap<String, String>,
    depth: usize,
) {
    // Scan for @import before inserting (we need the text)
    let imports = extract_css_imports(&css, url);
    cache.insert(url.to_string(), css);

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
    if resp.content_length().is_some_and(|len| len > MAX_CSS_SIZE) {
        return None;
    }
    let bytes = read_body_limited(resp, MAX_CSS_SIZE).await.ok()?;
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

        let href = if let Some(inner) = trimmed.strip_prefix("url(") {
            // @import url("...") or @import url(...)
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

        if after_ws >= lower.len() {
            break;
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

    let bytes = read_body_limited(response, MAX_IMAGE_SIZE)
        .await
        .map_err(|e| match e {
            ReadBodyError::TooLarge => {
                format!("image too large: exceeds {MAX_IMAGE_SIZE} byte limit")
            }
            ReadBodyError::Network(e) => e.to_string(),
        })?;

    Ok(bytes)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn css_imports_trailing_import_does_not_panic() {
        let base = Url::parse("https://example.com/style.css").unwrap();
        assert!(extract_css_imports("@import", &base).is_empty());
        assert!(extract_css_imports("@import ", &base).is_empty());
    }

    #[test]
    fn css_imports_resolves_urls() {
        let base = Url::parse("https://example.com/css/style.css").unwrap();
        let imports = extract_css_imports("@import url(\"a.css\");\n@import 'b.css';", &base);
        let urls: Vec<String> = imports.iter().map(Url::to_string).collect();
        assert_eq!(
            urls,
            vec![
                "https://example.com/css/a.css".to_string(),
                "https://example.com/css/b.css".to_string(),
            ]
        );
    }
}
