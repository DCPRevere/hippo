<script lang="ts">
	import { onMount } from 'svelte';
	import { ask, listGraphs } from '$lib/api';
	import type { AskResponse } from '$lib/types';

	let question = $state('');
	let selectedGraph = $state('');
	let graphs = $state<string[]>([]);
	let limit = $state(10);
	let loading = $state(false);
	let error = $state('');
	let response = $state<AskResponse | null>(null);

	onMount(() => {
		loadGraphs();
	});

	async function loadGraphs() {
		try {
			const resp = await listGraphs();
			graphs = resp.graphs;
			selectedGraph = resp.default;
		} catch {
			// non-critical
		}
	}

	async function handleAsk() {
		if (!question.trim()) return;
		loading = true;
		error = '';
		response = null;
		try {
			response = await ask({
				question: question.trim(),
				limit,
				graph: selectedGraph || undefined,
				verbose: true
			});
		} catch (e) {
			error = e instanceof Error ? e.message : String(e);
		} finally {
			loading = false;
		}
	}

	function handleKeydown(e: KeyboardEvent) {
		if (e.key === 'Enter' && !e.shiftKey) {
			e.preventDefault();
			handleAsk();
		}
	}
</script>

<div class="page">
	<h1>Ask</h1>
	<p class="desc">Ask a question and get an answer based on the knowledge graph.</p>

	<div class="ask-form">
		<div class="input-row">
			<textarea
				bind:value={question}
				onkeydown={handleKeydown}
				placeholder="What do you want to know?"
				rows="3"
				disabled={loading}
			></textarea>
		</div>

		<div class="options-row">
			{#if graphs.length > 1}
				<select bind:value={selectedGraph}>
					{#each graphs as g}
						<option value={g}>{g}</option>
					{/each}
				</select>
			{/if}
			<label class="limit-label">
				Limit
				<input type="number" bind:value={limit} min="1" max="500" />
			</label>
			<button class="ask-btn" onclick={handleAsk} disabled={loading || !question.trim()}>
				{loading ? 'Thinking...' : 'Ask'}
			</button>
		</div>
	</div>

	{#if error}
		<p class="error">{error}</p>
	{/if}

	{#if response}
		<div class="answer-section">
			<h2>Answer</h2>
			<div class="answer-block">{response.answer}</div>

			{#if response.facts && response.facts.length > 0}
				<h2>Facts Used ({response.facts.length})</h2>
				<div class="facts-list">
					{#each response.facts as fact}
						<div class="fact-item">
							<span class="fact-text">{fact.fact}</span>
							<span class="fact-meta">
								{fact.subject} - {fact.relation_type} - {fact.object}
								({(fact.confidence * 100).toFixed(0)}%)
							</span>
						</div>
					{/each}
				</div>
			{/if}
		</div>
	{/if}
</div>

<style>
	.page { max-width: 800px; }
	h1 {
		font-size: 1.5rem;
		font-weight: 600;
		margin-bottom: 4px;
	}
	h2 {
		font-size: 0.85rem;
		color: #888;
		text-transform: uppercase;
		letter-spacing: 0.03em;
		margin-bottom: 8px;
	}
	.desc {
		color: #666;
		font-size: 0.85rem;
		margin-bottom: 24px;
	}
	.ask-form {
		margin-bottom: 24px;
	}
	textarea {
		width: 100%;
		background: #161616;
		border: 1px solid #333;
		color: #e0e0e0;
		padding: 12px;
		border-radius: 8px;
		font-size: 0.9rem;
		font-family: inherit;
		resize: vertical;
		outline: none;
	}
	textarea:focus { border-color: #4e9af5; }
	.options-row {
		display: flex;
		align-items: center;
		gap: 12px;
		margin-top: 8px;
	}
	select {
		background: #161616;
		border: 1px solid #333;
		color: #ccc;
		padding: 6px 10px;
		border-radius: 6px;
		font-size: 0.825rem;
	}
	.limit-label {
		display: flex;
		align-items: center;
		gap: 6px;
		font-size: 0.8rem;
		color: #888;
	}
	.limit-label input {
		width: 60px;
		background: #161616;
		border: 1px solid #333;
		color: #ccc;
		padding: 6px 8px;
		border-radius: 6px;
		font-size: 0.825rem;
	}
	.ask-btn {
		margin-left: auto;
		background: #4e9af5;
		color: #fff;
		border: none;
		padding: 8px 20px;
		border-radius: 6px;
		font-size: 0.85rem;
		cursor: pointer;
		font-weight: 500;
	}
	.ask-btn:hover { background: #3d88e0; }
	.ask-btn:disabled { opacity: 0.5; cursor: not-allowed; }
	.error { color: #e55; font-size: 0.85rem; margin-bottom: 16px; }
	.answer-section { margin-top: 8px; }
	.answer-block {
		background: #161616;
		border: 1px solid #222;
		border-radius: 8px;
		padding: 16px;
		font-size: 0.9rem;
		line-height: 1.5;
		margin-bottom: 24px;
		white-space: pre-wrap;
	}
	.facts-list {
		display: flex;
		flex-direction: column;
		gap: 6px;
		margin-bottom: 16px;
	}
	.fact-item {
		background: #131313;
		border: 1px solid #1a1a1a;
		border-radius: 6px;
		padding: 10px 12px;
	}
	.fact-text {
		display: block;
		font-size: 0.85rem;
		color: #ccc;
	}
	.fact-meta {
		display: block;
		font-size: 0.75rem;
		color: #555;
		margin-top: 4px;
	}
</style>
