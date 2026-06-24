<script lang="ts">
  import { open, save } from "@tauri-apps/plugin-dialog";
  import { invoke } from "@tauri-apps/api/core";

  let files = $state<string[]>([]);
  let busy = $state(false);
  let error = $state<string | null>(null);
  let resultPath = $state<string | null>(null);

  const canMerge = $derived(files.length >= 2 && !busy);

  function basename(path: string): string {
    return path.split(/[\\/]/).pop() ?? path;
  }

  async function addFiles() {
    error = null;
    const picked = await open({
      multiple: true,
      filters: [{ name: "PDF", extensions: ["pdf"] }],
    });
    if (!picked) return;
    const next = Array.isArray(picked) ? picked : [picked];
    files = [...files, ...next.filter((p) => !files.includes(p))];
  }

  function removeAt(index: number) {
    files = files.filter((_, i) => i !== index);
    resultPath = null;
  }

  function move(index: number, delta: number) {
    const target = index + delta;
    if (target < 0 || target >= files.length) return;
    const next = [...files];
    [next[index], next[target]] = [next[target], next[index]];
    files = next;
  }

  function clearAll() {
    files = [];
    error = null;
    resultPath = null;
  }

  async function run() {
    error = null;
    resultPath = null;
    const output = await save({
      defaultPath: "merged.pdf",
      filters: [{ name: "PDF", extensions: ["pdf"] }],
    });
    if (!output) return;

    busy = true;
    try {
      resultPath = await invoke<string>("merge_pdfs", { inputs: files, output });
    } catch (e) {
      error = typeof e === "string" ? e : "Merge failed. Please try again.";
    } finally {
      busy = false;
    }
  }
</script>

<section class="page">
  <header>
    <h1>Merge</h1>
    <p>Combine multiple PDFs into one. Files stay on this computer.</p>
  </header>

  <div class="toolbar">
    <button class="btn" onclick={addFiles} disabled={busy}>Add PDFs</button>
    {#if files.length}
      <button class="btn btn-ghost" onclick={clearAll} disabled={busy}>
        Clear
      </button>
    {/if}
  </div>

  {#if files.length === 0}
    <div class="empty">No files yet. Add at least two PDFs to merge.</div>
  {:else}
    <ol class="list">
      {#each files as file, i (file)}
        <li class="item">
          <span class="idx">{i + 1}</span>
          <span class="name" title={file}>{basename(file)}</span>
          <span class="controls">
            <button
              class="icon"
              onclick={() => move(i, -1)}
              disabled={i === 0 || busy}
              aria-label="Move up">↑</button
            >
            <button
              class="icon"
              onclick={() => move(i, 1)}
              disabled={i === files.length - 1 || busy}
              aria-label="Move down">↓</button
            >
            <button
              class="icon"
              onclick={() => removeAt(i)}
              disabled={busy}
              aria-label="Remove">✕</button
            >
          </span>
        </li>
      {/each}
    </ol>
  {/if}

  <div class="actions">
    <button class="btn btn-primary" onclick={run} disabled={!canMerge}>
      {busy ? "Merging…" : "Merge PDFs"}
    </button>
  </div>

  {#if error}
    <p class="status error" role="alert">{error}</p>
  {/if}
  {#if resultPath}
    <p class="status success">Saved to {resultPath}</p>
  {/if}
</section>

<style>
  .page {
    max-width: 720px;
    margin: var(--space-6) auto;
    padding: 0 var(--space-6);
  }

  h1 {
    font-size: var(--text-2xl);
    font-weight: 600;
    letter-spacing: -0.02em;
  }

  header p {
    margin-top: var(--space-2);
    color: var(--color-text-muted);
  }

  .toolbar {
    display: flex;
    gap: var(--space-3);
    margin-top: var(--space-6);
  }

  .btn {
    padding: var(--space-2) var(--space-4);
    border: 1px solid var(--color-border-strong);
    border-radius: var(--radius-md);
    background-color: var(--color-surface-raised);
    color: var(--color-text);
    font-size: var(--text-sm);
    font-weight: 500;
    transition: border-color var(--duration-fast) var(--ease-out);
  }

  .btn:hover:not(:disabled) {
    border-color: var(--color-accent);
  }

  .btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .btn-ghost {
    border-color: transparent;
    color: var(--color-text-muted);
  }

  .btn-primary {
    border-color: transparent;
    background-color: var(--color-accent);
    color: oklch(100% 0 0);
  }

  .btn-primary:hover:not(:disabled) {
    background-color: var(--color-accent-strong);
  }

  .empty {
    margin-top: var(--space-5);
    padding: var(--space-7);
    border: 1px dashed var(--color-border);
    border-radius: var(--radius-lg);
    text-align: center;
    color: var(--color-text-subtle);
  }

  .list {
    margin-top: var(--space-5);
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .item {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-3) var(--space-4);
    background-color: var(--color-surface-raised);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-md);
  }

  .idx {
    flex: none;
    width: 1.5rem;
    color: var(--color-text-subtle);
    font-variant-numeric: tabular-nums;
    font-size: var(--text-sm);
  }

  .name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--text-sm);
  }

  .controls {
    flex: none;
    display: flex;
    gap: var(--space-1);
  }

  .icon {
    width: 1.75rem;
    height: 1.75rem;
    border: 1px solid transparent;
    border-radius: var(--radius-sm);
    color: var(--color-text-muted);
    font-size: var(--text-sm);
  }

  .icon:hover:not(:disabled) {
    border-color: var(--color-border);
    color: var(--color-text);
  }

  .icon:disabled {
    opacity: 0.3;
    cursor: not-allowed;
  }

  .actions {
    margin-top: var(--space-6);
  }

  .status {
    margin-top: var(--space-4);
    font-size: var(--text-sm);
    word-break: break-all;
  }

  .error {
    color: var(--color-danger);
  }

  .success {
    color: var(--color-success);
  }
</style>
