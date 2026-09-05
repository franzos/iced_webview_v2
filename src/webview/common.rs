//! Logic shared between the basic and advanced webviews. Behavior here is
//! identical for both; only the `Action` enums differ, which callers bridge
//! by passing variant constructors as `make_action`.

use std::collections::HashMap;
use std::sync::Arc;

use iced::Task;
use url::Url;

use crate::engines::Engine;
use crate::{PageType, ViewId};

/// Per-navigation cap on image fetches, mirroring MAX_STYLESHEETS in fetch.rs.
#[cfg(any(feature = "litehtml", feature = "blitz"))]
const MAX_IMAGES: usize = 128;

/// Result type of `crate::fetch::fetch_html`: `(html, css_cache)` on success.
pub(crate) type FetchHtmlResult = Result<(String, HashMap<String, String>), String>;

/// Constructor for a widget's `ImageFetchComplete` action variant.
#[cfg(any(feature = "litehtml", feature = "blitz"))]
pub(crate) type MakeImageAction<A> = fn(ViewId, String, Result<Vec<u8>, String>, bool, u64) -> A;

/// Where a clicked anchor should take the view.
pub(crate) enum AnchorTarget {
    /// Same-page link: scroll to this fragment.
    Fragment(String),
    /// Cross-page http(s) link: navigate here.
    Navigate(Url),
}

/// Resolve a clicked anchor `href` against the current page URL.
/// Non-http(s) schemes, same-page links without a fragment, and
/// unresolvable hrefs (logged) yield `None`.
pub(crate) fn resolve_anchor_click(href: &str, current_url: &str) -> Option<AnchorTarget> {
    let base = Url::parse(current_url).ok();
    match Url::parse(href).or_else(|_| {
        base.as_ref()
            .ok_or(url::ParseError::RelativeUrlWithoutBase)
            .and_then(|b| b.join(href))
    }) {
        Ok(resolved) => {
            let scheme = resolved.scheme();
            if scheme != "http" && scheme != "https" {
                return None;
            }
            let is_same_page = base
                .as_ref()
                .is_some_and(|cur| crate::util::is_same_page(&resolved, cur));
            if is_same_page {
                resolved
                    .fragment()
                    .map(|fragment| AnchorTarget::Fragment(fragment.to_string()))
            } else {
                Some(AnchorTarget::Navigate(resolved))
            }
        }
        Err(e) => {
            log::warn!("iced_webview: failed to resolve anchor URL '{href}': {e}");
            None
        }
    }
}

/// Query the window scale factor from iced and route it back as an action.
pub(crate) fn query_scale_factor<A, Message>(
    action_mapper: &Option<Arc<dyn Fn(A) -> Message + Send + Sync>>,
    make_action: fn(f32) -> A,
) -> Task<Message>
where
    A: 'static,
    Message: Send + 'static,
{
    if let Some(mapper) = action_mapper {
        let mapper = mapper.clone();
        iced::window::latest()
            .and_then(iced::window::scale_factor)
            .map(move |f| mapper(make_action(f)))
    } else {
        Task::none()
    }
}

/// Reset per-navigation image counters and bump the view's epoch so that
/// in-flight image fetches from the previous page are discarded.
pub(crate) fn begin_navigation(
    nav_epochs: &mut HashMap<ViewId, u64>,
    inflight_images: &mut usize,
    fetched_images: &mut usize,
    view_id: ViewId,
) {
    *inflight_images = 0;
    *fetched_images = 0;
    let epoch = nav_epochs.entry(view_id).or_insert(0);
    *epoch = epoch.wrapping_add(1);
}

/// Spawn an HTML fetch for engines without native URL support.
#[cfg(any(feature = "litehtml", feature = "blitz"))]
pub(crate) fn fetch_html_task<A, Message>(
    view_id: ViewId,
    url: String,
    mapper: Arc<dyn Fn(A) -> Message + Send + Sync>,
    make_action: fn(ViewId, String, FetchHtmlResult) -> A,
) -> Task<Message>
where
    A: 'static,
    Message: Send + 'static,
{
    let url_clone = url.clone();
    Task::perform(crate::fetch::fetch_html(url), move |result| {
        mapper(make_action(view_id, url_clone, result))
    })
}

/// Fetch images discovered during layout, bumping the shared counters and
/// tagging each fetch with the view's current navigation epoch.
#[cfg(any(feature = "litehtml", feature = "blitz"))]
#[allow(clippy::too_many_arguments)]
pub(crate) fn dispatch_image_fetches<E, A, Message>(
    engine: &mut E,
    nav_epochs: &HashMap<ViewId, u64>,
    fetched_images: &mut usize,
    inflight_images: &mut usize,
    mapper: &Arc<dyn Fn(A) -> Message + Send + Sync>,
    make_action: MakeImageAction<A>,
    tasks: &mut Vec<Task<Message>>,
) where
    E: Engine,
    A: 'static,
    Message: Send + 'static,
{
    let pending = engine.take_pending_images();
    for (view_id, src, baseurl, redraw_on_ready) in pending {
        let page_url = engine.get_url(view_id);
        // Resolve against the baseurl context (e.g. stylesheet URL),
        // falling back to the page URL.
        let resolved = match crate::util::resolve_url(&src, &baseurl, &page_url) {
            Ok(u) => u,
            Err(_) => continue,
        };
        let scheme = resolved.scheme();
        if scheme != "http" && scheme != "https" {
            continue;
        }
        if *fetched_images >= MAX_IMAGES {
            continue;
        }
        *fetched_images += 1;
        *inflight_images += 1;
        let mapper = mapper.clone();
        let raw_src = src.clone();
        let epoch = *nav_epochs.get(&view_id).unwrap_or(&0);
        tasks.push(Task::perform(
            crate::fetch::fetch_image(resolved.to_string()),
            move |result| {
                mapper(make_action(
                    view_id,
                    raw_src,
                    result,
                    redraw_on_ready,
                    epoch,
                ))
            },
        ));
    }
}

/// Apply a completed HTML fetch to the view. Returns `false` when the view
/// no longer exists (nothing was applied).
pub(crate) fn handle_fetch_complete<E: Engine>(
    engine: &mut E,
    view_id: ViewId,
    url: &str,
    result: FetchHtmlResult,
) -> bool {
    if !engine.has_view(view_id) {
        return false;
    }
    match result {
        Ok((html, css_cache)) => {
            engine.set_css_cache(view_id, css_cache);
            engine.goto(view_id, PageType::Html(html));
        }
        Err(e) => {
            let error_html = format!(
                "<html><body><h1>Failed to load</h1><p>{}</p><p>{}</p></body></html>",
                crate::util::html_escape(url),
                crate::util::html_escape(&e),
            );
            engine.goto(view_id, PageType::Html(error_html));
        }
    }
    true
}

/// Apply a completed image fetch: discard stale epochs, decrement the
/// inflight counter, and hand the bytes to the engine.
#[allow(clippy::too_many_arguments)]
pub(crate) fn handle_image_fetch_complete<E: Engine>(
    engine: &mut E,
    nav_epochs: &HashMap<ViewId, u64>,
    inflight_images: &mut usize,
    view_id: ViewId,
    src: &str,
    result: &Result<Vec<u8>, String>,
    redraw_on_ready: bool,
    epoch: u64,
) {
    let current_epoch = *nav_epochs.get(&view_id).unwrap_or(&0);
    if epoch != current_epoch {
        // Stale fetch from a previous navigation; the current epoch's
        // inflight counter must not be touched.
        return;
    }
    *inflight_images = inflight_images.saturating_sub(1);
    if engine.has_view(view_id) {
        match result {
            Ok(bytes) => {
                engine.load_image_from_bytes(view_id, src, bytes, redraw_on_ready);
            }
            Err(e) => {
                log::warn!("iced_webview: failed to fetch image '{}': {}", src, e);
            }
        }
    }
}
