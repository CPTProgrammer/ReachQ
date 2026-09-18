// @vitest-environment jsdom
/**
 * Security regression suite: every payload rendered with sanitization ON
 * (the default) must satisfy the no-XSS invariants below, per injection sink:
 *
 * - raw HTML blocks        → MarkdownNode / MarkdownNodeList `{@html}`
 * - raw inline HTML        → InlineContent `{@html}` (string render path)
 * - cross-block raw runs   → groupRenderUnits joined `{@html}`
 * - markdown link/image/autolink destinations → component-path href/src
 *
 * The suite deliberately re-tests classes DOMPurify covers upstream (mXSS,
 * entity obfuscation): the point is not to re-audit the sanitizer but to
 * prove every sink in THIS package actually routes through it.
 */
import { afterAll, afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { cleanup, render } from "@testing-library/svelte";
import { tick } from "svelte";
import IncrementalMarkdown from "../src/IncrementalMarkdown.svelte";

async function renderContainer(markdown: string, sanitize = true): Promise<HTMLElement> {
	const { container } = render(IncrementalMarkdown, { props: { text: markdown, sanitize } });
	await tick();
	return container.querySelector(".streaming-markdown") as HTMLElement;
}

const DANGEROUS_URL_RE = /^[\s\x00-\x20]*(?:javascript|vbscript|file):/i;

/** Generic invariants checked against every payload's rendered DOM. */
function assertNoXss(root: HTMLElement) {
	expect(root.querySelector("script, iframe, object, embed, base, meta[http-equiv], form, button"))
		.toBeNull();
	for (const el of root.querySelectorAll("*")) {
		for (const attr of el.attributes) {
			expect(attr.name, `<${el.tagName.toLowerCase()}> carries ${attr.name}`).not.toMatch(/^on/i);
			if (/^(?:href|src|action|formaction|data)$/i.test(attr.name)) {
				expect(attr.value, `<${el.tagName.toLowerCase()} ${attr.name}="${attr.value}">`)
					.not.toMatch(DANGEROUS_URL_RE);
				if (/^[\s\x00-\x20]*data:/i.test(attr.value)) {
					// data: survives only as a script-incapable raster image on <img>.
					expect(el.tagName, `data: URL on <${el.tagName.toLowerCase()}>`).toBe("IMG");
					expect(attr.value, attr.value).toMatch(/^data:image\/(?:png|gif|jpeg|webp)[;,]/i);
				}
			}
		}
	}
}

beforeAll(() => {
	vi.spyOn(console, "log").mockImplementation(() => {});
});
afterAll(() => {
	vi.restoreAllMocks();
});
afterEach(cleanup);

describe("raw HTML blocks", () => {
	const payloads: [string, string][] = [
		["img onerror", "<img src=x onerror=alert(1)>"],
		["svg onload", "<svg onload=alert(1)></svg>"],
		["details ontoggle", "<details open ontoggle=alert(1)></details>"],
		["marquee onstart", "<marquee onstart=alert(1)>x</marquee>"],
		["anchor javascript:", '<a href="javascript:alert(1)">x</a>'],
		["object", '<object data="https://evil.example/x.html"></object>'],
		["embed", '<embed src="https://evil.example/x">'],
		["form phishing", '<form action="https://evil.example"><input name="p"></form>'],
		["meta refresh", '<meta http-equiv="refresh" content="0;url=https://evil.example">'],
		["base hijack", '<base href="https://evil.example/">'],
		// mXSS smoke vectors (DOMPurify's job, but prove the sink is wired):
		["mXSS mglyph/style", "<math><mtext><table><mglyph><style><img src=x onerror=alert(1)>"],
		["mXSS noscript", '<noscript><p title="</noscript><img src=x onerror=alert(1)>"></noscript>'],
	];
	for (const [label, md] of payloads) {
		it(`neutralizes ${label}`, async () => {
			assertNoXss(await renderContainer(md));
		});
	}

	it("keeps the GFM tagfilter behavior for script (visible text, not element)", async () => {
		const root = await renderContainer("<script>alert(1)</script>");
		expect(root.querySelector("script")).toBeNull();
		expect(root.textContent).toContain("<script>alert(1)</script>");
	});

	it("keeps the GFM tagfilter behavior for iframe", async () => {
		const root = await renderContainer('<iframe src="https://evil.example"></iframe>');
		expect(root.querySelector("iframe")).toBeNull();
		expect(root.textContent).toContain("<iframe");
	});
});

describe("raw inline HTML (paragraph string path)", () => {
	const payloads: [string, string][] = [
		["img onerror", "a <img src=x onerror=alert(1)> b"],
		["svg onload", "a <svg onload=alert(1)></svg> b"],
		["anchor javascript:", 'a <a href="javascript:alert(1)">x</a> b'],
		["details ontoggle", "a <details open ontoggle=alert(1)></details> b"],
	];
	for (const [label, md] of payloads) {
		it(`neutralizes ${label}`, async () => {
			assertNoXss(await renderContainer(md));
		});
	}

	it("keeps benign inline raw HTML and markdown siblings intact", async () => {
		const root = await renderContainer('a <kbd>Ctrl</kbd>+<b>C</b> [ok](https://example.com)');
		expect(root.querySelector("kbd")?.textContent).toBe("Ctrl");
		expect(root.querySelector("b")?.textContent).toBe("C");
		expect(root.querySelector("a")?.getAttribute("href")).toBe("https://example.com");
	});
});

describe("cross-block raw runs", () => {
	it("strips handlers from a run whose raw wrapper survives", async () => {
		const root = await renderContainer("<div onmouseover=alert(1)>\n\nparagraph\n\n</div>");
		assertNoXss(root);
		// The unclosed-tag wrapping semantics survive sanitization.
		expect(root.querySelector("div > p")?.textContent).toBe("paragraph");
	});

	it("neutralizes a javascript: wrapper around rendered markdown", async () => {
		const root = await renderContainer('<a href="javascript:alert(1)">\n\nclick\n\n</a>');
		assertNoXss(root);
	});

	it("keeps task-list checkboxes rendered inside a run", async () => {
		const root = await renderContainer("<div>\n\n- [x] done\n- [ ] todo\n\n</div>");
		const boxes = root.querySelectorAll("input[type=checkbox]");
		expect(boxes).toHaveLength(2);
		expect((boxes[0] as HTMLInputElement).checked).toBe(true);
		expect((boxes[0] as HTMLInputElement).disabled).toBe(true);
	});
});

describe("markdown link/image destinations (component path)", () => {
	// Allowlist policy: anything but http(s)/mailto/tel/relative is neutralized
	// to href="" — dead link, no navigation.
	const blocked: [string, string][] = [
		["javascript:", "[click](javascript:alert(1))"],
		["javascript: entity-encoded", "[click](&#106;avascript:alert(1))"],
		["javascript: uppercased", "[click](JaVaScRiPt:alert(1))"],
		["vbscript:", "[click](vbscript:msgbox(1))"],
		["file:", "[click](file:///etc/passwd)"],
		["data:text/html", "[click](data:text/html,<script>alert(1)</script>)"],
		["reference-style javascript:", "[a][b]\n\n[b]: javascript:alert(1)"],
		["angle autolink javascript:", "<javascript:alert(1)>"],
		["unknown scheme (irc:)", "[click](irc://foo.bar:2233/baz)"],
		["android intent:", "[click](intent://evil.example/#Intent;end)"],
		["ssh:", "[click](ssh://root@evil.example)"],
	];
	for (const [label, md] of blocked) {
		it(`blocks ${label}`, async () => {
			const root = await renderContainer(md);
			assertNoXss(root);
			expect(root.querySelector("a")?.getAttribute("href") ?? "").toBe("");
		});
	}

	it("blocks javascript: in image src", async () => {
		const root = await renderContainer("![x](javascript:alert(1))");
		expect(root.querySelector("img")?.getAttribute("src") ?? "").toBe("");
	});

	it("blocks data:text/html in image src but keeps data:image/png", async () => {
		const blocked = await renderContainer("![x](data:text/html;base64,PHNjcmlwdD4=)");
		expect(blocked.querySelector("img")?.getAttribute("src") ?? "").toBe("");
		const allowed = await renderContainer("![x](data:image/png;base64,iVBORw0KGgo=)");
		expect(allowed.querySelector("img")?.getAttribute("src")).toBe("data:image/png;base64,iVBORw0KGgo=");
	});

	const benign: [string, string, string][] = [
		["https", "[ok](https://example.com)", "https://example.com"],
		["http", "[ok](http://example.com)", "http://example.com"],
		["relative path", "[ok](/docs/page)", "/docs/page"],
		["anchor", "[ok](#section)", "#section"],
		["mailto", "[ok](mailto:a@b.c)", "mailto:a@b.c"],
		["mailto uppercased", "<MAILTO:FOO@BAR.BAZ>", "MAILTO:FOO@BAR.BAZ"],
		["tel", "[ok](tel:+15551234567)", "tel:+15551234567"],
	];
	for (const [label, md, expected] of benign) {
		it(`keeps benign ${label} link`, async () => {
			const root = await renderContainer(md);
			expect(root.querySelector("a")?.getAttribute("href")).toBe(expected);
		});
	}

	it("keeps the link title attribute", async () => {
		const root = await renderContainer('[ok](https://example.com "the title")');
		expect(root.querySelector("a")?.getAttribute("title")).toBe("the title");
	});
});

describe("sanitize=false (spec-conformance mode)", () => {
	it("passes raw HTML through untouched", async () => {
		const root = await renderContainer("<img src=x onerror=alert(1)>", false);
		expect(root.querySelector("img")?.getAttribute("onerror")).toBe("alert(1)");
	});

	it("passes all URL schemes through untouched", async () => {
		const root = await renderContainer("[ok](irc://foo.bar:2233/baz)", false);
		expect(root.querySelector("a")?.getAttribute("href")).toBe("irc://foo.bar:2233/baz");
	});
});
