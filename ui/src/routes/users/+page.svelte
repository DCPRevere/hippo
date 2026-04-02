<script lang="ts">
	import { onMount } from 'svelte';
	import { listUsers, createUser, deleteUser, listKeys, createKey, deleteKey } from '$lib/api';
	import type { User, ApiKey } from '$lib/types';

	let users = $state<User[]>([]);
	let loading = $state(true);
	let error = $state('');

	// Create form
	let newUserId = $state('');
	let newDisplayName = $state('');
	let newRole = $state('user');
	let createLoading = $state(false);
	let createResult = $state('');

	// Keys — per-user state
	let expandedUser = $state<string | null>(null);
	let userKeys = $state<Record<string, ApiKey[]>>({});
	let keyLabels = $state<Record<string, string>>({});
	let keyResults = $state<Record<string, { label: string; api_key: string } | null>>({});

	onMount(() => {
		loadUsers();
	});

	async function loadUsers() {
		loading = true;
		error = '';
		try {
			const resp = await listUsers();
			users = resp.users;
		} catch (e) {
			error = e instanceof Error ? e.message : String(e);
		} finally {
			loading = false;
		}
	}

	async function handleCreateUser() {
		if (!newUserId.trim() || !newDisplayName.trim()) return;
		createLoading = true;
		createResult = '';
		try {
			const resp = await createUser(newUserId.trim(), newDisplayName.trim(), newRole);
			createResult = `Created. API key: ${resp.api_key}`;
			newUserId = '';
			newDisplayName = '';
			newRole = 'user';
			await loadUsers();
		} catch (e) {
			createResult = `Error: ${e instanceof Error ? e.message : String(e)}`;
		} finally {
			createLoading = false;
		}
	}

	async function handleDeleteUser(userId: string) {
		if (!confirm(`Delete user "${userId}"?`)) return;
		try {
			await deleteUser(userId);
			await loadUsers();
		} catch (e) {
			error = e instanceof Error ? e.message : String(e);
		}
	}

	async function toggleKeys(userId: string) {
		if (expandedUser === userId) {
			expandedUser = null;
			return;
		}
		expandedUser = userId;
		try {
			const resp = await listKeys(userId);
			userKeys[userId] = resp.keys;
		} catch (e) {
			error = e instanceof Error ? e.message : String(e);
		}
	}

	async function handleCreateKey(userId: string) {
		const label = (keyLabels[userId] || '').trim();
		if (!label) return;
		keyResults[userId] = null;
		try {
			const resp = await createKey(userId, label);
			keyResults[userId] = { label: resp.label, api_key: resp.api_key };
			keyLabels[userId] = '';
			const keysResp = await listKeys(userId);
			userKeys[userId] = keysResp.keys;
		} catch (e) {
			keyResults[userId] = null;
			error = e instanceof Error ? e.message : String(e);
		}
	}

	async function handleDeleteKey(userId: string, label: string) {
		if (!confirm(`Revoke key "${label}"?`)) return;
		try {
			await deleteKey(userId, label);
			const resp = await listKeys(userId);
			userKeys[userId] = resp.keys;
		} catch (e) {
			error = e instanceof Error ? e.message : String(e);
		}
	}

	function copyToClipboard(text: string) {
		navigator.clipboard.writeText(text);
	}
</script>

<div class="page">
	<h1>Users</h1>

	<div class="section">
		<h2>Create User</h2>
		<div class="create-form">
			<input type="text" bind:value={newUserId} placeholder="User ID" />
			<input type="text" bind:value={newDisplayName} placeholder="Display name" />
			<select bind:value={newRole}>
				<option value="user">user</option>
				<option value="admin">admin</option>
			</select>
			<button onclick={handleCreateUser} disabled={createLoading}>
				{createLoading ? 'Creating...' : 'Create'}
			</button>
		</div>
		{#if createResult}
			<p class="result">{createResult}</p>
		{/if}
	</div>

	{#if loading}
		<p class="muted">Loading users...</p>
	{:else if error}
		<p class="error">{error}</p>
	{:else}
		<div class="section">
			<h2>All Users ({users.length})</h2>
			<div class="user-list">
				{#each users as user}
					<div class="user-item">
						<div class="user-header">
							<div class="user-info">
								<span class="user-name">{user.display_name}</span>
								<span class="user-id">{user.user_id}</span>
								<span class="role-badge">{user.role}</span>
							</div>
							<div class="user-actions">
								<button class="sm-btn" onclick={() => toggleKeys(user.user_id)}>
									{expandedUser === user.user_id ? 'Hide Keys' : 'Keys'}
								</button>
								<button class="sm-btn danger" onclick={() => handleDeleteUser(user.user_id)}>Delete</button>
							</div>
						</div>

						{#if expandedUser === user.user_id}
							<div class="keys-panel">
								<div class="keys-list">
									{#each userKeys[user.user_id] ?? [] as key}
										<div class="key-item">
											<span class="key-label">{key.label}</span>
											<span class="key-prefix">{key.prefix}...</span>
											<span class="key-date">{new Date(key.created_at).toLocaleDateString()}</span>
											<button class="sm-btn danger" onclick={() => handleDeleteKey(user.user_id, key.label)}>Revoke</button>
										</div>
									{:else}
										<p class="muted">No API keys</p>
									{/each}
								</div>
								<div class="create-key-form">
									<input type="text" bind:value={keyLabels[user.user_id]} placeholder="Key label" />
									<button class="sm-btn" onclick={() => handleCreateKey(user.user_id)}>Create Key</button>
								</div>
								{#if keyResults[user.user_id]}
									<div class="key-result-block">
										<p class="result key-result">New key: {keyResults[user.user_id]?.api_key}</p>
										<button class="sm-btn copy-btn" onclick={() => copyToClipboard(keyResults[user.user_id]?.api_key ?? '')}>Copy</button>
									</div>
								{/if}
							</div>
						{/if}
					</div>
				{:else}
					<p class="muted">No users found</p>
				{/each}
			</div>
		</div>
	{/if}
</div>

<style>
	.page { max-width: 800px; }
	h1 { font-size: 1.5rem; font-weight: 600; margin-bottom: 24px; }
	h2 {
		font-size: 0.85rem; color: #888; text-transform: uppercase;
		letter-spacing: 0.03em; margin-bottom: 12px;
	}
	.section { margin-bottom: 32px; }
	.create-form {
		display: flex; gap: 8px; flex-wrap: wrap;
	}
	.create-form input, .create-form select {
		background: #161616; border: 1px solid #333; color: #e0e0e0;
		padding: 8px 12px; border-radius: 6px; font-size: 0.85rem;
	}
	.create-form button {
		background: #4e9af5; color: #fff; border: none;
		padding: 8px 16px; border-radius: 6px; cursor: pointer;
		font-size: 0.85rem;
	}
	.create-form button:disabled { opacity: 0.5; }
	.result {
		font-size: 0.8rem; color: #4e9af5; margin-top: 8px;
		font-family: 'SF Mono', monospace; word-break: break-all;
	}
	.user-list { display: flex; flex-direction: column; gap: 4px; }
	.user-item {
		background: #161616; border: 1px solid #222;
		border-radius: 8px; overflow: hidden;
	}
	.user-header {
		display: flex; justify-content: space-between; align-items: center;
		padding: 12px 16px;
	}
	.user-info { display: flex; align-items: center; gap: 10px; }
	.user-name { font-weight: 500; font-size: 0.9rem; }
	.user-id { font-size: 0.75rem; color: #666; font-family: monospace; }
	.role-badge {
		font-size: 0.7rem; padding: 2px 8px; border-radius: 12px;
		background: #2a2a3a; color: #aab;
	}
	.user-actions { display: flex; gap: 6px; }
	.sm-btn {
		background: #222; border: 1px solid #333; color: #aaa;
		padding: 4px 10px; border-radius: 4px; cursor: pointer;
		font-size: 0.75rem;
	}
	.sm-btn:hover { background: #2a2a2a; color: #ccc; }
	.sm-btn.danger { color: #c88; border-color: #533; }
	.sm-btn.danger:hover { background: #2a1515; }
	.keys-panel {
		border-top: 1px solid #222; padding: 12px 16px;
		background: #131313;
	}
	.keys-list { display: flex; flex-direction: column; gap: 4px; margin-bottom: 8px; }
	.key-item {
		display: flex; align-items: center; gap: 10px;
		font-size: 0.8rem; padding: 4px 0;
	}
	.key-label { color: #ccc; min-width: 80px; }
	.key-prefix { font-family: monospace; color: #666; }
	.key-date { color: #555; flex: 1; }
	.create-key-form {
		display: flex; gap: 8px; margin-top: 8px;
	}
	.create-key-form input {
		background: #161616; border: 1px solid #333; color: #ccc;
		padding: 6px 10px; border-radius: 4px; font-size: 0.8rem;
		flex: 1;
	}
	.key-result-block {
		display: flex; align-items: center; gap: 8px; margin-top: 6px;
	}
	.key-result { margin-top: 0; }
	.copy-btn { flex-shrink: 0; }
	.muted { color: #555; font-size: 0.85rem; }
	.error { color: #e55; font-size: 0.85rem; }
</style>
