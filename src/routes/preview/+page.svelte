<script lang="ts">
	const previews = [
		{
			href: '/preview/icons',
			title: 'Distro Icons',
			description:
				'All OS/distro SVG icons from $lib/data/distro-icons and the os-release ID → icon mapping resolution.'
		},
		{
			href: '/preview/agent',
			title: 'AI Agent Panel',
			description:
				'The real agent panel driven by an in-browser mock backend: streaming text/thinking, tool cards, approvals, queueing, branches, and error states.'
		}
	];
</script>

<h1 class="page-title">Component Previews</h1>
<p class="page-desc">
	Pure frontend preview pages — no Tauri backend required. Run <code>npm run dev</code> and open
	these routes in your browser.
</p>

<div class="card-grid">
	{#each previews as preview (preview.href)}
		<a class="card" href={preview.href}>
			<span class="card-title">{preview.title}</span>
			<span class="card-desc">{preview.description}</span>
		</a>
	{/each}
</div>

<style>
	.page-title {
		margin: 0 0 8px;
		font-size: 1.25rem;
		font-weight: 600;
	}

	.page-desc {
		margin: 0 0 24px;
		font-size: 0.8125rem;
		color: var(--color-text-secondary);
	}

	.page-desc code {
		font-family: var(--font-mono);
		font-size: 0.75rem;
		background-color: var(--color-bg-elevated);
		padding: 2px 6px;
		border-radius: 4px;
	}

	.card-grid {
		display: grid;
		grid-template-columns: repeat(auto-fill, minmax(260px, 1fr));
		gap: 16px;
	}

	.card {
		display: flex;
		flex-direction: column;
		gap: 8px;
		padding: 16px;
		background-color: var(--color-bg-secondary);
		border: 1px solid var(--color-border);
		border-radius: var(--radius-card);
		text-decoration: none;
		transition: border-color 150ms ease;
	}

	.card:hover {
		border-color: var(--color-accent);
	}

	.card-title {
		font-size: 0.875rem;
		font-weight: 600;
		color: var(--color-text-primary);
	}

	.card-desc {
		font-size: 0.75rem;
		color: var(--color-text-secondary);
		line-height: 1.5;
	}
</style>
