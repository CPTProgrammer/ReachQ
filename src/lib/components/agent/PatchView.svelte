<script module lang="ts">
	import type { DiffPreview } from '$lib/ipc/agent';

	export interface Props {
		diff: DiffPreview;
	}
</script>

<script lang="ts">
	import { diffWordsWithSpace } from 'diff';
	import { t } from '$lib/state/i18n.svelte';

	let { diff }: Props = $props();

	interface Segment {
		text: string;
		hot: boolean;
	}
	interface Row {
		kind: 'add' | 'del' | 'ctx';
		segments: Segment[];
	}
	type Item =
		| { type: 'gap'; key: string; count: number }
		| { type: 'row'; key: string; row: Row };

	/**
	 * Flatten the preview into renderable rows. Each run of deleted lines plus
	 * the added lines right after it forms a change block; paired lines (in
	 * order, min(n,m) pairs) get word-level highlights — unless the pair's
	 * non-whitespace similarity (2*common/(len1+len2)) is below 0.25, which
	 * is too noisy to highlight.
	 */
	function buildItems(d: DiffPreview): Item[] {
		const items: Item[] = [];
		d.hunks.forEach((hunk, hi) => {
			if (hi > 0) {
				const prev = d.hunks[hi - 1];
				const prevOldEnd = prev.oldStart + prev.lines.filter((l) => l.kind !== 'add').length;
				const gap = hunk.oldStart - prevOldEnd;
				if (gap > 0) items.push({ type: 'gap', key: `g${hi}`, count: gap });
			}
			const lines = hunk.lines;
			const inline: (Segment[] | null)[] = lines.map(() => null);
			let i = 0;
			while (i < lines.length) {
				if (lines[i].kind !== 'del') {
					i++;
					continue;
				}
				let j = i;
				while (j < lines.length && lines[j].kind === 'del') j++;
				let k = j;
				while (k < lines.length && lines[k].kind === 'add') k++;
				const pairs = Math.min(j - i, k - j);
				for (let p = 0; p < pairs; p++) {
					const oldText = lines[i + p].text;
					const newText = lines[j + p].text;
					const changes = diffWordsWithSpace(oldText, newText);
					// Gate on non-whitespace similarity only — indentation and
					// inter-word spaces would otherwise inflate the score and let
					// unrelated line pairs through.
					let common = 0;
					for (const c of changes)
						if (!c.added && !c.removed) common += c.value.replace(/\s/g, '').length;
					const oldLen = oldText.replace(/\s/g, '').length;
					const newLen = newText.replace(/\s/g, '').length;
					const ratio = oldLen + newLen === 0 ? 0 : (2 * common) / (oldLen + newLen);
					if (ratio < 0.25) continue;
					inline[i + p] = coalesceFragments(
						changes.filter((c) => !c.added).map((c) => ({ text: c.value, hot: !!c.removed }))
					);
					inline[j + p] = coalesceFragments(
						changes.filter((c) => !c.removed).map((c) => ({ text: c.value, hot: !!c.added }))
					);
				}
				i = k;
			}
			lines.forEach((line, li) => {
				// Keyed by line number so hunk reshuffles move DOM nodes instead of
				// rewriting them; the x-prefix fallback covers missing numbers.
				const fallback = `x${hi}-${li}`;
				const key =
					line.kind === 'add' ? `n${line.newNo ?? fallback}` : `o${line.oldNo ?? fallback}`;
				items.push({
					type: 'row',
					key,
					row: { kind: line.kind, segments: inline[li] ?? [{ text: line.text, hot: false }] }
				});
			});
		});
		return items;
	}

	/**
	 * Merge fragmentary highlights: a whitespace-only cold segment sandwiched
	 * between two hot segments belongs to the same logical edit, so flip it hot
	 * and coalesce adjacent same-hot runs into one block. Whitespace at the
	 * edges stays cold, which keeps shared indentation and trailing punctuation
	 * unhighlighted.
	 */
	function coalesceFragments(segments: Segment[]): Segment[] {
		const flipped = segments.map((s, i) =>
			!s.hot && /^\s+$/.test(s.text) && segments[i - 1]?.hot && segments[i + 1]?.hot
				? { text: s.text, hot: true }
				: s
		);
		const merged: Segment[] = [];
		for (const s of flipped) {
			const last = merged[merged.length - 1];
			if (last && last.hot === s.hot) last.text += s.text;
			else merged.push({ ...s });
		}
		return merged;
	}

	let items = $derived(buildItems(diff));
</script>

<div class="patch-view">
	{#each items as item (item.key)}
		{#if item.type === 'gap'}
			<div class="hunk-gap">
				<span class="gap-ellipsis" aria-hidden="true">⋯</span>{t('agent.hunk_collapsed', {
					count: item.count
				})}
			</div>
		{:else}
			<div class="patch-line {item.row.kind}">{#each item.row.segments as segment, si (si)}{#if segment.hot}<span class="hot">{segment.text}</span>{:else}{segment.text}{/if}{/each}</div>
		{/if}
	{/each}
</div>

<style>
	.patch-view {
		padding: 6px 0;
		font-family: var(--font-content-mono);
		font-size: 0.72rem;
		line-height: 1.5;
		color: var(--color-text-primary);
		white-space: pre-wrap;
		word-break: break-all;
		user-select: text;
	}

	.patch-line {
		padding: 0 6px;
		min-height: calc(1em * 1.5);
	}

	.patch-line.del {
		background: color-mix(in srgb, var(--color-danger) 10%, transparent);
	}

	.patch-line.add {
		background: color-mix(in srgb, var(--color-success) 10%, transparent);
	}

	.patch-line.del .hot {
		background: color-mix(in srgb, var(--color-danger) 28%, transparent);
	}

	.patch-line.add .hot {
		background: color-mix(in srgb, var(--color-success) 28%, transparent);
	}

	.hunk-gap {
		display: flex;
		align-items: center;
		gap: 6px;
		padding: 2px 10px;
		background: color-mix(in srgb, var(--color-text-secondary) 8%, transparent);
		color: var(--color-text-secondary);
		font-size: 0.68rem;
		user-select: none;
	}
</style>
