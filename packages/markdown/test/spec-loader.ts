import { readFileSync } from "fs";
import path from "path";

type SpecTest = {
	markdown: string,
	html: string,
	section: string,
	index: number,
};

const testNames = ["commonmark-spec", "gfm-spec"] as const;

function extractSpecTests(text: string) {
	const examples: SpecTest[] = [];
	let currentSection = "";
	let index = 0;
	const tests = text.replace(/\r\n?/g, "\n").replace(/^<!-- END TESTS -->(.|[\n])*/m, "");

	for (const [, markdownSubmatch, htmlSubmatch, sectionSubmatch] of tests.matchAll(/^`{32} example\n([\s\S]*?)^\.\n([\s\S]*?)^`{32}$|^#{1,6} *(.*)$/gm)) {
		if (sectionSubmatch) {
			currentSection = sectionSubmatch;
		} else {
			index++;
			examples.push({
				markdown: markdownSubmatch,
				html: htmlSubmatch,
				section: currentSection,
				index: index,
			});
		}
	}

	return examples;
}

export const specs = testNames
	.map((name) => new URL(path.join("./fixtures", name + ".txt"), import.meta.url))
	.map((url) => readFileSync(url, "utf8"))
	.map(extractSpecTests);
