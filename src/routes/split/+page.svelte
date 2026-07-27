<script lang="ts">
  import { open } from "@tauri-apps/plugin-dialog";
  import { invoke } from "@tauri-apps/api/core";

  type SplitMode =
    | { kind: "ranges"; ranges: string }
    | { kind: "everyN"; size: number };

  let input = $state<string | null>(null);
  let pageTotal = $state<number | null>(null);
  let outputDir = $state<string | null>(null);
  let mode = $state<"ranges" | "everyN">("ranges");
  let ranges = $state("");
  let chunkSize = $state(1);
  let busy = $state(false);
  let error = $state<string | null>(null);
  let results = $state<string[]>([]);

  const optionsReady = $derived(
    mode === "ranges" ? ranges.trim().length > 0 : chunkSize >= 1,
  );
  const canSplit = $derived(Boolean(input) && optionsReady && !busy);

  function basename(path: string): string {
    return path.split(/[\\/]/).pop() ?? path;
  }

  function parentDir(path: string): string {
    const cut = Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\"));
    return cut > 0 ? path.slice(0, cut) : "";
  }

  // Where the pieces land: an explicit choice, else beside the source file.
  const targetDir = $derived(outputDir ?? (input ? parentDir(input) : null));

  function reset() {
    error = null;
    results = [];
  }

  async function pickInput() {
    const picked = await open({
      multiple: false,
      filters: [{ name: "PDF", extensions: ["pdf"] }],
    });
    if (typeof picked !== "string") return;

    reset();
    input = picked;
    pageTotal = null;
    try {
      pageTotal = await invoke<number>("page_count", { input: picked });
    } catch (e) {
      error = typeof e === "string" ? e : "Could not read that PDF.";
      input = null;
    }
  }

  async function pickOutputDir() {
    const picked = await open({ directory: true, multiple: false });
    if (typeof picked !== "string") return;
    outputDir = picked;
    reset();
  }

  function buildMode(): SplitMode {
    return mode === "ranges"
      ? { kind: "ranges", ranges }
      : { kind: "everyN", size: Math.floor(chunkSize) };
  }

  async function run() {
    if (!input) return;
    reset();

    if (!targetDir) {
      error = "Choose a folder to save the pieces into.";
      return;
    }

    busy = true;
    try {
      results = await invoke<string[]>("split_pdf", {
        input,
        mode: buildMode(),
        outputDir: targetDir,
      });
    } catch (e) {
      error = typeof e === "string" ? e : "Split failed. Please try again.";
    } finally {
      busy = false;
    }
  }
</script>

<section class="page">
  <header>
    <h1>Split</h1>
    <p>Break a PDF into separate files. Files stay on this computer.</p>
  </header>

  <div class="toolbar">
    <button class="btn" onclick={pickInput} disabled={busy}>Choose PDF</button>
    {#if input}
      <button class="btn btn-ghost" onclick={pickOutputDir} disabled={busy}>
        Save to folder…
      </button>
    {/if}
  </div>

  {#if !input}
    <div class="empty">No file yet. Choose a PDF to split.</div>
  {:else}
    <div class="source">
      <span class="name" title={input}>{basename(input)}</span>
      {#if pageTotal !== null}
        <span class="meta">{pageTotal} pages</span>
      {/if}
    </div>

    <fieldset class="modes">
      <legend>How to split</legend>

      <label class="mode">
        <input type="radio" bind:group={mode} value="ranges" disabled={busy} />
        <span class="mode-body">
          <span class="mode-title">By page ranges</span>
          <span class="mode-hint">One file per range you list.</span>
        </span>
      </label>

      {#if mode === "ranges"}
        <div class="field">
          <input
            class="text"
            type="text"
            bind:value={ranges}
            oninput={reset}
            placeholder="1-3, 5, 8-10"
            disabled={busy}
            aria-label="Page ranges"
          />
        </div>
      {/if}

      <label class="mode">
        <input type="radio" bind:group={mode} value="everyN" disabled={busy} />
        <span class="mode-body">
          <span class="mode-title">Every N pages</span>
          <span class="mode-hint">Cut into equal chunks, last one short.</span>
        </span>
      </label>

      {#if mode === "everyN"}
        <div class="field">
          <input
            class="text number"
            type="number"
            min="1"
            max={pageTotal ?? undefined}
            bind:value={chunkSize}
            oninput={reset}
            disabled={busy}
            aria-label="Pages per file"
          />
          <span class="meta">pages per file</span>
        </div>
      {/if}
    </fieldset>

    {#if targetDir}
      <p class="meta destination" title={targetDir}>
        Saving to {basename(targetDir)}{outputDir ? "" : " (beside the source)"}
      </p>
    {/if}
  {/if}

  <div class="actions">
    <button class="btn btn-primary" onclick={run} disabled={!canSplit}>
      {busy ? "Splitting…" : "Split PDF"}
    </button>
  </div>

  {#if error}
    <p class="status error" role="alert">{error}</p>
  {/if}
  {#if results.length}
    <div class="status success">
      <p>Wrote {results.length} {results.length === 1 ? "file" : "files"}:</p>
      <ul class="results">
        {#each results as path (path)}
          <li title={path}>{basename(path)}</li>
        {/each}
      </ul>
    </div>
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

  .source {
    display: flex;
    align-items: baseline;
    gap: var(--space-3);
    margin-top: var(--space-5);
    padding: var(--space-3) var(--space-4);
    background-color: var(--color-surface-raised);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-md);
  }

  .name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--text-sm);
  }

  .meta {
    flex: none;
    color: var(--color-text-subtle);
    font-size: var(--text-sm);
    font-variant-numeric: tabular-nums;
  }

  .modes {
    margin-top: var(--space-5);
    border: none;
  }

  legend {
    padding: 0;
    color: var(--color-text-muted);
    font-size: var(--text-sm);
  }

  .mode {
    display: flex;
    align-items: flex-start;
    gap: var(--space-3);
    margin-top: var(--space-3);
    cursor: pointer;
  }

  .mode-body {
    display: flex;
    flex-direction: column;
    gap: 0.15rem;
  }

  .mode-title {
    font-size: var(--text-sm);
    font-weight: 500;
  }

  .mode-hint {
    color: var(--color-text-subtle);
    font-size: var(--text-sm);
  }

  .field {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    margin: var(--space-3) 0 var(--space-4) var(--space-6);
  }

  .text {
    flex: 1;
    min-width: 0;
    padding: var(--space-2) var(--space-3);
    border: 1px solid var(--color-border-strong);
    border-radius: var(--radius-md);
    background-color: var(--color-surface-raised);
    color: var(--color-text);
    font-size: var(--text-sm);
  }

  .text:focus-visible {
    outline: none;
    border-color: var(--color-accent);
  }

  .number {
    flex: none;
    width: 5rem;
    font-variant-numeric: tabular-nums;
  }

  .destination {
    margin-top: var(--space-2);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
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

  .results {
    margin-top: var(--space-2);
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    color: var(--color-text-muted);
    list-style: none;
  }
</style>
