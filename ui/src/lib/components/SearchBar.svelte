<script lang="ts">
	let { value = $bindable(''), placeholder = 'Search...', onsubmit }: {
		value: string;
		placeholder?: string;
		onsubmit?: () => void;
	} = $props();

	function handleKeydown(e: KeyboardEvent) {
		if (e.key === 'Enter' && onsubmit) {
			onsubmit();
		}
	}
</script>

<div class="search-bar">
	<input
		type="text"
		bind:value
		{placeholder}
		onkeydown={handleKeydown}
	/>
	{#if onsubmit}
		<button onclick={onsubmit}>Search</button>
	{/if}
</div>

<style>
	.search-bar {
		display: flex;
		gap: 8px;
	}
	input {
		flex: 1;
		background: #161616;
		border: 1px solid #333;
		color: #e0e0e0;
		padding: 8px 12px;
		border-radius: 6px;
		font-size: 0.875rem;
		outline: none;
	}
	input:focus {
		border-color: #555;
	}
	button {
		background: #333;
		color: #e0e0e0;
		border: 1px solid #444;
		padding: 8px 16px;
		border-radius: 6px;
		cursor: pointer;
		font-size: 0.875rem;
	}
	button:hover {
		background: #444;
	}
</style>
