<script lang="ts">
	import { icons, osIdToSlug, getDistroIcon } from '$lib/data/distro-icons';
	import DistroIcon from '$lib/components/sessions/DistroIcon.svelte';

	let size = $state(24);

	const mappings = Object.entries(osIdToSlug);
	const allIcons = Object.entries(icons);

	const edgeCases: { label: string; osId: string | undefined }[] = [
		{ label: "'centos-stream' (unmapped → Linux fallback)", osId: 'centos-stream' },
		{ label: "'UBUNTU' (case-insensitive)", osId: 'UBUNTU' },
		{ label: 'undefined (renders nothing)', osId: undefined }
	];

	function fillFor(hex: string): string {
		return hex.toLowerCase() === '000000' ? '#f5f5f7' : `#${hex}`;
	}
</script>

<div class="toolbar">
	<h1 class="page-title">Distro Icons</h1>
	<label class="size-control">
		<span>Size: {size}px</span>
		<input type="range" min="14" max="64" bind:value={size} />
	</label>
</div>

<section>
	<h2 class="section-title">OS ID resolution ({mappings.length})</h2>
	<p class="section-desc">
		Each os-release ID rendered through the real <code>DistroIcon</code> component.
	</p>
	<div class="grid">
		{#each mappings as [osId, slug] (osId)}
			{@const icon = getDistroIcon(osId)}
			<div class="cell" title="{osId} → {slug}">
				<span class="cell-icon"><DistroIcon {osId} {size} /></span>
				<span class="cell-label">{osId}</span>
				<span class="cell-sub">{icon?.title ?? '—'}</span>
			</div>
		{/each}
	</div>
</section>

<section>
	<h2 class="section-title">Edge cases</h2>
	<div class="grid">
		{#each edgeCases as { label, osId } (label)}
			<div class="cell">
				<span class="cell-icon"><DistroIcon {osId} {size} /></span>
				<span class="cell-label wide">{label}</span>
			</div>
		{/each}
	</div>
</section>

<section>
	<h2 class="section-title">All icon data ({allIcons.length})</h2>
	<div class="grid">
		{#each allIcons as [slug, icon] (slug)}
			<div class="cell" title={icon.title}>
				<span class="cell-icon">
					<svg
						xmlns="http://www.w3.org/2000/svg"
						viewBox="0 0 24 24"
						width={size}
						height={size}
						fill={fillFor(icon.hex)}
						role="img"
						aria-label={icon.title}
					>
						<path d={icon.path} />
					</svg>
				</span>
				<span class="cell-label">{slug}</span>
				<span class="cell-sub">#{icon.hex}</span>
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
		grid-template-columns: repeat(auto-fill, minmax(120px, 1fr));
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
