<script lang="ts">
	import { features } from '$lib/features';
	import { health } from '$lib/api';

	let apiKey = $state('');
	let error = $state('');
	let loading = $state(false);

	async function handleLogin() {
		if (!apiKey.trim()) {
			error = 'Please enter an API key';
			return;
		}
		loading = true;
		error = '';
		localStorage.setItem('hippo_api_key', apiKey.trim());
		try {
			await health();
			window.location.href = '/';
		} catch (e) {
			localStorage.removeItem('hippo_api_key');
			error = e instanceof Error ? e.message : 'Failed to connect';
		} finally {
			loading = false;
		}
	}

	function handleKeydown(e: KeyboardEvent) {
		if (e.key === 'Enter') handleLogin();
	}
</script>

<div class="login-page">
	<div class="login-card">
		<h1 class="logo">hippo</h1>
		<p class="subtitle">Knowledge graph engine</p>

		{#if features.mode === 'standalone'}
			<div class="form-group">
				<label for="api-key">API Key</label>
				<input
					id="api-key"
					type="password"
					bind:value={apiKey}
					onkeydown={handleKeydown}
					placeholder="Enter your API key"
					disabled={loading}
				/>
			</div>

			{#if error}
				<p class="error">{error}</p>
			{/if}

			<button class="login-btn" onclick={handleLogin} disabled={loading}>
				{loading ? 'Connecting...' : 'Connect'}
			</button>
		{:else}
			<p class="cloud-info">Redirecting to login...</p>
		{/if}
	</div>
</div>

<style>
	.login-page {
		display: flex;
		align-items: center;
		justify-content: center;
		min-height: 100vh;
		background: #0a0a0a;
	}
	.login-card {
		background: #111;
		border: 1px solid #222;
		border-radius: 12px;
		padding: 40px;
		width: 360px;
		text-align: center;
	}
	.logo {
		font-size: 2rem;
		font-weight: 700;
		color: #fff;
		margin-bottom: 4px;
	}
	.subtitle {
		color: #666;
		font-size: 0.85rem;
		margin-bottom: 32px;
	}
	.form-group {
		text-align: left;
		margin-bottom: 16px;
	}
	label {
		display: block;
		font-size: 0.8rem;
		color: #888;
		margin-bottom: 6px;
	}
	input {
		width: 100%;
		background: #0a0a0a;
		border: 1px solid #333;
		color: #e0e0e0;
		padding: 10px 12px;
		border-radius: 6px;
		font-size: 0.9rem;
		outline: none;
	}
	input:focus {
		border-color: #4e9af5;
	}
	.error {
		color: #e55;
		font-size: 0.8rem;
		margin-bottom: 12px;
	}
	.login-btn {
		width: 100%;
		background: #4e9af5;
		color: #fff;
		border: none;
		padding: 10px;
		border-radius: 6px;
		font-size: 0.9rem;
		cursor: pointer;
		font-weight: 500;
	}
	.login-btn:hover {
		background: #3d88e0;
	}
	.login-btn:disabled {
		opacity: 0.6;
		cursor: not-allowed;
	}
	.cloud-info {
		color: #666;
		font-size: 0.85rem;
	}
</style>
