/**
 * Escape a string for safe interpolation in HTML text content.
 *
 * Replaces &, <, >, ", ' with their named/hex character references.
 * The ampersand is escaped first so that already-escaped sequences are
 * not double-escaped.
 */
export function escapeHtml(s: string): string {
	return s
		.replace(/&/g, '&amp;')
		.replace(/</g, '&lt;')
		.replace(/>/g, '&gt;')
		.replace(/"/g, '&quot;')
		.replace(/'/g, '&#39;');
}
