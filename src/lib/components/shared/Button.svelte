<script lang="ts">
	import type { Snippet } from 'svelte';

	interface Props {
		variant?: 'primary' | 'secondary' | 'ghost' | 'danger' | 'warning';
		size?: 'sm' | 'md' | 'lg';
		type?: 'button' | 'submit' | 'reset';
		disabled?: boolean;
		children: Snippet;
		onclick?: (e: MouseEvent) => void;
	}

	let {
		variant = 'primary',
		size = 'md',
		type = 'button',
		disabled = false,
		children,
		onclick
	}: Props = $props();

	const sizeClasses: Record<string, string> = {
		sm: 'px-3 py-1.5 text-xs',
		md: 'px-4 py-2 text-sm',
		lg: 'px-6 py-3 text-base'
	};
</script>

<button
	{type}
	class="btn btn-{variant} {sizeClasses[size]}"
	{disabled}
	{onclick}
>
	{@render children()}
</button>

<style>
	.btn {
		display: inline-flex;
		align-items: center;
		justify-content: center;
		gap: 0.5rem;
		border-radius: var(--radius-btn);
		font-family: var(--font-sans);
		font-weight: 500;
		border: none;
		cursor: pointer;
		transition:
			background-color var(--duration-default) var(--ease-default),
			opacity var(--duration-default) var(--ease-default),
			transform var(--duration-default) var(--ease-default),
			box-shadow var(--duration-default) var(--ease-default);
		user-select: none;
		white-space: nowrap;
		position: relative;
		overflow: hidden;
	}
	.btn::before {
		opacity: 0;
		transition: opacity var(--duration-default) var(--ease-default);
		pointer-events: none;
		position: absolute;
		top: 0;
		left: 0;
		display: block;
		width: 100%;
		height: 100%;
	}

	.btn:active:not(:disabled) {
		transform: scale(0.97);
	}

	.btn:disabled {
		opacity: 0.4;
		cursor: not-allowed;
	}

	/* Primary */
	.btn-primary {
		background-color: var(--color-accent);
		color: #fff;
	}
	.btn-primary:hover:not(:disabled) {
		background-color: var(--color-accent-hover);
	}

	/* Secondary */
	.btn-secondary {
		background-color: var(--color-bg-elevated);
		color: var(--color-text-primary);
		border: 1px solid var(--color-border);
	}
	.btn-secondary:hover:not(:disabled) {
		background-color: rgba(255, 255, 255, 0.08);
		background-color: color-mix(in srgb, var(--color-text-primary) 8%, transparent);
	}

	/* Ghost */
	.btn-ghost {
		background-color: transparent;
		color: var(--color-text-primary);
	}
	.btn-ghost:hover:not(:disabled) {
		background-color: rgba(255, 255, 255, 0.06);
		background-color: color-mix(in srgb, var(--color-text-primary) 6%, transparent);
	}

	/* Danger */
	.btn-danger {
		background-color: var(--color-danger);
		color: #fff;
	}
	.btn-danger:hover:not(:disabled) {
		background-color: #ff6961;
	}

	/* Warning */
	.btn-warning {
		background-color: var(--color-bg-elevated);
		color: var(--color-warning);
		border: 1px solid var(--color-warning);
	}
	.btn-warning::before {
		content: "";
		background-color: var(--color-warning);
	}
	.btn-warning:hover:not(:disabled)::before {
		opacity: 0.08;
	}
</style>
