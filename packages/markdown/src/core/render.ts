//! Spec-oriented HTML rendering for the lowered node tree.
//!
//! Two render paths share the value computations in this module:
//!
//! 1. The component tree (`components/MarkdownNode.svelte`) renders one Svelte
//!    component per node, so unchanged subtrees keep their DOM while streaming.
//! 2. `renderNode` builds an HTML string. It is used only where raw HTML from
//!    the source must interleave with rendered elements textually — an unclosed
//!    `<div>` in an HTML block has to wrap following blocks in the resulting
//!    DOM, which is impossible when every node is a separate DOM insertion.
//!    This applies to (a) runs of top-level blocks around unbalanced raw HTML
//!    blocks (`groupRenderUnits`) and (b) inline containers that contain raw
//!    inline HTML (`hasRawInline` / `renderInlineToString`).

import { decodeHTMLStrict } from "entities";
import type { Node } from "./markdown";
import { filterDisallowedTags, renderRawBlock } from "./raw-html";

// Public API moved to ./raw-html; re-exported here for existing consumers.
export { renderRawBlock, filterDisallowedTags };

// ---------------------------------------------------------------------------
// Shared value helpers (used by both the string renderer and the components)
// ---------------------------------------------------------------------------

/**
 * A lowered node with any node-name type. The render helpers accept nodes of
 * any parser configuration (the component layer passes the GFM-configured
 * `Node<GetNodeNames<typeof parser>>`).
 */
export type AnyNode = Node<any>;

const XML_SPECIAL_RE = /[&<>"]/g;

/** Escapes text/attribute content the same way cmark's escape_xml does. */
export function escapeXml(str: string): string {
	return str.replace(XML_SPECIAL_RE, (ch) =>
		ch === "&" ? "&amp;" : ch === "<" ? "&lt;" : ch === ">" ? "&gt;" : "&quot;");
}

function sanitizeCodePoint(cp: number): string {
	if (cp === 0 || cp > 0x10ffff || (cp >= 0xd800 && cp <= 0xdfff)) return "\uFFFD";
	return String.fromCodePoint(cp);
}

/** Decodes a single entity/numeric character reference; null when invalid. */
export function decodeEntity(raw: string): string | null {
	let m = /^&#(\d{1,7});$/.exec(raw);
	if (m) return sanitizeCodePoint(parseInt(m[1], 10));
	m = /^&#[xX]([0-9a-fA-F]{1,6});$/.exec(raw);
	if (m) return sanitizeCodePoint(parseInt(m[1], 16));
	if (raw.startsWith("&#")) return null;

	const decoded = decodeHTMLStrict(raw);
	return decoded === raw ? null : decoded;
}

const ESCAPABLE_RE = /\\([!"#$%&'()*+,\-./:;<=>?@\[\\\]^_`{|}~])|&(?:#[0-9]{1,7}|#[xX][0-9a-fA-F]{1,6}|[a-zA-Z][a-zA-Z0-9]{1,31});/g;

/** Resolves backslash escapes and entity references, per cmark's unescape_string. */
export function unescapeString(str: string): string {
	return str.replace(ESCAPABLE_RE, (match, escaped: string | undefined) =>
		escaped !== undefined ? escaped : decodeEntity(match) ?? match);
}

// ---------------------------------------------------------------------------
// URI percent-encoding (mirrors mdurl/encode with default chars)
// ---------------------------------------------------------------------------

const URL_DEFAULT_CHARS = new Set(";/?:@&=+$,-_.!~*'()#");

function encodeHref(url: string): string {
	let out = "";
	for (let i = 0; i < url.length; i++) {
		const code = url.charCodeAt(i);
		// Keep already-encoded sequences intact.
		if (code === 0x25 /* % */ && i + 2 < url.length && /^[0-9a-fA-F]{2}$/.test(url.slice(i + 1, i + 3))) {
			out += url.slice(i, i + 3);
			i += 2;
			continue;
		}
		if (code < 128) {
			const ch = url[i];
			out += /[0-9a-zA-Z]/.test(ch) || URL_DEFAULT_CHARS.has(ch)
				? ch
				: "%" + code.toString(16).toUpperCase().padStart(2, "0");
			continue;
		}
		const cp = url.codePointAt(i)!;
		if (cp > 0xffff) i++;
		try {
			out += encodeURIComponent(String.fromCodePoint(cp));
		} catch {
			out += "%EF%BF%BD";
		}
	}
	return out;
}

/** CommonMark's link destination cleanup: unescape, then percent-encode. */
export function normalizeUrl(raw: string): string {
	return encodeHref(unescapeString(raw));
}

// ---------------------------------------------------------------------------
// Node value extraction
// ---------------------------------------------------------------------------

/** Text node content with whitespace collapsed around soft line breaks. */
export function textContent(node: AnyNode): string {
	return (node.content ?? "").replace(/[ \t]*\n[ \t]*/g, "\n");
}

/** Code span text content (CommonMark's leading/trailing space rule). */
export function codeSpanText(node: AnyNode, inTable = false): string {
	let raw = "";
	for (const child of node.children) {
		if (child.name === "Text") raw += child.content;
	}
	// GFM resolves `\|` escapes in table cells even inside code spans.
	if (inTable) raw = raw.replace(/\\\|/g, "|");
	raw = raw.replaceAll("\n", " ");
	if (raw.startsWith(" ") && raw.endsWith(" ") && /[^ ]/.test(raw)) {
		raw = raw.slice(1, -1);
	}
	return raw;
}

/** Code block text: each line contributes its terminator when present. */
export function codeBlockText(node: AnyNode): string {
	let content = "";
	for (const child of node.children) {
		if (child.name === "CodeText") content += child.content ?? "";
	}
	if (content === "") return "";
	return content.endsWith("\n") ? content : content + "\n";
}

/** The info string's first word with escapes/entities resolved ("" when absent). */
export function codeBlockLanguage(node: AnyNode): string {
	const info = node.children.find((c) => c.name === "CodeInfo")?.content;
	return info ? unescapeString(info).split(/\s+/)[0] : "";
}

/** Destination/title children of a Link/Image node (values resolved). */
export function linkAttrs(node: AnyNode): { url: string | null; title: string | null } {
	let url: string | null = null;
	let title: string | null = null;
	for (const child of node.children) {
		if (child.name === "URL" && child.content !== undefined) {
			let raw = child.content;
			if (raw.startsWith("<") && raw.endsWith(">")) raw = raw.slice(1, -1);
			url = normalizeUrl(raw);
		} else if (child.name === "LinkTitle" && child.content !== undefined) {
			title = unescapeString(child.content.slice(1, -1));
		}
	}
	return { url, title };
}

/**
 * Whether the link/image has a usable destination: an inline `(...)` part, or
 * a reference resolved by the session (materialized as a URL child).
 */
export function hasDest(node: AnyNode): boolean {
	return node.children.some((c) =>
		(c.name === "LinkMark" && c.content === "(") ||
		(c.name === "URL" && c.content !== undefined));
}

/** Children that form the link text / image description (marks excluded). */
export function textChildren(node: AnyNode): AnyNode[] {
	// Text content sits between the opening `[`/`![` mark and the `]` mark;
	// the destination part (whitespace gaps included) follows it.
	let end = node.children.length;
	for (let i = 1; i < node.children.length; i++) {
		const child = node.children[i];
		if (child.name === "LinkMark" && child.content === "]") {
			end = i;
			break;
		}
	}
	return node.children.slice(1, end).filter((c) => c.name !== "LinkMark");
}

/** Plain-text rendering used for image alt attributes (tags suppressed). */
export function plainText(node: AnyNode): string {
	switch (node.name) {
		case "Text":
			return textContent(node);
		case "Escape":
			return node.content?.[1] ?? "";
		case "Entity":
			return decodeEntity(node.content!) ?? node.content!;
		case "InlineCode":
			return codeSpanText(node);
		case "HardBreak":
			return "\n";
		case "HTMLTag":
		case "Comment":
		case "ProcessingInstruction":
			return "";
		default: {
			if (node.name === "Link" || node.name === "Image") {
				const children = hasDest(node) ? textChildren(node) : node.children;
				let out = "";
				for (const child of children) {
					if (child.name === "LinkMark" && child.content !== "![") continue;
					out += plainText(child);
				}
				return out;
			}
			let out = "";
			for (const child of node.children) out += plainText(child);
			return out;
		}
	}
}

/** Angle-bracket autolink href: URIs stay, emails get a mailto: prefix. */
export function autolinkHref(url: string): string {
	return /^[a-zA-Z][a-zA-Z0-9+.-]{1,31}:/.test(url) ? encodeHref(url) : encodeHref("mailto:" + url);
}

/** GFM autolink (extension) href for bare www./scheme/email URL nodes. */
export function bareAutolinkHref(content: string): string {
	if (/^www\./.test(content)) return encodeHref("http://" + content);
	if (/^[a-zA-Z][a-zA-Z0-9+.-]{1,31}:/.test(content)) return encodeHref(content);
	return encodeHref("mailto:" + content);
}

/** Ordered list start number from the first item's marker; null when unknown. */
export function orderedListStart(node: AnyNode): number | null {
	const firstItem = node.children.find((c) => c.name === "ListItem");
	const mark = firstItem?.children.find((c) => c.name === "ListMark")?.content;
	if (!mark) return null;
	const start = parseInt(mark, 10);
	return Number.isNaN(start) ? null : start;
}

/** Whether a task list item's marker is checked (`[x]`/`[X]`). */
export function taskChecked(node: AnyNode): boolean {
	const marker = node.children.find((c) => c.name === "TaskMarker")?.content ?? "";
	return /^\[[xX]\]$/.test(marker);
}

/** Task children minus the checkbox marker. */
export function taskChildren(node: AnyNode): AnyNode[] {
	return node.children.filter((c) => c.name !== "TaskMarker");
}

// ---------------------------------------------------------------------------
// Tables (GFM extension)
// ---------------------------------------------------------------------------

export type TableAlign = "left" | "center" | "right" | null;

export interface TableParts {
	aligns: TableAlign[];
	headerCells: AnyNode[];
	rows: AnyNode[];
	colCount: number;
}

function parseAlignRow(content: string): TableAlign[] {
	// The delimiter row node spans the whole line, e.g. `| :-- | :-: |`.
	return content
		.replace(/^\s*\|/, "")
		.replace(/\|\s*$/, "")
		.split("|")
		.map((cell) => {
			const spec = cell.trim();
			if (!/^:?-+:?$/.test(spec)) return null;
			const left = spec.startsWith(":"), right = spec.endsWith(":");
			return left && right ? "center" : left ? "left" : right ? "right" : null;
		});
}

export function tableParts(node: AnyNode): TableParts {
	const header = node.children.find((c) => c.name === "TableHeader");
	const delimRow = node.children.find((c) => c.name === "TableDelimiter");
	const aligns = delimRow?.content !== undefined ? parseAlignRow(delimRow.content) : [];
	const headerCells = header?.children.filter((c) => c.name === "TableCell") ?? [];
	const rows = node.children.filter((c) => c.name === "TableRow");
	return { aligns, headerCells, rows, colCount: headerCells.length };
}

/** Row cells normalized to the column count (truncated/padded per GFM). */
export function rowCells(row: AnyNode, colCount: number): (AnyNode | null)[] {
	const cells: (AnyNode | null)[] = row.children.filter((c) => c.name === "TableCell").slice(0, colCount);
	while (cells.length < colCount) cells.push(null);
	return cells;
}

// ---------------------------------------------------------------------------
// Raw HTML
// ---------------------------------------------------------------------------

const RAW_INLINE_NAMES = new Set(["HTMLTag", "Comment", "ProcessingInstruction"]);

/** Whether any descendant is a raw inline HTML node (tag/comment/PI). */
export function hasRawInline(nodes: AnyNode[]): boolean {
	for (const node of nodes) {
		if (RAW_INLINE_NAMES.has(node.name)) return true;
		if (node.children.length && hasRawInline(node.children)) return true;
	}
	return false;
}

// ---------------------------------------------------------------------------
// String renderer (used for raw-HTML runs and raw-inline containers)
// ---------------------------------------------------------------------------

/** Rendering context propagated down the tree. */
interface RenderCtx {
	/** Inside a table cell: `\|` escapes are resolved at the table level. */
	inTable?: boolean;
}

function renderChildren(node: AnyNode, ctx?: RenderCtx): string {
	let out = "";
	for (const child of node.children) out += renderNode(child, ctx);
	return out;
}

function renderLink(node: AnyNode, ctx?: RenderCtx): string {
	if (!hasDest(node)) return renderFailedLink(node, ctx);
	const { url, title } = linkAttrs(node);
	const href = url ?? ""; // `[foo]()` — empty destination
	const titleAttr = title ? ` title="${escapeXml(title)}"` : "";
	let inner = "";
	for (const child of textChildren(node)) inner += renderNode(child, ctx);
	return `<a href="${escapeXml(href)}"${titleAttr}>${inner}</a>`;
}

function renderImage(node: AnyNode, ctx?: RenderCtx): string {
	if (!hasDest(node)) return renderFailedLink(node, ctx);
	const { url, title } = linkAttrs(node);
	const src = url ?? "";
	let alt = "";
	for (const child of textChildren(node)) alt += plainText(child);
	const titleAttr = title ? ` title="${escapeXml(title)}"` : "";
	return `<img src="${escapeXml(src)}" alt="${escapeXml(alt)}"${titleAttr} />`;
}

/** Literal rendering of a link/image without a usable destination. */
function renderFailedLink(node: AnyNode, ctx?: RenderCtx): string {
	let out = "";
	for (const child of node.children) {
		// The label is opaque source text in the tree, but a failed link renders
		// as ordinary inline text: escapes/entities inside it are resolved.
		if (child.name === "LinkLabel") out += escapeXml(unescapeString(child.content ?? ""));
		else if (child.name === "LinkMark") out += escapeXml(child.content ?? "");
		else out += renderNode(child, ctx);
	}
	return out;
}

function renderTask(node: AnyNode, ctx?: RenderCtx): string {
	let out = taskChecked(node)
		? `<input checked="" disabled="" type="checkbox" />`
		: `<input disabled="" type="checkbox" />`;
	for (const child of taskChildren(node)) out += renderNode(child, ctx);
	return out;
}

function renderTableCell(cell: AnyNode | null, tag: string, align: TableAlign): string {
	if (cell === null) return `<${tag}></${tag}>`;
	const alignAttr = align ? ` align="${align}"` : "";
	return `<${tag}${alignAttr}>${renderChildren(cell, { inTable: true })}</${tag}>`;
}

function renderTable(node: AnyNode): string {
	const { aligns, headerCells, rows, colCount } = tableParts(node);
	let out = "<table><thead><tr>";
	headerCells.forEach((cell, i) => (out += renderTableCell(cell, "th", aligns[i] ?? null)));
	out += "</tr></thead>";
	if (rows.length) {
		out += "<tbody>";
		for (const row of rows) {
			const cells = rowCells(row, colCount);
			out += "<tr>";
			cells.forEach((cell, i) => (out += renderTableCell(cell, "td", aligns[i] ?? null)));
			out += "</tr>";
		}
		out += "</tbody>";
	}
	return out + "</table>";
}

const HEADING_TAGS: Record<string, string> = {
	ATXHeading1: "h1", ATXHeading2: "h2", ATXHeading3: "h3",
	ATXHeading4: "h4", ATXHeading5: "h5", ATXHeading6: "h6",
	SetextHeading1: "h1", SetextHeading2: "h2",
};

export function renderNode(node: AnyNode, ctx?: RenderCtx): string {
	const name = node.name;
	if (name.endsWith("Mark") || name === "CodeInfo" || name === "CodeText" || name === "LinkLabel") return "";

	switch (name) {
		case "Text":
			return escapeXml(textContent(node));
		case "Entity":
			return escapeXml(decodeEntity(node.content!) ?? node.content!);
		case "Escape":
			return escapeXml(node.content?.[1] ?? "");
		case "URL":
			return `<a href="${escapeXml(bareAutolinkHref(node.content ?? ""))}">${escapeXml(node.content ?? "")}</a>`;
		case "Paragraph":
			return node.children.length === 0 ? "" : `<p>${renderChildren(node, ctx)}</p>`;
		case "StrongEmphasis":
			return `<strong>${renderChildren(node, ctx)}</strong>`;
		case "Emphasis":
			return `<em>${renderChildren(node, ctx)}</em>`;
		case "Strikethrough":
			return `<del>${renderChildren(node, ctx)}</del>`;
		case "InlineCode":
			return `<code>${escapeXml(codeSpanText(node, ctx?.inTable))}</code>`;
		case "CodeBlock":
		case "FencedCode": {
			const language = codeBlockLanguage(node);
			const classAttr = language ? ` class="language-${escapeXml(language)}"` : "";
			return `<pre><code${classAttr}>${escapeXml(codeBlockText(node))}</code></pre>`;
		}
		case "HardBreak":
			return "<br/>";
		case "HorizontalRule":
			return "<hr/>";
		case "Blockquote":
			return `<blockquote>${renderChildren(node, ctx)}</blockquote>`;
		case "BulletList":
			return `<ul>${renderChildren(node, ctx)}</ul>`;
		case "OrderedList": {
			const start = orderedListStart(node);
			const attr = start !== null && start !== 1 ? ` start="${start}"` : "";
			return `<ol${attr}>${renderChildren(node, ctx)}</ol>`;
		}
		case "ListItem":
			return `<li>${renderChildren(node, ctx)}</li>`;
		case "Task":
			return renderTask(node, ctx);
		case "Table":
			return renderTable(node);
		case "Autolink": {
			const url = node.children.find((c) => c.name === "URL")?.content ?? "";
			return `<a href="${escapeXml(autolinkHref(url))}">${escapeXml(url)}</a>`;
		}
		case "HTMLBlock":
		case "CommentBlock":
		case "ProcessingInstructionBlock":
			return renderRawBlock(node);
		case "HTMLTag":
		case "Comment":
		case "ProcessingInstruction":
			return filterDisallowedTags(node.content ?? "");
		case "Link":
			return renderLink(node, ctx);
		case "Image":
			return renderImage(node, ctx);
		case "LinkReference":
			return "";
		default: {
			const heading = HEADING_TAGS[name];
			if (heading) return `<${heading}>${renderChildren(node, ctx)}</${heading}>`;
			return node.content !== undefined ? escapeXml(node.content) : renderChildren(node, ctx);
		}
	}
}

/** Renders an inline child list to a string (for raw-HTML containers). */
export function renderInlineToString(nodes: AnyNode[], inTable = false): string {
	let out = "";
	for (const node of nodes) out += renderNode(node, { inTable });
	return out;
}

// ---------------------------------------------------------------------------
// Render-unit grouping: consecutive blocks whose combined HTML has unclosed
// tags must be inserted as one fragment so the browser nests them correctly.
// ---------------------------------------------------------------------------

export type RenderUnit =
	| { key: number; node: AnyNode; html?: undefined }
	| { key: number; html: string; node?: undefined };

/**
 * Groups a block-level node list into render units. Balanced nodes become
 * component units (rendered as a Svelte component tree, keeping fine-grained
 * DOM updates); a raw HTML block that leaves tags open starts an HTML run
 * that absorbs following blocks until the tag stack empties, rendered as one
 * `{@html}` fragment. Closers apply to the run stack exactly as the browser
 * parser would (a matching close drops everything above it, unmatched closers
 * are ignored), so run boundaries match the DOM the browser would build.
 * Raw summaries are materialized onto nodes by the reconciler (`Node.raw`),
 * so this scan is a pure O(number of blocks) read with no per-node work.
 */
export function groupRenderUnits(nodes: AnyNode[]): RenderUnit[] {
	const units: RenderUnit[] = [];
	let i = 0;
	while (i < nodes.length) {
		const node = nodes[i];
		const summary = node.raw ?? null;
		if (summary === null) {
			units.push({ key: node.id, node });
			i++;
			continue;
		}
		// A fragment that leaves nothing open is standalone; at this point the
		// stack is empty, so its unmatched closers are dropped by the parser.
		if (summary.opens.length === 0) {
			units.push({ key: node.id, html: summary.html });
			i++;
			continue;
		}
		// Unbalanced raw HTML: absorb following blocks until the stack empties.
		const htmlParts = [summary.html];
		const stack = [...summary.opens];
		i++;
		while (stack.length > 0 && i < nodes.length) {
			const member = nodes[i];
			const memberSummary = member.raw ?? null;
			htmlParts.push(memberSummary ? memberSummary.html : renderNode(member));
			if (memberSummary) {
				for (const tag of memberSummary.closers) {
					const at = stack.lastIndexOf(tag);
					if (at >= 0) stack.length = at;
				}
				stack.push(...memberSummary.opens);
			}
			i++;
		}
		units.push({ key: node.id, html: htmlParts.join("\n") });
	}
	return units;
}
