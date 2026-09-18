<script lang="ts">
	import { getContext, setContext, untrack } from 'svelte';
	import type { GetNodeNames } from '../core/lezer/wrapper';
	import { parser, type Node } from '../core/markdown';
	import {
		autolinkHref, bareAutolinkHref, codeBlockLanguage, codeBlockText, codeSpanText,
		decodeEntity, hasDest, linkAttrs, orderedListStart, plainText, renderRawBlock, unescapeString,
		tableParts, rowCells, taskChecked, taskChildren, textChildren, textContent,
		filterDisallowedTags, sanitizeUrl, type AnyNode,
	} from '../core/render';
	import { sanitizeHtml } from '../sanitize';
	import InlineContent from './InlineContent.svelte';
	import MarkdownNodeList from './MarkdownNodeList.svelte';
	import MarkdownNode from './MarkdownNode.svelte';

	let { node }: { node: Node<GetNodeNames<typeof parser>> } = $props();

	const HEADING_TAGS: Record<string, string> = {
		ATXHeading1: "h1", ATXHeading2: "h2", ATXHeading3: "h3",
		ATXHeading4: "h4", ATXHeading5: "h5", ATXHeading6: "h6",
		SetextHeading1: "h1", SetextHeading2: "h2",
	};

	/** Structural tokens that never produce output on their own. */
	function isSilent(name: string): boolean {
		return name.endsWith("Mark") || name === "CodeInfo" || name === "CodeText" ||
			name === "LinkLabel" || name === "LinkReference" || name === "TableHeader" ||
			name === "TableRow" || name === "TableDelimiter";
	}

	/** alt text of an image: its description rendered as plain text. */
	function imageAlt(node: AnyNode): string {
		let out = "";
		for (const child of textChildren(node)) out += plainText(child);
		return out;
	}

	const inTable = getContext("reach-md-in-table") === true;
	const sanitize = getContext("reach-md-sanitize") !== false;
	/** URL scheme policy, off in sanitize=false (spec-conformance) mode. */
	const hrefUrl = (url: string, isImage = false) => sanitize ? sanitizeUrl(url, isImage) : url;
	// GFM resolves `\|` escapes even inside code spans in table cells. Node
	// identity is stable per component instance, so the name check is init-only.
	if (untrack(() => node.name === "Table")) setContext("reach-md-in-table", true);
</script>
{#if isSilent(node.name)}
	<!-- structural token: no output -->
{:else if node.name === "Text"}
	{textContent(node)}
{:else if node.name === "Entity"}
	{decodeEntity(node.content!) ?? node.content}
{:else if node.name === "Escape"}
	{node.content?.[1]}
{:else if node.name === "URL"}
	<!-- GFM autolink extension: bare www./scheme/email URL -->
	<a href={hrefUrl(bareAutolinkHref(node.content ?? ""))}>{node.content}</a>
{:else if node.name === "Paragraph"}
	{#if node.children.length}
		<p><InlineContent nodes={node.children} /></p>
	{/if}
{:else if node.name === "StrongEmphasis"}
	<strong><InlineContent nodes={node.children} /></strong>
{:else if node.name === "Emphasis"}
	<em><InlineContent nodes={node.children} /></em>
{:else if node.name === "Strikethrough"}
	<del><InlineContent nodes={node.children} /></del>
{:else if node.name === "InlineCode"}
	<code>{codeSpanText(node, inTable)}</code>
{:else if node.name === "CodeBlock" || node.name === "FencedCode"}
	{@const language = codeBlockLanguage(node)}
	<pre><code class={language ? `language-${language}` : null}>{codeBlockText(node)}</code></pre>
{:else if node.name === "HardBreak"}
	<br/>
{:else if node.name === "HorizontalRule"}
	<hr/>
{:else if HEADING_TAGS[node.name]}
	<svelte:element this={HEADING_TAGS[node.name]}><InlineContent nodes={node.children} /></svelte:element>
{:else if node.name === "Blockquote"}
	<blockquote><MarkdownNodeList nodes={node.children} /></blockquote>
{:else if node.name === "BulletList"}
	<ul><MarkdownNodeList nodes={node.children} /></ul>
{:else if node.name === "OrderedList"}
	{@const start = orderedListStart(node)}
	<ol start={start !== null && start !== 1 ? start : null}><MarkdownNodeList nodes={node.children} /></ol>
{:else if node.name === "ListItem"}
	<li><MarkdownNodeList nodes={node.children} /></li>
{:else if node.name === "Task"}
	{#if taskChecked(node)}<input checked disabled type="checkbox" />{:else}<input disabled type="checkbox" />{/if}<InlineContent nodes={taskChildren(node)} />
{:else if node.name === "Table"}
	{@const parts = tableParts(node)}
	<table>
		<thead>
			<tr>
				{#each parts.headerCells as cell, i (cell.id)}
					<th align={parts.aligns[i] ?? null}><InlineContent nodes={cell.children} /></th>
				{/each}
			</tr>
		</thead>
		{#if parts.rows.length}
			<tbody>
				{#each parts.rows as row (row.id)}
					{@const cells = rowCells(row, parts.colCount)}
					<tr>
						{#each cells as cell, i (cell ? cell.id : -i - 1)}
							<td align={parts.aligns[i] ?? null}>{#if cell}<InlineContent nodes={cell.children} />{/if}</td>
						{/each}
					</tr>
				{/each}
			</tbody>
		{/if}
	</table>
{:else if node.name === "Autolink"}
	{@const url = node.children.find((c) => c.name === "URL")?.content ?? ""}
	<a href={hrefUrl(autolinkHref(url))}>{url}</a>
{:else if node.name === "HTMLBlock" || node.name === "CommentBlock" || node.name === "ProcessingInstructionBlock"}
	<!-- reached only for balanced raw blocks (unbalanced ones join a run in MarkdownNodeList) -->
	{@html sanitize ? sanitizeHtml(renderRawBlock(node)) : renderRawBlock(node)}
{:else if node.name === "HTMLTag" || node.name === "Comment" || node.name === "ProcessingInstruction"}
	<!-- normally unreachable: containers with raw inline HTML render via InlineContent -->
	{@html sanitize ? sanitizeHtml(filterDisallowedTags(node.content ?? "")) : filterDisallowedTags(node.content ?? "")}
{:else if node.name === "Link"}
	{#if hasDest(node)}
		{@const attrs = linkAttrs(node)}
		<a href={hrefUrl(attrs.url ?? "")} title={attrs.title || null}>{#each textChildren(node) as child (child.id)}<MarkdownNode node={child} />{/each}</a>
	{:else}
		<!-- no usable destination: render the brackets literally; the label renders as ordinary inline text (escapes/entities resolved) -->
		{#each node.children as child (child.id)}
			{#if child.name === "LinkLabel"}
				{unescapeString(child.content ?? "")}
			{:else if child.name === "LinkMark"}
				{child.content}
			{:else}
				<MarkdownNode node={child} />
			{/if}
		{/each}
	{/if}
{:else if node.name === "Image"}
	{#if hasDest(node)}
		{@const attrs = linkAttrs(node)}
		<img src={hrefUrl(attrs.url ?? "", true)} alt={imageAlt(node)} title={attrs.title || null} />
	{:else}
		{#each node.children as child (child.id)}
			{#if child.name === "LinkLabel"}
				{unescapeString(child.content ?? "")}
			{:else if child.name === "LinkMark"}
				{child.content}
			{:else}
				<MarkdownNode node={child} />
			{/if}
		{/each}
	{/if}
{:else if node.children.length}
	<MarkdownNodeList nodes={node.children} />
{:else}
	{node.content}
{/if}
