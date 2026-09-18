//! HTML sanitization for raw fragments, living at the component layer so the
//! core (`core/`) stays DOM-free and its unit tests keep running under node.
//!
//! Applied only to strings that already passed the GFM tagfilter and that
//! contain raw author HTML; component-rendered content never touches this.

import DOMPurify from "dompurify";

const CONFIG = {
	// The renderer never emits forms; raw ones only enable credential phishing.
	// Bare <input> stays (inert without a form — formaction is not allowed by
	// default) so task-list checkboxes inside raw HTML runs survive.
	FORBID_TAGS: ["form", "button"],
};

/**
 * Bounded memoization: unchanged raw fragments are string-identical across
 * updates (their RawSummary is cached on the node by the reconciler), so
 * repeat sanitization of untouched blocks is O(1). Runs (unbalanced fragments
 * joined per update) miss the cache by nature — they were going to be fully
 * re-parsed by the browser on insertion anyway.
 */
const cache = new Map<string, string>();
const CACHE_LIMIT = 256;

/** Sanitizes an HTML fragment for `{@html}` insertion. */
export function sanitizeHtml(html: string): string {
	const hit = cache.get(html);
	if (hit !== undefined) return hit;
	const clean = DOMPurify.sanitize(html, CONFIG);
	if (cache.size >= CACHE_LIMIT) cache.clear();
	cache.set(html, clean);
	return clean;
}
