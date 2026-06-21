<script lang="ts">
	interface DropdownOption {
		label: string;
		value: string;
	}

	interface Props {
		options: DropdownOption[];
		selected?: string;
		placeholder?: string;
		disabled?: boolean;
		onchange?: (value: string) => void;
	}

	let {
		options,
		selected = $bindable(''),
		placeholder = 'Select...',
		disabled = false,
		onchange
	}: Props = $props();

	let isOpen = $state(false);
	let dropUp = $state(false);
	let dropdownEl: HTMLDivElement | undefined = $state();

	let selectedLabel = $derived(
		options.find((o) => o.value === selected)?.label ?? placeholder
	);

	function getScrollableContainer(el: HTMLElement): HTMLElement | null {
		let p: HTMLElement | null = el.parentElement;
		while (p) {
			const s = getComputedStyle(p);
			if ((s.overflowY === 'auto' || s.overflowY === 'scroll') && p.scrollHeight > p.clientHeight) {
				return p;
			}
			p = p.parentElement;
		}
		return null;
	}

	function toggleOpen() {
		if (!isOpen && dropdownEl) {
			const spaceBelow = window.innerHeight - dropdownEl.getBoundingClientRect().bottom;
			// const scrollable = getScrollableContainer(dropdownEl);
			// console.log(scrollable && (scrollable.getBoundingClientRect().bottom - dropdownEl.getBoundingClientRect().bottom));
			dropUp = spaceBelow < 220;
		}
		isOpen = !isOpen;
	}

	function selectOption(option: DropdownOption) {
		selected = option.value;
		isOpen = false;
		onchange?.(option.value);
	}

	function onKeydown(e: KeyboardEvent) {
		if (e.key === 'Escape') {
			isOpen = false;
		}
	}

	$effect(() => {
		if (!isOpen) return;

		function handleClickOutside(e: MouseEvent) {
			if (dropdownEl && !dropdownEl.contains(e.target as Node)) {
				isOpen = false;
			}
		}

		document.addEventListener('click', handleClickOutside, true);

		return () => {
			document.removeEventListener('click', handleClickOutside, true);
		};
	});
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="dropdown" bind:this={dropdownEl} onkeydown={onKeydown}>
	<button
		class="dropdown-trigger"
		class:open={isOpen}
		class:has-value={!!selected}
		onclick={toggleOpen}
		aria-haspopup="listbox"
		aria-expanded={isOpen}
		{disabled}
	>
		<span class="dropdown-text">{selectedLabel}</span>
		<svg class="dropdown-chevron" class:open={isOpen} width="12" height="12" viewBox="0 0 12 12" fill="none">
			<path d="M3 4.5L6 7.5L9 4.5" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" />
		</svg>
	</button>

	{#if isOpen}
		<ul class="dropdown-list" class:drop-up={dropUp} role="listbox">
			{#each options as option (option.value)}
				<li
					class="dropdown-item"
					class:selected={option.value === selected}
					role="option"
					aria-selected={option.value === selected}
				>
					<button class="dropdown-item-btn" onclick={() => selectOption(option)}>
						<span>{option.label}</span>
						{#if option.value === selected}
							<svg width="14" height="14" viewBox="0 0 14 14" fill="none">
								<path d="M2 7L5.5 10.5L12 3.5" stroke="var(--color-accent)" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" />
							</svg>
						{/if}
					</button>
				</li>
			{/each}
		</ul>
	{/if}
</div>

<style>
	.dropdown {
		position: relative;
		width: 100%;
	}

	.dropdown-trigger {
		display: flex;
		align-items: center;
		justify-content: space-between;
		width: 100%;
		padding: 10px 12px;
		font-family: var(--font-sans);
		font-size: 0.875rem;
		color: var(--color-text-secondary);
		background-color: var(--color-bg-elevated);
		border: 1px solid var(--color-border);
		border-radius: var(--radius-btn);
		cursor: pointer;
		transition:
			border-color var(--duration-default) var(--ease-default),
			box-shadow var(--duration-default) var(--ease-default);
	}

	.dropdown-trigger.has-value {
		color: var(--color-text-primary);
	}

	.dropdown-trigger.open {
		border-color: var(--color-accent);
		box-shadow: 0 0 0 1px var(--color-accent);
	}

	.dropdown-trigger:disabled {
		opacity: 0.4;
		cursor: not-allowed;
	}

	.dropdown-text {
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.dropdown-chevron {
		flex-shrink: 0;
		color: var(--color-text-secondary);
		transition: transform var(--duration-default) var(--ease-default);
	}

	.dropdown-chevron.open {
		transform: rotate(180deg);
	}

	.dropdown-list {
		position: absolute;
		top: calc(100% + 4px);
		left: 0;
		right: 0;
		z-index: 50;
		margin: 0;
		padding: 4px;
		list-style: none;
		background-color: var(--color-bg-elevated);
		border: 1px solid var(--color-border);
		border-radius: var(--radius-btn);
		box-shadow: var(--shadow-elevated);
		max-height: 200px;
		overflow-y: auto;
		animation: dropdownIn var(--duration-default) var(--ease-default);
	}

	.dropdown-item {
		margin: 0;
	}

	.dropdown-item-btn {
		display: flex;
		align-items: center;
		justify-content: space-between;
		width: 100%;
		padding: 8px 10px;
		font-family: var(--font-sans);
		font-size: 0.875rem;
		color: var(--color-text-primary);
		background: transparent;
		border: none;
		border-radius: 4px;
		cursor: pointer;
		transition: background-color var(--duration-default) var(--ease-default);
	}

	.dropdown-item-btn:hover {
		background-color: rgba(255, 255, 255, 0.06);
	}

	.dropdown-item.selected .dropdown-item-btn {
		color: var(--color-accent);
	}

	@keyframes dropdownIn {
		from {
			opacity: 0;
			transform: translateY(-4px);
		}
		to {
			opacity: 1;
			transform: translateY(0);
		}
	}

	@keyframes dropdownInUp {
		from {
			opacity: 0;
			transform: translateY(4px);
		}
		to {
			opacity: 1;
			transform: translateY(0);
		}
	}

	.dropdown-list.drop-up {
		top: auto;
		bottom: calc(100% + 4px);
		animation: dropdownInUp var(--duration-default) var(--ease-default);
	}
</style>
