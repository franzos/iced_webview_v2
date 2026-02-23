use url::Url;

/// Minimal HTML escaping for error pages.
pub(crate) fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Resolve a resource URL (image, CSS) with a 3-tier fallback:
/// 1. Parse `src` as absolute URL
/// 2. Resolve against `baseurl` (e.g. stylesheet URL)
/// 3. Resolve against `page_url`
pub(crate) fn resolve_url(
    src: &str,
    baseurl: &str,
    page_url: &str,
) -> Result<Url, url::ParseError> {
    Url::parse(src)
        .or_else(|_| {
            if !baseurl.is_empty() {
                Url::parse(baseurl).and_then(|b| b.join(src))
            } else {
                Err(url::ParseError::RelativeUrlWithoutBase)
            }
        })
        .or_else(|_| Url::parse(page_url).and_then(|base| base.join(src)))
}

/// Check if two URLs refer to the same page (ignoring fragment).
pub(crate) fn is_same_page(a: &Url, b: &Url) -> bool {
    a.scheme() == b.scheme()
        && a.host() == b.host()
        && a.port() == b.port()
        && a.path() == b.path()
        && a.query() == b.query()
}
