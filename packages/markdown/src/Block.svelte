<script lang="ts" generics="N extends string">
	import type { Slice } from './core/markdown';
	import Inline from './Inline.svelte';
	import Block from './Block.svelte';
    import { type TypedSyntaxNode } from './core/lezer/wrapper';

	let { node, slice }: { node: TypedSyntaxNode<N>; slice: Slice } = $props();

	const name = $derived(node.name);

	// Block children minus delimiter tokens (QuoteMark, ListMark, ...).
	const children = $derived.by(() => {
		const out: TypedSyntaxNode<N>[] = [];
		for (let c = node.firstChild; c; c = c.nextSibling) {
			if (!c.name.endsWith('Mark') && c.name !== 'CodeInfo') out.push(c);
		}
		return out;
	});

	const headingLevel = $derived(name.match(/^(?:ATX|Setext)Heading(\d)$/)?.[1]);

	const codeText = $derived.by(() => {
		for (let c = node.firstChild; c; c = c.nextSibling) {
			if (c.name === 'CodeText') return slice(c.from, c.to);
		}
		// Indented CodeBlock has no CodeText child; take the raw range.
		// TODO: strip the 4-space indent per line.
		return slice(node.from, node.to);
	});
</script>

{#if name === "Paragraph"}
	<p><Inline {node} {slice} /></p>
{:else if headingLevel}
	<svelte:element this={'h' + headingLevel}><Inline {node} {slice} /></svelte:element>
{:else if name === 'FencedCode' || name === 'CodeBlock'}
	<pre><code>{codeText}</code></pre>
{:else if name === 'Blockquote'}
	<blockquote>
		{#each children as child (child.from)}
			<Block node={child} {slice} />
		{/each}
	</blockquote>
{:else if name === 'BulletList'}
	<ul>
		{#each children as child (child.from)}
			<Block node={child} {slice} />
		{/each}
	</ul>
{:else if name === 'OrderedList'}
	<ol>
		{#each children as child (child.from)}
			<Block node={child} {slice} />
		{/each}
	</ol>
{:else if name === 'ListItem'}
	<!-- TODO: tight lists should suppress the inner <p>; TaskMarker checkbox -->
	<li>
		{#each children as child (child.from)}
			<Block node={child} {slice} />
		{/each}
	</li>
{:else if name === 'HorizontalRule'}
	<hr>
{:else}
	<!-- Fallback for unhandled blocks (Table, HTMLBlock, ...): raw text, auto-escaped -->
	{slice(node.from, node.to)}
{/if}
