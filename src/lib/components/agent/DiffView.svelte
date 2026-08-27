<script module lang="ts">
	export interface Props {
		/** Unified diff text (hunks with `@@ -a,b +c,d @@` headers). */
		diff: string;
	}
</script>

<script lang="ts">
	type LineKind = 'add' | 'del' | 'ctx' | 'meta';
	interface DiffLine {
		kind: LineKind;
		text: string;
	}
	interface Hunk {
		header: string;
		lines: DiffLine[];
	}

	let { diff }: Props = $props();

	function parseDiff(src: string): { meta: string[]; hunks: Hunk[] } {
		const meta: string[] = [];
		const hunks: Hunk[] = [];
		let cur: Hunk | null = null;
		for (const raw of src.replace(/\r\n/g, '\n').split('\n')) {
			if (raw.startsWith('@@')) {
				cur = { header: raw, lines: [] };
				hunks.push(cur);
				continue;
			}
			if (
				raw.startsWith('+++') ||
				raw.startsWith('---') ||
				raw.startsWith('diff ') ||
				raw.startsWith('index ') ||
				raw.startsWith('new file') ||
				raw.startsWith('deleted file')
			) {
				if (cur) cur.lines.push({ kind: 'meta', text: raw });
				else meta.push(raw);
				continue;
			}
			if (!cur) {
				if (raw.trim()) meta.push(raw);
				continue;
			}
			if (raw.startsWith('+')) cur.lines.push({ kind: 'add', text: raw.slice(1) });
			else if (raw.startsWith('-')) cur.lines.push({ kind: 'del', text: raw.slice(1) });
			else if (raw.startsWith('\\')) cur.lines.push({ kind: 'meta', text: raw });
			else cur.lines.push({ kind: 'ctx', text: raw.startsWith(' ') ? raw.slice(1) : raw });
		}
		return { meta, hunks };
	}

	let parsed = $derived(parseDiff(diff));
</script>

<div class="diff-view">
	{#each parsed.meta as line, i (i)}
		<div class="diff-line meta">{line}</div>
	{/each}
	{#each parsed.hunks as hunk, hi (hi)}
		{#if hi > 0 || parsed.meta.length > 0}
			<!-- Unchanged region between hunks collapses to a divider line. -->
			<div class="hunk-divider" aria-hidden="true"></div>
		{/if}
		<div class="hunk-header">{hunk.header}</div>
		{#each hunk.lines as line, li (li)}
			<div class="diff-line {line.kind}">
				<span class="sign"
					>{line.kind === 'add' ? '+' : line.kind === 'del' ? '-' : line.kind === 'ctx' ? ' ' : ''}</span
				><span class="text">{line.text}</span>
			</div>
		{/each}
	{/each}
</div>

<style>
	.diff-view {
		font-family: var(--font-mono);
		font-size: 0.72rem;
		line-height: 1.5;
		overflow-x: auto;
		padding: 6px 0;
	}

	.diff-line {
		display: flex;
		white-space: pre;
		padding: 0 10px;
	}

	.diff-line .sign {
		width: 12px;
		flex-shrink: 0;
		user-select: none;
	}

	.diff-line .text {
		white-space: pre-wrap;
		word-break: break-all;
		min-width: 0;
	}

	.diff-line.add {
		color: var(--color-success);
		background: color-mix(in srgb, var(--color-success) 10%, transparent);
	}

	.diff-line.del {
		color: var(--color-danger);
		background: color-mix(in srgb, var(--color-danger) 10%, transparent);
	}

	.diff-line.ctx {
		color: var(--color-text-secondary);
	}

	.diff-line.meta {
		color: var(--color-text-secondary);
		opacity: 0.6;
		padding: 0 10px;
	}

	.hunk-header {
		padding: 2px 10px;
		background: color-mix(in srgb, var(--color-text-secondary) 12%, transparent);
		color: var(--color-text-secondary);
		font-family: var(--font-mono);
		font-size: 0.68rem;
		user-select: none;
	}

	.hunk-divider {
		height: 1px;
		margin: 4px 10px;
		background: var(--color-border);
	}
</style>
