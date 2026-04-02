<script lang="ts">
	import type { Edge } from '$lib/types';

	let { edge, onclose }: {
		edge: Edge;
		onclose: () => void;
	} = $props();
</script>

<div class="detail-panel">
	<div class="panel-header">
		<h3>Edge #{edge.edge_id}</h3>
		<button class="close-btn" onclick={onclose}>x</button>
	</div>

	<div class="fact-block">
		<p class="fact">{edge.fact}</p>
	</div>

	<div class="meta-grid">
		<div class="meta-item">
			<span class="meta-label">Subject</span>
			<span class="meta-value">{edge.subject_name}</span>
		</div>
		<div class="meta-item">
			<span class="meta-label">Relation</span>
			<span class="meta-value">{edge.relation_type}</span>
		</div>
		<div class="meta-item">
			<span class="meta-label">Object</span>
			<span class="meta-value">{edge.object_name}</span>
		</div>
		<div class="meta-item">
			<span class="meta-label">Confidence</span>
			<span class="meta-value">{(edge.confidence * 100).toFixed(0)}%</span>
		</div>
		<div class="meta-item">
			<span class="meta-label">Salience</span>
			<span class="meta-value">{edge.salience}</span>
		</div>
		<div class="meta-item">
			<span class="meta-label">Tier</span>
			<span class="meta-value">{edge.memory_tier}</span>
		</div>
		<div class="meta-item">
			<span class="meta-label">Valid at</span>
			<span class="meta-value">{new Date(edge.valid_at).toLocaleString()}</span>
		</div>
		{#if edge.invalid_at}
			<div class="meta-item">
				<span class="meta-label">Invalid at</span>
				<span class="meta-value">{new Date(edge.invalid_at).toLocaleString()}</span>
			</div>
		{/if}
		{#if edge.expires_at}
			<div class="meta-item">
				<span class="meta-label">Expires</span>
				<span class="meta-value">{new Date(edge.expires_at).toLocaleString()}</span>
			</div>
		{/if}
		<div class="meta-item">
			<span class="meta-label">Decayed</span>
			<span class="meta-value">{(edge.decayed_confidence * 100).toFixed(0)}%</span>
		</div>
		<div class="meta-item">
			<span class="meta-label">Sources</span>
			<span class="meta-value">{edge.source_agents.split('|').filter(Boolean).join(', ') || 'none'}</span>
		</div>
	</div>
</div>

<style>
	.detail-panel {
		background: #111;
		border-left: 1px solid #222;
		padding: 20px;
		height: 100%;
		overflow-y: auto;
	}
	.panel-header {
		display: flex;
		justify-content: space-between;
		align-items: center;
		margin-bottom: 16px;
	}
	h3 {
		font-size: 1.1rem;
		font-weight: 600;
	}
	.close-btn {
		background: none;
		border: none;
		color: #666;
		font-size: 1.2rem;
		cursor: pointer;
		padding: 4px 8px;
	}
	.close-btn:hover { color: #ccc; }
	.fact-block {
		background: #181818;
		border: 1px solid #222;
		border-radius: 6px;
		padding: 12px;
		margin-bottom: 16px;
	}
	.fact {
		font-size: 0.9rem;
		color: #ddd;
		line-height: 1.4;
	}
	.meta-grid {
		display: grid;
		gap: 8px;
	}
	.meta-item {
		display: flex;
		gap: 8px;
	}
	.meta-label {
		font-size: 0.75rem;
		color: #666;
		min-width: 70px;
	}
	.meta-value {
		font-size: 0.825rem;
		color: #ccc;
	}
</style>
