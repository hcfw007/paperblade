<script lang="ts">
  import { onMount } from "svelte";
  import { open, save } from "@tauri-apps/plugin-dialog";
  import { invoke } from "@tauri-apps/api/core";

  type Quality = "screen" | "ebook" | "printer";

  type Report = {
    output: string;
    beforeBytes: number;
    afterBytes: number;
    grew: boolean;
  };

  const QUALITIES: { id: Quality; title: string; hint: string }[] = [
    { id: "screen", title: "Screen", hint: "72 dpi images — smallest" },
    { id: "ebook", title: "Ebook", hint: "150 dpi images — a good default" },
    { id: "printer", title: "Printer", hint: "300 dpi images — still printable" },
  ];

  let input = $state<string | null>(null);
  let quality = $state<Quality>("ebook");
  let engineReady = $state<boolean | null>(null);
  let busy = $state(false);
  let error = $state<string | null>(null);
  let report = $state<Report | null>(null);

  const canRun = $derived(Boolean(input) && engineReady === true && !busy);

  onMount(async () => {
    engineReady = await invoke<boolean>("has_compression_engine");
  });

  function basename(path: string): string {
    return path.split(/[\\/]/).pop() ?? path;
  }

  function formatSize(bytes: number): string {
    if (bytes < 1024) return `${bytes} B`;
    const units = ["KB", "MB", "GB"];
    let value = bytes / 1024;
    let unit = 0;
    while (value >= 1024 && unit < units.length - 1) {
      value /= 1024;
      unit += 1;
    }
    return `${value.toFixed(1)} ${units[unit]}`;
  }

  const savedPercent = $derived(
    report
      ? Math.round(
          ((report.beforeBytes - report.afterBytes) / report.beforeBytes) * 100,
        )
      : 0,
  );

  async function pickInput() {
    const picked = await open({
      multiple: false,
      filters: [{ name: "PDF", extensions: ["pdf"] }],
    });
    if (typeof picked !== "string") return;
    input = picked;
    error = null;
    report = null;
  }

  async function run() {
    if (!input) return;
    error = null;
    report = null;

    const name = basename(input).replace(/\.pdf$/i, "");
    const output = await save({
      defaultPath: `${name}-compressed.pdf`,
      filters: [{ name: "PDF", extensions: ["pdf"] }],
    });
    if (!output) return;

    busy = true;
    try {
      report = await invoke<Report>("compress_pdf", { input, output, quality });
    } catch (e) {
      error = typeof e === "string" ? e : "Compression failed. Please try again.";
    } finally {
      busy = false;
    }
  }
</script>

<section class="page">
  <header>
    <h1>Compress</h1>
    <p>Shrink a PDF with quality presets. Files stay on this computer.</p>
  </header>

  {#if engineReady === false}
    <div class="notice" role="status">
      <strong>Compression engine not available.</strong>
      This build doesn't bundle Ghostscript yet. Install it with
      <code>brew install ghostscript</code> to use this tool.
    </div>
  {/if}

  <div class="toolbar">
    <button class="btn" onclick={pickInput} disabled={busy}>Choose PDF</button>
  </div>

  {#if !input}
    <div class="empty">No file yet. Choose a PDF to compress.</div>
  {:else}
    <div class="source">
      <span class="name" title={input}>{basename(input)}</span>
    </div>

    <fieldset class="qualities">
      <legend>Quality</legend>
      {#each QUALITIES as option (option.id)}
        <label class="option">
          <input
            type="radio"
            bind:group={quality}
            value={option.id}
            disabled={busy}
          />
          <span class="option-body">
            <span class="option-title">{option.title}</span>
            <span class="option-hint">{option.hint}</span>
          </span>
        </label>
      {/each}
    </fieldset>

    <p class="hint">
      Compression re-encodes the whole document. A PDF whose images are already
      smaller than the preset can come out slightly larger.
    </p>
  {/if}

  <div class="actions">
    <button class="btn btn-primary" onclick={run} disabled={!canRun}>
      {busy ? "Compressing…" : "Compress PDF"}
    </button>
  </div>

  {#if error}
    <p class="status error" role="alert">{error}</p>
  {/if}

  {#if report}
    <div class="status" class:success={!report.grew} class:warn={report.grew}>
      {#if report.grew}
        <p>
          No smaller — {formatSize(report.beforeBytes)} →
          {formatSize(report.afterBytes)}. This PDF was already well compressed
          at this setting. Keep the original.
        </p>
      {:else}
        <p>
          {formatSize(report.beforeBytes)} → {formatSize(report.afterBytes)}
          <strong>({savedPercent}% smaller)</strong>
        </p>
      {/if}
      <p class="path" title={report.output}>Saved to {report.output}</p>
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

  .notice {
    margin-top: var(--space-5);
    padding: var(--space-4);
    border: 1px solid var(--color-border-strong);
    border-radius: var(--radius-md);
    background-color: var(--color-surface-raised);
    color: var(--color-text-muted);
    font-size: var(--text-sm);
    line-height: 1.6;
  }

  .notice strong {
    display: block;
    color: var(--color-text);
  }

  code {
    padding: 0.1em 0.35em;
    border-radius: var(--radius-sm);
    background-color: var(--color-surface);
    font-size: 0.95em;
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
    margin-top: var(--space-5);
    padding: var(--space-3) var(--space-4);
    background-color: var(--color-surface-raised);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-md);
  }

  .name {
    display: block;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--text-sm);
  }

  .qualities {
    margin-top: var(--space-5);
    border: none;
  }

  legend {
    padding: 0;
    color: var(--color-text-muted);
    font-size: var(--text-sm);
  }

  .option {
    display: flex;
    align-items: flex-start;
    gap: var(--space-3);
    margin-top: var(--space-3);
    cursor: pointer;
  }

  .option-body {
    display: flex;
    flex-direction: column;
    gap: 0.15rem;
  }

  .option-title {
    font-size: var(--text-sm);
    font-weight: 500;
  }

  .option-hint {
    color: var(--color-text-subtle);
    font-size: var(--text-sm);
  }

  .hint {
    margin-top: var(--space-4);
    color: var(--color-text-subtle);
    font-size: var(--text-sm);
    line-height: 1.6;
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

  .warn {
    color: var(--color-text-muted);
  }

  .path {
    margin-top: var(--space-2);
    color: var(--color-text-subtle);
  }
</style>
