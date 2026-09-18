// Multi-update behavior of reference resolution: the spec suite only renders
// single-shot documents, while the resolver's whole point is staying correct
// and incremental across streaming edits.
import { describe, expect, it } from "vitest";
import { MarkdownSession } from "../src/core/markdown";
import { renderNode, type AnyNode } from "../src/core/render";

function htmlOf(nodes: AnyNode[]): string {
	let out = "";
	for (const node of nodes) out += renderNode(node);
	return out;
}

describe("reference resolution across updates", () => {
	it("resolves a shortcut reference when the definition arrives later", () => {
		const session = new MarkdownSession();
		let nodes = session.update("[foo]");
		expect(htmlOf(nodes)).toBe("<p>[foo]</p>");
		nodes = session.update("[foo]\n\n[foo]: /url \"title\"");
		expect(htmlOf(nodes)).toBe('<p><a href="/url" title="title">foo</a></p>');
	});

	it("resolves full and collapsed references", () => {
		const session = new MarkdownSession();
		const nodes = session.update("[a][b] and [a][]\n\n[b]: /u1\n[a]: /u2");
		expect(htmlOf(nodes)).toBe('<p><a href="/u1">a</a> and <a href="/u2">a</a></p>');
	});

	it("resolves references to definitions inside block quotes", () => {
		const session = new MarkdownSession();
		const nodes = session.update("> [foo]: /url\n\n[foo]");
		expect(htmlOf(nodes)).toBe('<blockquote></blockquote><p><a href="/url">foo</a></p>');
	});

	it("updates links when the definition's destination changes", () => {
		const session = new MarkdownSession();
		session.update("[foo]\n\n[foo]: /url1");
		const nodes = session.update("[foo]\n\n[foo]: /url2");
		expect(htmlOf(nodes)).toBe('<p><a href="/url2">foo</a></p>');
	});

	it("reverts to literal text when the definition disappears", () => {
		const session = new MarkdownSession();
		session.update("[foo]\n\n[foo]: /url");
		const nodes = session.update("[foo]\n\ntext");
		expect(htmlOf(nodes)).toBe("<p>[foo]</p><p>text</p>");
	});

	it("first definition wins; removing it promotes the second", () => {
		const session = new MarkdownSession();
		let nodes = session.update("[x]\n\n[x]: /1\n[x]: /2");
		expect(htmlOf(nodes)).toBe('<p><a href="/1">x</a></p>');
		nodes = session.update("[x]\n\n[x]: /2");
		expect(htmlOf(nodes)).toBe('<p><a href="/2">x</a></p>');
	});

	it("re-resolves when the link text is edited into another label", () => {
		const session = new MarkdownSession();
		let nodes = session.update("[fo]\n\n[fo]: /a\n[foo]: /b");
		expect(htmlOf(nodes)).toBe('<p><a href="/a">fo</a></p>');
		nodes = session.update("[foo]\n\n[fo]: /a\n[foo]: /b");
		expect(htmlOf(nodes)).toBe('<p><a href="/b">foo</a></p>');
	});

	it("keeps resolutions stable when an unrelated paragraph is edited", () => {
		const session = new MarkdownSession();
		session.update("[foo]\n\nmiddle\n\n[foo]: /url");
		const nodes = session.update("[foo]\n\nmiddle!\n\n[foo]: /url");
		expect(htmlOf(nodes)).toBe('<p><a href="/url">foo</a></p><p>middle!</p>');
	});

	it("updates every link sharing a label when its definition changes", () => {
		const session = new MarkdownSession();
		session.update("[x] one\n\n[x] two\n\n[x]: /1");
		const nodes = session.update("[x] one\n\n[x] two\n\n[x]: /2");
		expect(htmlOf(nodes)).toBe('<p><a href="/2">x</a> one</p><p><a href="/2">x</a> two</p>');
	});

	it("matches labels case-insensitively with whitespace collapsing", () => {
		const session = new MarkdownSession();
		const nodes = session.update("[FOO]\n\n[Foo\n  Bar]: /url\n\n[foo  bar]");
		expect(htmlOf(nodes)).toBe('<p>[FOO]</p><p><a href="/url">foo  bar</a></p>');
	});

	it("falls back to a shortcut when the inline destination is invalid", () => {
		const session = new MarkdownSession();
		const nodes = session.update("[foo](not a link)\n\n[foo]: /url1");
		expect(htmlOf(nodes)).toBe('<p><a href="/url1">foo</a>(not a link)</p>');
	});

	it("does not resolve escapes when matching labels", () => {
		const session = new MarkdownSession();
		const nodes = session.update("[bar][foo\\!]\n\n[foo!]: /url");
		expect(htmlOf(nodes)).toBe("<p>[bar][foo!]</p>");
	});

	it("matches labels with full Unicode case folding", () => {
		const session = new MarkdownSession();
		// ß→ss (the spec's example), ſ→s (long s), ﬀ→ff (ligature), µ→μ (micro sign)
		const nodes = session.update(
			"[ẞ] [ſoo] [ﬀ] [µ]\n\n[SS]: /1\n[soo]: /2\n[ff]: /3\n[μ]: /4"
		);
		expect(htmlOf(nodes)).toBe(
			'<p><a href="/1">ẞ</a> <a href="/2">ſoo</a> <a href="/3">ﬀ</a> <a href="/4">µ</a></p>'
		);
	});
});
