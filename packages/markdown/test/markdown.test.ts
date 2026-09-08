// @vitest-environment jsdom
import { afterAll, afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { cleanup, render } from "@testing-library/svelte";
import { tick } from "svelte";
import IncrementalMarkdown from "../src/IncrementalMarkdown.svelte";
import { specs } from "./spec-loader";

/**
 * Renders markdown through the IncrementalMarkdown component and returns the
 * produced HTML (the contents of the component's root element).
 */
async function renderMarkdown(markdown: string): Promise<string> {
	const { container } = render(IncrementalMarkdown, { props: { text: markdown } });
	await tick(); // let the $effect that feeds session.update(text) run
	return container.querySelector(".streaming-markdown")!.innerHTML;
}

// Whitespace adjacent to block-level element boundaries is insignificant in
// HTML (the specs' expected HTML is pretty-printed with newlines), so it is
// stripped before comparison. Whitespace next to inline elements is kept —
// e.g. the newline in `<em>a</em>\n<em>b</em>` renders as a real space and
// dropping it would mask genuine softbreak bugs. Text inside <pre>/<textarea>
// is always significant.
const BLOCK_ELEMENTS = new Set([
	"BODY", "DIV", "P", "UL", "OL", "LI", "BLOCKQUOTE", "TABLE", "THEAD",
	"TBODY", "TR", "TD", "TH", "DL", "DT", "DD", "SECTION", "ARTICLE",
	"HEADER", "FOOTER", "H1", "H2", "H3", "H4", "H5", "H6",
]);

function normalizeHtml(html: string, stripComments = false): string {
	// cmark's output conventionally ends with a single newline. Strip it
	// before parsing — otherwise DOMParser would suck it into an unclosed
	// trailing inline element (e.g. a raw `<a href="...">` example) and it
	// would survive serialization as phantom content.
	const doc = new DOMParser().parseFromString(html.replace(/\n$/, ""), "text/html");

	if (stripComments) {
		// Svelte 5 emits <!----> anchor comments around every block. Only strip
		// them on the component side — comments in spec HTML are real content.
		const walker = doc.createTreeWalker(doc.body, NodeFilter.SHOW_COMMENT);
		const comments: Comment[] = [];
		for (let node = walker.nextNode() as Comment | null; node; node = walker.nextNode() as Comment | null) {
			comments.push(node);
		}
		for (const node of comments) node.remove();
		// Svelte splits consecutive expressions into separate text nodes around
		// anchor comments. Merge them back — otherwise a lone "\n" softbreak
		// node between two expressions would look like insignificant whitespace.
		doc.body.normalize();
	}

	const preserved = (node: Node): boolean => {
		for (let el = node.parentElement; el; el = el.parentElement) {
			if (el.tagName === "PRE" || el.tagName === "TEXTAREA") return true;
		}
		return false;
	};
	const isBlockBoundary = (node: Node | null): boolean =>
		node === null || (node.nodeType === 1 && BLOCK_ELEMENTS.has((node as Element).tagName));

	const walker = doc.createTreeWalker(doc.body, NodeFilter.SHOW_TEXT);
	const toRemove: Text[] = [];
	for (let node = walker.nextNode() as Text | null; node; node = walker.nextNode() as Text | null) {
		if (preserved(node)) continue;
		// cmark emits a newline right after <br>; collapsible whitespace at the
		// start of a line (immediately after a forced break) renders as nothing,
		// so strip it like any other insignificant whitespace.
		const prev = node.previousSibling;
		if (prev !== null && prev.nodeType === 1 && (prev as Element).tagName === "BR") {
			node.data = node.data.replace(/^\s*\n\s*/, "");
			if (node.data === "") toRemove.push(node);
			continue;
		}
		if (!node.data.includes("\n")) continue;
		const parent = node.parentElement;
		if (parent === null || !BLOCK_ELEMENTS.has(parent.tagName)) continue;
		if (node.data.trim() === "") {
			toRemove.push(node);
			continue;
		}
		if (isBlockBoundary(node.previousSibling)) node.data = node.data.replace(/^\s*\n\s*/, "");
		if (isBlockBoundary(node.nextSibling)) node.data = node.data.replace(/\s*\n\s*$/, "");
	}
	for (const node of toRemove) node.remove();
	return doc.body.innerHTML;
}

/**
 * Display-equivalence rules, applied to BOTH sides after whitespace
 * normalization. Each rule codifies a known, accepted difference between the
 * renderer's output and the spec's expected HTML — cases where both render
 * identically under this app's CSS.
 */
const equivalenceRules: { name: string; apply: (root: HTMLElement) => void }[] = [
	{
		// lezer does not distinguish tight from loose lists, and this app's CSS
		// gives li > p no visible margins, so a lone paragraph inside a list
		// item is unwrapped on both sides. Items containing multiple paragraphs
		// (loose structure) keep their <p> tags and stay comparable.
		name: "unwrap-single-p-in-li",
		apply(root) {
			for (const p of root.querySelectorAll("li > p")) {
				if (p.parentElement!.querySelectorAll(":scope > p").length > 1) continue;
				p.replaceWith(...p.childNodes);
			}
		},
	},
	{
		// GFM flattens long or nested same-type emphasis runs
		// (******foo****** → a single <strong>); lezer nests them per
		// CommonMark. Bold/italic are idempotent: a <strong> inside a <strong>
		// renders identically to its contents, so unwrap any emphasis element
		// that already has a same-tag ancestor. Text content is unchanged.
		name: "collapse-redundant-nested-emphasis",
		apply(root) {
			for (const tag of ["strong", "em"]) {
				for (const el of root.querySelectorAll(`${tag} ${tag}`)) {
					el.replaceWith(...el.childNodes);
				}
			}
		},
	},
];

function applyEquivalences(html: string): string {
	const doc = new DOMParser().parseFromString(html, "text/html");
	for (const rule of equivalenceRules) rule.apply(doc.body);
	return doc.body.innerHTML;
}

/**
 * Known parser-level gaps (NOT display-equivalence): lezer drops information
 * the spec expects in the output, so no render-layer fix is possible.
 * - Tabs 5/6/7: a tab straddling a container boundary and a code-block indent
 *   is partially consumed; the residual columns (2 spaces of code content)
 *   exist only as virtual tab-stop expansion and are absent from CodeText.
 * - Tabs 10: lezer does not treat a tab after an ATX marker as a space, so
 *   `#\tFoo` parses as a Paragraph instead of a heading.
 * Marked it.fails so an upstream fix flips them red and reminds us to delist.
 */
const KNOWN_PARSER_GAPS: { spec: string; index: number }[] = [
	...[5, 6, 7, 10].map((index) => ({ spec: "CommonMark spec", index })),
	...[5, 6, 7, 10].map((index) => ({ spec: "GFM spec", index })),
];

describe("html normalization", () => {
	it("strips the newline after <br>", () => {
		expect(normalizeHtml("<p>foo<br>\nbar</p>")).toBe("<p>foo<br>bar</p>");
	});
	it("keeps the newline after <br> inside <pre>", () => {
		expect(normalizeHtml("<pre>a<br>\nb</pre>")).toBe("<pre>a<br>\nb</pre>");
	});
});

describe("equivalence rules", () => {
	it("unwraps a lone paragraph inside a list item", () => {
		expect(applyEquivalences("<ul><li><p>a</p></li></ul>")).toBe("<ul><li>a</li></ul>");
	});
	it("unwraps a lone paragraph next to a nested list", () => {
		expect(applyEquivalences("<ul><li><p>a</p><ul><li><p>b</p></li></ul></li></ul>"))
			.toBe("<ul><li>a<ul><li>b</li></ul></li></ul>");
	});
	it("keeps multiple paragraphs in one item comparable", () => {
		expect(applyEquivalences("<ul><li><p>a</p><p>b</p></li></ul>"))
			.toBe("<ul><li><p>a</p><p>b</p></li></ul>");
	});
	it("collapses a chain of nested <strong>", () => {
		expect(applyEquivalences("<p><strong><strong><strong>foo</strong></strong></strong></p>"))
			.toBe("<p><strong>foo</strong></p>");
	});
	it("unwraps a redundant nested <strong> with siblings", () => {
		expect(applyEquivalences("<p><strong>foo <strong>bar</strong></strong></p>"))
			.toBe("<p><strong>foo bar</strong></p>");
	});
	it("keeps mixed <strong>/<em> nesting", () => {
		expect(applyEquivalences("<p><em><strong>foo</strong></em></p>"))
			.toBe("<p><em><strong>foo</strong></em></p>");
	});
});

// IncrementalMarkdown logs its nodes/tree on every update; keep output readable.
beforeAll(() => {
	vi.spyOn(console, "log").mockImplementation(() => {});
});
afterAll(() => {
	vi.restoreAllMocks();
});
afterEach(cleanup);

// Run statistics: how many spec examples pass byte-for-byte vs. only after
// the equivalence rules. A fast-growing "rescued" share means the rules may
// be overreaching and silently weakening the suite.
let rawPass = 0;
let rescuedByRules = 0;
afterAll(() => {
	const total = specs.reduce((n, list) => n + list.length, 0);
	// console.log is mocked above; write straight to stdout.
	process.stdout.write(
		`\n[markdown spec] ${rawPass} passed raw, ${rescuedByRules} rescued by equivalence rules, ` +
		`${total - rawPass - rescuedByRules} still failing (incl. ${KNOWN_PARSER_GAPS.length} known parser gaps)\n`
	);
});

const specNames = ["CommonMark spec", "GFM spec"] as const;

specNames.forEach((specName, specIndex) => {
	const examples = specs[specIndex];
	const bySection = new Map<string, typeof examples>();
	for (const example of examples) {
		const list = bySection.get(example.section) ?? [];
		list.push(example);
		bySection.set(example.section, list);
	}

	describe(specName, () => {
		for (const [section, sectionExamples] of bySection) {
			describe(section || "(no section)", () => {
				for (const example of sectionExamples) {
					const isParserGap = KNOWN_PARSER_GAPS.some(
						(gap) => gap.spec === specName && gap.index === example.index
					);
					(isParserGap ? it.fails : it)(`example ${example.index}`, async () => {
						// The specs use `→` as a readable stand-in for tab characters
						// in both the markdown input and the expected HTML.
						const expected = example.html.replaceAll("→", "\t");
						const actual = await renderMarkdown(example.markdown.replaceAll("→", "\t"));

						const rawActual = normalizeHtml(actual, true);
						const rawExpected = normalizeHtml(expected);
						let finalActual = rawActual, finalExpected = rawExpected;
						if (rawActual === rawExpected) {
							rawPass++;
						} else {
							finalActual = applyEquivalences(rawActual);
							finalExpected = applyEquivalences(rawExpected);
							if (finalActual === finalExpected) rescuedByRules++;
						}
						expect(finalActual).toBe(finalExpected);
					});
				}
			});
		}
	});
});
