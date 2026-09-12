<script lang="ts">
	import { providerIcons } from '$lib/data/provider-icons';
	import ProviderIcon from '$lib/components/agent/ProviderIcon.svelte';

	let size = $state(24);

	const allIcons = Object.entries(providerIcons);

	// Provider icons are monochrome and rendered with fill="currentColor", so
	// these contexts mirror every place they appear in the composer.
	const colorContexts = [
		{ label: 'Menu item text (default)', color: 'var(--color-text-primary)' },
		{ label: 'Model button text', color: 'var(--color-text-secondary)' },
		{ label: 'Selected menu item (accent)', color: 'var(--color-accent)' }
	];

	const edgeCases: { label: string; preset: string | undefined }[] = [
		{ label: "'anthropic' (unknown preset, renders nothing)", preset: 'anthropic' },
		{ label: 'undefined (renders nothing)', preset: undefined }
	];
</script>

<div class="toolbar">
	<h1 class="page-title">Provider Icons</h1>
	<label class="size-control">
		<span>Size: {size}px</span>
		<input type="range" min="12" max="64" bind:value={size} />
	</label>
</div>

<section>
	<h2 class="section-title">All presets ({allIcons.length})</h2>
	<p class="section-desc">
		Each preset id from <code>$lib/data/provider-icons</code> rendered through the real
		<code>ProviderIcon</code> component.
	</p>
	<div class="grid">
		{#each allIcons as [preset, icon] (preset)}
			<div class="cell" title={preset}>
				<span class="cell-icon"><ProviderIcon {preset} {size} /></span>
				<span class="cell-label">{preset}</span>
				<span class="cell-sub">{icon.viewBox} · {icon.paths.length} path{icon.paths.length === 1 ? '' : 's'}</span>
			</div>
		{/each}
	</div>
</section>

<section>
	<h2 class="section-title">Color contexts (currentColor)</h2>
	<p class="section-desc">Icons inherit the surrounding text color, matching their composer usage.</p>
	{#each colorContexts as ctx (ctx.label)}
		<h3 class="context-title">
			{ctx.label} <span class="context-color">{ctx.color}</span>
		</h3>
		<div class="grid context-grid">
			{#each allIcons as [preset] (preset)}
				<div class="cell">
					<span class="cell-icon" style:color={ctx.color}><ProviderIcon {preset} {size} /></span>
					<span class="cell-label">{preset}</span>
				</div>
			{/each}
		</div>
	{/each}
</section>

<section>
	<h2 class="section-title">Edge cases</h2>
	<div class="grid">
		{#each edgeCases as { label, preset } (label)}
			<div class="cell">
				<span class="cell-icon"><ProviderIcon {preset} {size} /></span>
				<span class="cell-label wide">{label}</span>
			</div>
		{/each}
	</div>
</section>

<style>
	.toolbar {
		display: flex;
		align-items: center;
		justify-content: space-between;
		margin-bottom: 24px;
	}

	.page-title {
		margin: 0;
		font-size: 1.25rem;
		font-weight: 600;
	}

	.size-control {
		display: flex;
		align-items: center;
		gap: 12px;
		font-size: 0.8125rem;
		color: var(--color-text-secondary);
	}

	.size-control input {
		accent-color: var(--color-accent);
	}

	section {
		margin-bottom: 32px;
	}

	.section-title {
		margin: 0 0 4px;
		font-size: 0.9375rem;
		font-weight: 600;
	}

	.section-desc {
		margin: 0 0 16px;
		font-size: 0.75rem;
		color: var(--color-text-secondary);
	}

	.section-desc code {
		font-family: var(--font-mono);
		background-color: var(--color-bg-elevated);
		padding: 1px 5px;
		border-radius: 4px;
	}

	.grid {
		display: grid;
		grid-template-columns: repeat(auto-fill, minmax(160px, 1fr));
		gap: 12px;
	}

	.cell {
		display: flex;
		flex-direction: column;
		align-items: center;
		gap: 8px;
		padding: 16px 8px 12px;
		background-color: var(--color-bg-secondary);
		border: 1px solid var(--color-border);
		border-radius: var(--radius-card);
	}

	.cell-icon {
		display: flex;
		align-items: center;
		justify-content: center;
		min-height: 64px;
	}

	.context-title {
		margin: 0 0 10px;
		font-size: 0.8125rem;
		font-weight: 600;
		color: var(--color-text-primary);
	}

	.context-color {
		font-weight: 400;
		font-family: var(--font-mono);
		font-size: 0.6875rem;
		color: var(--color-text-secondary);
	}

	.context-grid {
		margin-bottom: 20px;
	}

	.cell-label {
		font-size: 0.75rem;
		font-family: var(--font-mono);
		color: var(--color-text-primary);
		text-align: center;
		word-break: break-all;
	}

	.cell-label.wide {
		font-family: var(--font-sans);
		color: var(--color-text-secondary);
	}

	.cell-sub {
		font-size: 0.6875rem;
		color: var(--color-text-secondary);
	}
</style>
