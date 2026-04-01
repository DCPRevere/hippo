<script lang="ts">
	import { features } from '$lib/features';
	import { getMetrics, getUsage } from '$lib/api';
	import StatCard from '$lib/components/StatCard.svelte';
	import type { UsagePeriod } from '$lib/types';

	let metricsText = $state('');
	let parsedMetrics = $state<Record<string, number>>({});
	let cloudUsage = $state<UsagePeriod | null>(null);
	let loading = $state(true);
	let error = $state('');

	$effect(() => {
		loadData();
	});

	async function loadData() {
		loading = true;
		error = '';
		try {
			if (features.mode === 'cloud' && features.features.tenantManagement) {
				// In cloud mode, use the tenant usage endpoint
				// tenant ID would come from session/context
				const tenantId = localStorage.getItem('hippo_tenant_id') || '';
				if (tenantId) {
					cloudUsage = await getUsage(tenantId);
				}
			} else {
				metricsText = await getMetrics();
				parsedMetrics = parsePrometheus(metricsText);
			}
		} catch (e) {
			error = e instanceof Error ? e.message : String(e);
		} finally {
			loading = false;
		}
	}

	function parsePrometheus(text: string): Record<string, number> {
		const result: Record<string, number> = {};
		for (const line of text.split('\n')) {
			if (line.startsWith('#') || !line.trim()) continue;
			const match = line.match(/^(\S+)\s+(\S+)/);
			if (match) {
				const val = parseFloat(match[2]);
				if (!isNaN(val)) result[match[1]] = val;
			}
		}
		return result;
	}
</script>

<div class="page">
	<h1>Usage</h1>

	{#if loading}
		<p class="muted">Loading metrics...</p>
	{:else if error}
		<p class="error">{error}</p>
	{:else if features.mode === 'cloud' && cloudUsage}
		<div class="stats-row">
			<StatCard label="Remember Calls" value={cloudUsage.remember_calls} />
			<StatCard label="Context Calls" value={cloudUsage.context_calls} />
			<StatCard label="Facts Stored" value={cloudUsage.facts_stored} />
			<StatCard label="Entities Stored" value={cloudUsage.entities_stored} />
		</div>
		<p class="period">Period start: {new Date(cloudUsage.period_start).toLocaleDateString()}</p>
	{:else}
		<div class="stats-row">
			{#each Object.entries(parsedMetrics) as [name, value]}
				<StatCard label={name} value={typeof value === 'number' && value % 1 !== 0 ? value.toFixed(2) : value} />
			{/each}
		</div>

		{#if Object.keys(parsedMetrics).length === 0}
			<p class="muted">No metrics available</p>
		{/if}

		<div class="raw-section">
			<h2>Raw Metrics</h2>
			<pre class="raw-metrics">{metricsText}</pre>
		</div>
	{/if}
</div>

<style>
	.page { max-width: 900px; }
	h1 { font-size: 1.5rem; font-weight: 600; margin-bottom: 24px; }
	h2 {
		font-size: 0.85rem; color: #888; text-transform: uppercase;
		letter-spacing: 0.03em; margin-bottom: 12px; margin-top: 32px;
	}
	.stats-row {
		display: flex; gap: 12px; flex-wrap: wrap;
	}
	.period { font-size: 0.8rem; color: #666; margin-top: 16px; }
	.raw-section { margin-top: 16px; }
	.raw-metrics {
		background: #111; border: 1px solid #222; border-radius: 8px;
		padding: 16px; font-family: 'SF Mono', 'Fira Code', monospace;
		font-size: 0.75rem; color: #888; overflow-x: auto;
		max-height: 400px; overflow-y: auto;
		white-space: pre;
	}
	.muted { color: #555; font-size: 0.85rem; }
	.error { color: #e55; font-size: 0.85rem; }
</style>
