<script lang="ts">
  import { open, save } from "@tauri-apps/plugin-dialog";
  import { invoke } from "@tauri-apps/api/core";

  type PageSize = "image" | "a4" | "letter";
  type Fit = "contain" | "cover" | "stretch";

  type Report = {
    output: string;
    pages: number;
    outputBytes: number;
  };

  const PAGE_SIZES: { id: PageSize; title: string; hint: string }[] = [
    {
      id: "image",
      title: "Follow image",
      hint: "Each page matches its picture's own size and density",
    },
    { id: "a4", title: "A4", hint: "210 × 297 mm" },
    { id: "letter", title: "Letter", hint: "8.5 × 11 in" },
  ];

  const FITS: { id: Fit; title: string; hint: string }[] = [
    {
      id: "contain",
      title: "Fit",
      hint: "Whole picture visible, centered — white bands if needed",
    },
    {
      id: "cover",
      title: "Fill",
      hint: "Covers the whole page — edges past it are cropped",
    },
    { id: "stretch", title: "Stretch", hint: "Fills the page exactly, may distort" },
  ];

  let files = $state<string[]>([]);
  let pageSize = $state<PageSize>("image");
  let fit = $state<Fit>("contain");
  let busy = $state(false);
  let error = $state<string | null>(null);
  let report = $state<Report | null>(null);

  // A follow-image page already has the picture's aspect ratio, so every
  // fit lands on the same rectangle — the choice stands down in that mode.
  const followImage = $derived(pageSize === "image");
  const canRun = $derived(files.length > 0 && !busy);

  function basename(path: string): string {
    return path.split(/[\\/]/).pop() ?? path;
  }

  function stem(path: string): string {
    return basename(path).replace(/\.[^.]+$/, "");
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

  async function addImages() {
    error = null;
    const picked = await open({
      multiple: true,
      filters: [{ name: "Images", extensions: ["png", "jpg", "jpeg"] }],
    });
    if (!picked) return;
    const next = Array.isArray(picked) ? picked : [picked];
    files = [...files, ...next.filter((p) => !files.includes(p))];
    report = null;
  }

  function removeAt(index: number) {
    files = files.filter((_, i) => i !== index);
    report = null;
  }

  function move(index: number, delta: number) {
    const target = index + delta;
    if (target < 0 || target >= files.length) return;
    const next = [...files];
    [next[index], next[target]] = [next[target], next[index]];
    files = next;
    report = null;
  }

  function clearAll() {
    files = [];
    error = null;
    report = null;
  }

  async function run() {
    if (files.length === 0) return;
    error = null;
    report = null;

    const output = await save({
      defaultPath: files.length === 1 ? `${stem(files[0])}.pdf` : "images.pdf",
      filters: [{ name: "PDF", extensions: ["pdf"] }],
    });
    if (!output) return;

    busy = true;
    try {
      report = await invoke<Report>("images_to_pdf", {
        inputs: files,
        output,
        options: { pageSize, fit },
      });
    } catch (e) {
      error = typeof e === "string" ? e : "Building the PDF failed. Please try again.";
    } finally {
      busy = false;
    }
  }
</script>

<section class="page">
  <header>
    <h1>Images to PDF</h1>
    <p>Combine pictures into one PDF, a page each. Files stay on this computer.</p>
  </header>

  <div class="toolbar">
    <button class="btn" onclick={addImages} disabled={busy}>Add images</button>
    {#if files.length}
      <button class="btn btn-ghost" onclick={clearAll} disabled={busy}>
        Clear
      </button>
    {/if}
  </div>

  {#if files.length === 0}
    <div class="empty">No images yet. Add PNG or JPEG pictures to combine.</div>
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

    <div class="fields">
      <fieldset class="options">
        <legend>Page size</legend>
        {#each PAGE_SIZES as option (option.id)}
          <label class="option">
            <input
              type="radio"
              bind:group={pageSize}
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

      <fieldset class="options">
        <legend>Fit on the page</legend>
        {#each FITS as option (option.id)}
          <label class="option">
            <input
              type="radio"
              bind:group={fit}
              value={option.id}
              disabled={busy || followImage}
            />
            <span class="option-body">
              <span class="option-title">{option.title}</span>
              <span class="option-hint">{option.hint}</span>
            </span>
          </label>
        {/each}
        {#if followImage}
          <span class="hint">
            Pages follow the image size here, so fitting changes nothing.
          </span>
        {/if}
      </fieldset>
    </div>

    <p class="hint note">
      Your image data goes in untouched: JPEGs are embedded exactly as they
      are — no re-encoding and no size surprise — and PNGs are stored
      losslessly. Want it smaller? Run the result through Compress.
    </p>
  {/if}

  <div class="actions">
    <button class="btn btn-primary" onclick={run} disabled={!canRun}>
      {busy ? "Building…" : "Build PDF"}
    </button>
  </div>

  {#if error}
    <p class="status error" role="alert">{error}</p>
  {/if}

  {#if report}
    <div class="status success">
      <p>
        {report.pages}
        {report.pages === 1 ? "page" : "pages"} ·
        {formatSize(report.outputBytes)} — every picture is one page.
      </p>
      <p class="path" title={report.output}>Saved to {report.output}</p>
    </div>
  {/if}

  <p class="hint note">
    PDF → images is still planned: it needs the pdfium rendering engine, and
    bundling that safely is a Milestone 3 prerequisite (see ROADMAP.md).
  </p>
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

  .fields {
    margin-top: var(--space-5);
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }

  .options {
    margin-top: var(--space-1);
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
    color: var(--color-text-subtle);
    font-size: var(--text-sm);
    line-height: 1.6;
  }

  .note {
    margin-top: var(--space-4);
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

  .path {
    margin-top: var(--space-2);
    color: var(--color-text-subtle);
  }
</style>
