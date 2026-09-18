<script lang="ts">
	import { getContext } from 'svelte';
	import type { GetNodeNames } from '../core/lezer/wrapper';
	import { parser, type Node } from '../core/markdown';
	import { hasRawInline, renderInlineToString } from '../core/render';
	import MarkdownNode from './MarkdownNode.svelte';
	import { sanitizeHtml } from '../sanitize';

	/**
	 * Renders the inline children of a container (paragraph, heading, table
	 * cell, task). Normally each child gets its own component for fine-grained
	 * updates. When the content contains raw inline HTML (tags/comments), the
	 * children are rendered as one HTML string instead: raw tags interleave
	 * with rendered elements textually (an unclosed `<b>` must wrap the
	 * following siblings in the DOM, which separate component insertions
	 * cannot express).
	 */
	type MdNode = Node<GetNodeNames<typeof parser>>;
	let { nodes }: { nodes: MdNode[] } = $props();

	const inTable = getContext("reach-md-in-table") === true;
	const sanitize = getContext("reach-md-sanitize") !== false;
	const rawHtml = $derived.by(() => {
		if (!hasRawInline(nodes)) return null;
		const html = renderInlineToString(nodes, inTable);
		return sanitize ? sanitizeHtml(html) : html;
	});
</script>
{#if rawHtml !== null}
	{@html rawHtml}
{:else}
	{#each nodes as node (node.id)}
		<MarkdownNode node={node} />
	{/each}
{/if}
