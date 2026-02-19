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

static HTTP_CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .expect("failed to build HTTP client")
});

/// Fetch a URL, resolve external stylesheets and inline them, then return
/// self-contained HTML ready for a renderer that has no networking.
pub(crate) async fn fetch_html(page_url: String) -> Result<String, String> {
    let client = &*HTTP_CLIENT;
    let base = Url::parse(&page_url).map_err(|e| e.to_string())?;

    let response = client.get(&page_url).send().await.map_err(|e| e.to_string())?;

    if let Some(len) = response.content_length() {
        if len > MAX_PAGE_SIZE {
            return Err(format!("page too large: {len} bytes exceeds {MAX_PAGE_SIZE} byte limit"));
        }
    }

    let body = response.bytes().await.map_err(|e| e.to_string())?;
    if body.len() as u64 > MAX_PAGE_SIZE {
        return Err(format!(
            "page too large: {} bytes exceeds {MAX_PAGE_SIZE} byte limit",
            body.len()
        ));
    }
    let mut html = String::from_utf8_lossy(&body).into_owned();

    // Find external stylesheets, process from end so byte offsets stay valid.
    let links = extract_stylesheet_links(&html, &base);
    let capped = if links.len() > MAX_STYLESHEETS {
        &links[..MAX_STYLESHEETS]
    } else {
        &links
    };
    for (range, css_url) in capped.iter().rev() {
        if let Ok(resp) = client.get(css_url.clone()).send().await {
            let too_large = resp
                .content_length()
                .map_or(false, |len| len > MAX_CSS_SIZE);
            if too_large {
                continue;
            }
            if let Ok(bytes) = resp.bytes().await {
                if bytes.len() as u64 > MAX_CSS_SIZE {
                    continue;
                }
                let css = String::from_utf8_lossy(&bytes);
                // Escape '<' so fetched CSS can't break out of the <style> tag
                let safe_css = css.replace('<', "\\3c ");
                html.replace_range(range.clone(), &format!("<style>{safe_css}</style>"));
            }
        }
    }

    Ok(html)
}

/// Scan HTML for `<link rel="stylesheet" href="...">` tags.
/// Returns (byte_range, resolved_url) pairs sorted by position.
fn extract_stylesheet_links(html: &str, base: &Url) -> Vec<(std::ops::Range<usize>, Url)> {
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
            results.push((start..end, resolved));
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
