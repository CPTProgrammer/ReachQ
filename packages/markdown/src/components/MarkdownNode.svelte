<script lang="ts">
	import { decodeHTMLStrict } from "entities";
    import type { GetNodeNames } from '../core/lezer/wrapper';
    import { parser, type Node } from '../core/markdown';
    import MarkdownNodeList from './MarkdownNodeList.svelte';

	let { node }: { node: Node<GetNodeNames<typeof parser>> } = $props();

	function inlineCodeTrim(str: string): string {
		if (str.startsWith(" ") && str.endsWith(" ") && /[^ ]/.test(str)) {
			return str.slice(1, -1);
		}
		return str;
	}

	const ESCAPE_RE = /\\([!"#$%&'()*+,\-./:;<=>?@\[\\\]^_`{|}~])/g;
	const unescapeBackslash = (str: string) => str.replace(ESCAPE_RE, "$1");

	function decodeEntity(raw: string): string | null {
		let m = /^&#(\d{1,7});$/.exec(raw);
		if (m) return sanitizeCodePoint(parseInt(m[1], 10));
		m = /^&#[xX]([0-9a-fA-F]{1,6});$/.exec(raw);
		if (m) return sanitizeCodePoint(parseInt(m[1], 16));
		if (raw.startsWith("&#")) return null;

		const decoded = decodeHTMLStrict(raw);
		return decoded === raw ? null : decoded;
	}

	function sanitizeCodePoint(cp: number): string {
		if (cp === 0 || cp > 0x10ffff || (cp >= 0xd800 && cp <= 0xdfff)) return "\uFFFD";
		return String.fromCodePoint(cp);
	}
</script>
{#if node.name === "Text"}
	<!-- {node.content?.split("\n").map(str => str.trim()).join("\n")} -->
	{node.content?.replace(/[ \t]*\n[ \t]*/g, "\n")}
{:else if node.name === "URL"}
	{node.content}
{:else if node.name === "Entity"}
	{decodeEntity(node.content!) ?? node.content}
{:else if node.name === "Paragraph"}
	<p><MarkdownNodeList nodes={node.children} /></p>
{:else if node.name === "Escape"}
	{node.content![1]}
{:else if node.name === "StrongEmphasis"}
	<strong><MarkdownNodeList nodes={node.children} /></strong>
{:else if node.name === "Emphasis"}
	<em><MarkdownNodeList nodes={node.children} /></em>
{:else if node.name === "Strikethrough"}
	<del><MarkdownNodeList nodes={node.children} /></del>
{:else if node.name === "InlineCode"}
	{@const codeRawText = node.children.filter(n => n.name === "Text")[0].content!}
	{@const codeText = inlineCodeTrim(codeRawText.replaceAll("\n", " "))}
	<code>{codeText}</code>
{:else if node.name === "CodeBlock" || node.name === "FencedCode"}
	{@const codeContent = node.children.filter(n => n.name === "CodeText").map(n => n.content).join("")}
	{@const language = node.children.filter(n => n.name === "CodeInfo")[0]?.content?.split(" ")[0]}
	<pre><code class={language ? `language-${unescapeBackslash(language)}` : null}>{codeContent === "" ? "" : codeContent + "\n"}</code></pre>
{:else if node.name === "HardBreak"}
	<br/>
{:else if node.name === "HorizontalRule"}
	<hr/>
{:else if node.name === "ATXHeading1"}
	<h1><MarkdownNodeList nodes={node.children} /></h1>
{:else if node.name === "ATXHeading2"}
	<h2><MarkdownNodeList nodes={node.children} /></h2>
{:else if node.name === "ATXHeading3"}
	<h3><MarkdownNodeList nodes={node.children} /></h3>
{:else if node.name === "ATXHeading4"}
	<h4><MarkdownNodeList nodes={node.children} /></h4>
{:else if node.name === "ATXHeading5"}
	<h5><MarkdownNodeList nodes={node.children} /></h5>
{:else if node.name === "ATXHeading6"}
	<h6><MarkdownNodeList nodes={node.children} /></h6>
{:else if node.name === "SetextHeading1"}
	<h1><MarkdownNodeList nodes={node.children} /></h1>
{:else if node.name === "SetextHeading2"}
	<h2><MarkdownNodeList nodes={node.children} /></h2>
{:else if node.name === "Blockquote"}
	<blockquote><MarkdownNodeList nodes={node.children} /></blockquote>
{:else if node.name === "BulletList"}
	<ul><MarkdownNodeList nodes={node.children} /></ul>
{:else if node.name === "OrderedList"}
	<ol><MarkdownNodeList nodes={node.children} /></ol>
{:else if node.name === "ListItem"}
	<li><MarkdownNodeList nodes={node.children} /></li>
{:else if node.name === "Autolink"}
	{@const url = node.children.filter(n => n.name === "URL")[0]?.content}
	<a href={encodeURI(url!)}>{url}</a>
{:else if node.name === "HTMLBlock" || node.name === "HTMLTag"}
	{@html node.content}
{:else if node.name === "Link"}
	{@const text = node.children.filter(n => n.name === "Text")[0]?.content}
	{@const url = node.children.filter(n => n.name === "URL")[0]?.content}
	{@const title = node.children.filter(n => n.name === "LinkTitle")[0]?.content}
	<a href={url !== undefined ? encodeURI(unescapeBackslash(url)) : null}
		title={title !== undefined ? unescapeBackslash(title.slice(1, -1)) : null}
	>{text}</a>
<!-- {:else if node.children.length}
	<MarkdownNodeList nodes={node.children} />
{:else}
	{node.content} -->
{/if}
