<script lang="ts">
	let { label, value, sub = '' }: {
		label: string;
		value: string | number;
		sub?: string;
	} = $props();
</script>

<div class="stat-card">
	<div class="label">{label}</div>
	<div class="value">{value}</div>
	{#if sub}
		<div class="sub">{sub}</div>
	{/if}
</div>

<style>
	.stat-card {
		background: #161616;
		border: 1px solid #222;
		border-radius: 8px;
		padding: 20px;
		min-width: 140px;
	}
	.label {
		font-size: 0.75rem;
		color: #888;
		text-transform: uppercase;
		letter-spacing: 0.05em;
		margin-bottom: 8px;
	}
	.value {
		font-size: 1.75rem;
		font-weight: 600;
		color: #fff;
	}
	.sub {
		font-size: 0.75rem;
		color: #666;
		margin-top: 4px;
	}
</style>
