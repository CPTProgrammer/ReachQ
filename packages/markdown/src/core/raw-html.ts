//! Raw HTML block support shared by the reconciler (`markdown.ts`) and the
//! string renderer (`render.ts`): extracting a raw block's source text and
//! summarizing its tag balance across block boundaries.
//!
//! The reconciler materializes a `RawSummary` onto each raw block node
//! (`Node.raw`) at the exact moment the block's content (re)assigns, so the
//! summary can never disagree with the content and render-side grouping never
//! rescans unchanged text.

import type { Node } from "./markdown";

export interface RawSummary {
	html: string;
	/** Closing tags that matched nothing within this fragment (in order). */
	closers: string[];
	/** Tags left open at the end of this fragment (innermost last). */
	opens: string[];
}

export const RAW_BLOCK_NAMES = new Set(["HTMLBlock", "CommentBlock", "ProcessingInstructionBlock"]);

const VOID_TAGS = new Set([
	"area", "base", "br", "col", "embed", "hr", "img", "input",
	"link", "meta", "param", "source", "track", "wbr",
]);

const RAW_TAG_RE = /<!--[\s\S]*?-->|<!\[CDATA\[[\s\S]*?\]\]>|<![\s\S]*?>|<\?[\s\S]*?\?>|<(\/?)([a-zA-Z][a-zA-Z0-9-]*)((?:"[^"]*"|'[^']*'|[^>"'])*)>/g;

/**
 * GFM's disallowed-raw-HTML extension ("tagfilter", GFM spec §Disallowed
 * Raw HTML): these tags are neutralized by escaping the leading `<`, turning
 * them into visible text. Like cmark-gfm, the filter is applied at the
 * output layer rather than during parsing.
 */
const DISALLOWED_TAGS = new Set([
	"title", "textarea", "style", "xmp", "iframe",
	"noembed", "noframes", "script", "plaintext",
]);

/**
 * Escapes disallowed tags in a raw HTML fragment. RAW_TAG_RE consumes
 * comments, CDATA sections, declarations and PIs whole, so a `<script`
 * inside one of those is never touched.
 */
export function filterDisallowedTags(html: string): string {
	return html.replace(RAW_TAG_RE, (match, _slash: string | undefined, tagName: string | undefined) =>
		tagName !== undefined && DISALLOWED_TAGS.has(tagName.toLowerCase()) ? "&lt;" + match.slice(1) : match);
}

/**
 * Scans an HTML fragment for the tag events that matter across block
 * boundaries: closing tags the fragment could not pair locally, and the tags
 * it leaves open. A matched close also drops everything opened above it,
 * mirroring the browser parser's implicit closes.
 */
function tagSummary(html: string): { closers: string[]; opens: string[] } {
	const stack: string[] = [];
	const closers: string[] = [];
	RAW_TAG_RE.lastIndex = 0;
	for (let m; (m = RAW_TAG_RE.exec(html));) {
		const [, closingSlash, tagName, attrs] = m;
		if (tagName === undefined) continue; // comment / declaration / PI / CDATA
		const tag = tagName.toLowerCase();
		if (closingSlash) {
			const at = stack.lastIndexOf(tag);
			if (at >= 0) stack.length = at;
			else closers.push(tag);
		} else if (!VOID_TAGS.has(tag) && !attrs.trimEnd().endsWith("/")) {
			stack.push(tag);
		}
	}
	return { closers, opens: stack };
}

/** Raw passthrough for HTMLBlock/CommentBlock/ProcessingInstructionBlock,
 *  with the GFM tagfilter applied (see filterDisallowedTags). */
export function renderRawBlock(node: Node<any>): string {
	if (node.content !== undefined) return filterDisallowedTags(node.content);
	// The block carries container markers (QuoteMark) as element children;
	// the actual raw text lives in the synthetic Text children between them.
	let out = "";
	let afterQuoteMark = false;
	for (const child of node.children) {
		if (child.name === "QuoteMark") {
			afterQuoteMark = true;
			continue;
		}
		let content = child.content ?? "";
		// The quote marker consumes one optional following space.
		if (afterQuoteMark && content.startsWith(" ")) content = content.slice(1);
		afterQuoteMark = false;
		out += content;
	}
	return filterDisallowedTags(out);
}

/**
 * Source text plus tag-balance summary for raw blocks; null for other nodes.
 * The reconciler calls this only when the block's content (re)assigns.
 */
export function summarizeRawBlock(node: Node<any>): RawSummary | null {
	if (!RAW_BLOCK_NAMES.has(node.name)) return null;
	const html = renderRawBlock(node);
	const { closers, opens } = tagSummary(html);
	return { html, closers, opens };
}
