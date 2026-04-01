<script lang="ts">
	import type { Edge } from '$lib/types';

	let { edges, onselect }: {
		edges: Edge[];
		onselect?: (edge: Edge) => void;
	} = $props();
</script>

<div class="edge-table-wrap">
	<table class="edge-table">
		<thead>
			<tr>
				<th>Fact</th>
				<th>Relation</th>
				<th>Subject</th>
				<th>Object</th>
				<th>Confidence</th>
				<th>Tier</th>
			</tr>
		</thead>
		<tbody>
			{#each edges as edge}
				<tr
					class:clickable={!!onselect}
					onclick={() => onselect?.(edge)}
				>
					<td class="fact">{edge.fact}</td>
					<td><span class="rel-badge">{edge.relation_type}</span></td>
					<td>{edge.subject_name}</td>
					<td>{edge.object_name}</td>
					<td class="num">{(edge.confidence * 100).toFixed(0)}%</td>
					<td><span class="tier-badge">{edge.memory_tier}</span></td>
				</tr>
			{/each}
			{#if edges.length === 0}
				<tr><td colspan="6" class="empty">No edges</td></tr>
			{/if}
		</tbody>
	</table>
</div>

<style>
	.edge-table-wrap {
		overflow-x: auto;
	}
	.edge-table {
		width: 100%;
		border-collapse: collapse;
		font-size: 0.825rem;
	}
	th {
		text-align: left;
		padding: 8px 12px;
		color: #888;
		font-weight: 500;
		font-size: 0.75rem;
		text-transform: uppercase;
		letter-spacing: 0.03em;
		border-bottom: 1px solid #222;
	}
	td {
		padding: 8px 12px;
		border-bottom: 1px solid #1a1a1a;
		color: #ccc;
	}
	.fact {
		max-width: 300px;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.num {
		text-align: right;
		font-variant-numeric: tabular-nums;
	}
	.rel-badge, .tier-badge {
		font-size: 0.7rem;
		padding: 2px 6px;
		border-radius: 4px;
		background: #222;
		color: #999;
	}
	.clickable {
		cursor: pointer;
	}
	.clickable:hover td {
		background: #1a1a1a;
	}
	.empty {
		text-align: center;
		color: #555;
		padding: 24px 12px;
	}
</style>
