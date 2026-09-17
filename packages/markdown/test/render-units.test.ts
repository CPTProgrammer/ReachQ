import { describe, expect, it } from "vitest";
import { MarkdownSession } from "../src/core/markdown";
import { groupRenderUnits, type RenderUnit } from "../src/core/render";

/** Compact unit description: "component:Paragraph" or "html:<div>...". */
function kinds(units: RenderUnit[]): string[] {
	return units.map((u) => (u.node ? `component:${u.node.name}` : `html:${u.html}`));
}

function parse(doc: string) {
	return new MarkdownSession().update(doc);
}

describe("groupRenderUnits", () => {
	it("ends the HTML run at the closing tag block", () => {
		const units = groupRenderUnits(parse("<div>\n\nhello\n\n</div>\n\nafter"));
		expect(kinds(units)).toEqual([
			"html:<div>\n<p>hello</p>\n</div>",
			"component:Paragraph",
		]);
	});

	it("keeps a balanced raw block standalone", () => {
		const units = groupRenderUnits(parse("<div>inline</div>\n\nafter"));
		expect(kinds(units)).toEqual(["html:<div>inline</div>", "component:Paragraph"]);
	});

	it("keeps independent raw HTML sections in separate runs", () => {
		const units = groupRenderUnits(
			parse("<div>\n\na\n\n</div>\n\nb\n\n<span>\n\nc\n\n</span>\n\nd"),
		);
		expect(kinds(units)).toEqual([
			"html:<div>\n<p>a</p>\n</div>",
			"component:Paragraph",
			"html:<span>\n<p>c</p>\n</span>",
			"component:Paragraph",
		]);
	});

	it("treats a lone closing tag at top level as standalone (parser drops it)", () => {
		const units = groupRenderUnits(parse("</div>\n\nafter"));
		expect(kinds(units)).toEqual(["html:</div>", "component:Paragraph"]);
	});

	it("honors implicit closes: a closer also drops tags opened above it", () => {
		// </div> closes the still-open <span> as well, so the run ends there.
		const units = groupRenderUnits(parse("<div>\n\n<span>\n\n</div>\n\nx"));
		expect(kinds(units)).toEqual([
			"html:<div>\n<span>\n</div>",
			"component:Paragraph",
		]);
	});

	it("ignores foreign closers that match no open tag", () => {
		// The browser drops </a>; the div stays open and swallows the rest.
		const units = groupRenderUnits(parse("<div>\n\n</a>\n\nx"));
		expect(kinds(units)).toEqual(["html:<div>\n</a>\n<p>x</p>"]);
	});

	it("streaming: blocks after the closer become separate units", () => {
		const session = new MarkdownSession();
		let nodes = session.update("<div>\n\nhello");
		expect(kinds(groupRenderUnits(nodes))).toEqual(["html:<div>\n<p>hello</p>"]);

		nodes = session.append("\n\n</div>");
		const closed = groupRenderUnits(nodes);
		expect(kinds(closed)).toEqual(["html:<div>\n<p>hello</p>\n</div>"]);

		nodes = session.append("\n\nafter");
		const grown = groupRenderUnits(nodes);
		expect(kinds(grown)).toEqual([
			"html:<div>\n<p>hello</p>\n</div>",
			"component:Paragraph",
		]);
		// The closed run's html string is unchanged — Svelte leaves its DOM alone.
		expect(grown[0].html).toBe(closed[0].html);

		nodes = session.append("\n\nmore");
		expect(kinds(groupRenderUnits(nodes))).toEqual([
			"html:<div>\n<p>hello</p>\n</div>",
			"component:Paragraph",
			"component:Paragraph",
		]);
	});

	it("streaming: an html block growing at the document tail stays fresh", () => {
		// Regression: the node object is reused in place while its content
		// grows, so any cache keyed by node identity serves stale summaries.
		const session = new MarkdownSession();
		let nodes = session.update("<div");
		expect(kinds(groupRenderUnits(nodes))).toEqual(["html:<div"]);

		// "<div>" leaves a tag open: starts a run absorbing following blocks.
		nodes = session.append(">");
		expect(kinds(groupRenderUnits(nodes))).toEqual(["html:<div>"]);

		nodes = session.append("\nhello");
		expect(kinds(groupRenderUnits(nodes))).toEqual(["html:<div>\nhello"]);

		nodes = session.append("\n</div>");
		expect(kinds(groupRenderUnits(nodes))).toEqual(["html:<div>\nhello\n</div>"]);

		nodes = session.append("\n\nafter");
		expect(kinds(groupRenderUnits(nodes))).toEqual([
			"html:<div>\nhello\n</div>",
			"component:Paragraph",
		]);
	});
});
