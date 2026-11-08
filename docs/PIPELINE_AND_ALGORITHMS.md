# Vira Translator Studio — Algorithms & Pipeline (Teaching Edition)

> Audience: students who know basic Python/JavaScript and want to understand how a
> real end-to-end AI application is engineered.
>
> Goal: after reading this document, a student should be able to draw the whole
> pipeline from memory, explain every stage's input/output, read the source files
> without getting lost, and debug the common failure modes.

---

## 0. How to use this document in class

Suggested teaching plan (2 sessions of ~90 minutes, or 4 sessions of ~45 minutes):

| Session | Topic | What students should do |
|---|---|---|
| 1 | Big picture, data model, job system | Draw the boxes-and-arrows diagram from memory |
| 2 | OCR pipeline (mode detection, chunks, clustering, dedup, cross-page) | Trace one page through the code with a debugger |
| 3 | Translation + editing + inpainting | Explain why translation is JSON-batched and why masks are built from polygons |
| 4 | Rendering + export + failure modes | Run a small PDF end-to-end and read the job log line by line |

Board-first rule: **never show code before the student can describe the data
transformation in one sentence** ("PDF page → list of text boxes with text").

---

## 1. What the system does (one paragraph)

The user uploads a comic/manga/manhwa PDF (or imports chapter images). The system:

1. detects where the text is (OCR) and groups detected text lines into speech
   "blocks";
2. optionally translates each block to Persian with an LLM;
3. lets the user correct boxes and translations in a visual editor;
4. erases the original text from the artwork (inpainting);
5. draws the Persian translation back into the same region with a Persian font;
6. exports the result as PDF, WebP ZIP, or layered PSD.

Everything runs locally except the optional translation API call.

---

## 2. Technology stack and repository map

### Stack

| Layer | Technology | Why it matters |
|---|---|---|
| Frontend | Next.js 15 (App Router), React, TypeScript, Tailwind | Editor UI, live preview, polling job status |
| Backend | Django 5 + Django REST Framework | Data model, REST API, background-job orchestration |
| Database | SQLite | Zero-config local storage |
| OCR | PaddleOCR 3.x / PP-OCRv6 (English) | Text-line detection + recognition |
| Inpainting | OpenCV, SimpleLaMa (optional), PanelCleaner (optional) | Remove original lettering |
| Rendering | Pillow (FreeType) + bundled Persian fonts | Draw shaped Persian text |
| PDF I/O | PyMuPDF (`fitz`) | Render PDF pages and write output PDF |
| PSD export | `psd-tools` | Layered output for editors |

### Repository map

```
Vira_MangaTranslatorStudio/
├── backend/
│   ├── backend/                  Django project (settings, urls)
│   ├── studio/                   Django app: API + persistence + job runners
│   │   ├── models.py             Project, Page, TextBlock, Job, MaskMark, GlossaryEntry
│   │   ├── views.py              REST endpoints + job submission
│   │   ├── serializers.py        Request/response validation
│   │   ├── jobs.py               Background thread runner, locks, cancellation
│   │   ├── services.py           THE ORCHESTRATOR: extract / translate / clean / render / apply
│   │   ├── page_sources.py       Editable page collections (append, delete, crop)
│   │   └── chapter_import.py     Bounded web chapter image importer
│   ├── studio_engine/
│   │   ├── config.json           Seed configuration
│   │   └── comic_translator/     Vendored engine (no Django dependency)
│   │       ├── config.py         Dataclass configs + JSON → AppConfig
│   │       ├── models.py         OCRItem, OCRDialog, Block
│   │       ├── pdf.py            Render pages, build PDF/ZIP, concatenate strips
│   │       ├── document_analyzer.py   Long-strip vs standard mode
│   │       ├── ocr.py            PaddleOCR wrapper + DBSCAN clustering
│   │       ├── ocr_pipeline/     Chunking, OCR, dedup, cross-page merge
│   │       ├── translator/       Provider interface + OpenRouter/Gemini/Custom
│   │       ├── inpaint.py        Backend dispatch + fallback chain
│   │       ├── inpainters/       cv_auto, opencv, color, pcleaner backends
│   │       ├── renderer.py       Font fitting, color, stroke, chapter stamp
│   │       ├── typography.py     Central rule catalog (specification)
│   │       └── psd_export.py     Layered PSD export
│   ├── fonts/                    Persian TTF/OTF fonts
│   └── media/projects/<uuid>/    pages/, cleaned/, rendered/, outputs/, sources/
└── frontend/
    ├── app/                      Routes: landing, projects, workspace, settings
    ├── components/workspace/     CanvasEditor, panels, modals, hooks
    └── lib/                      API client, types, geometry, i18n, theme
```

**Most important file for the whole system: `backend/studio/services.py`.**
It is the bridge between the Django world and the engine world. If a student
understands `services.py`, they understand 70% of the application.

---

## 3. Big-picture architecture

```
┌──────────────────────────────┐
│  Browser (Next.js frontend)  │
│  editor • preview • polling │
└──────────────┬───────────────┘
               │  REST /api (JSON, multipart)
               ▼
┌──────────────────────────────┐
│  Django + DRF (backend)      │
│  views → serializers → DB    │
│  submit_job() starts thread  │
└──────────────┬───────────────┘
               │  in-process function calls
               ▼
┌──────────────────────────────┐
│  studio/services.py          │
│  extract/translate/clean/    │
│  render/apply runners        │
└──────────────┬───────────────┘
               │  Python API (lazy imports)
               ▼
┌──────────────────────────────┐
│  comic_translator engine     │
│  pdf • ocr • inpaint •       │
│  translator • renderer •     │
│  psd_export                  │
└──────────────┬───────────────┘
               │
               ▼
┌──────────────────────────────┐
│  SQLite + media/projects/    │
│  pages/ cleaned/ rendered/   │
│  outputs/ sources/           │
└──────────────────────────────┘
```

Key architectural decision: **the engine is vendored and framework-agnostic**.
`comic_translator` knows nothing about Django. Django imports it lazily inside
service functions, so the server can boot even when heavy AI packages are not
installed.

---

## 4. The data model

### 4.1 Database entities

```
Project (1) ───< Page (1) ───< TextBlock
    │              │
    │              └──< MaskMark
    ├──< Job
    └──< GlossaryEntry
```

| Entity | Meaning | Important fields |
|---|---|---|
| `Project` | One PDF / chapter / collection | `status`, `pdf_type` (manga/manhwa), `uses_page_sources`, config snapshots, counters |
| `Page` | One image in the pipeline | `page_number`, `width`, `height` |
| `TextBlock` | One dialogue/SFX region | `key`, `original_text`, `translation`, `block_type`, bbox `(x1,y1,x2,y2)`, `text_boxes` polygons, style overrides, `rotation`, `orientation` |
| `Job` | One long-running operation | `kind`, `status`, `progress`, `stage`, `logs`, `params`, `result` |
| `MaskMark` | Freehand eraser stroke | `points` in image pixels |
| `GlossaryEntry` | Consistent name translation | `source` → `target` |

### 4.2 Project status state machine

```
created ──extract──► extracting ──success──► ready
   ▲                     │                    │
   │                     └──failure──► failed│
   │                                          │
   ├────────────── translate ────────────────►│
   │                                          │
   ├────────────── apply/clean/render ───────►│
   │                                          │
   └────────────── reset/cancel ──────────────┘
```

`TRANSLATING` and `APPLYING` are transient; jobs normally settle back to
`READY`, and `APPLY` settles to `DONE`.

### 4.3 Job state machine

```
pending ──► running ──► success
   │           │
   │           ├──► failed
   │           └──► cancelled
   └──► cancelled
```

The `Job` row is the single source of truth for the UI progress bar. The worker
thread writes `progress` (0..1), `stage` (human text), and `logs` (last 400
lines).

### 4.4 Where the bytes live

```
media/projects/<project-uuid>/
├── pages/page-N.jpg        standardized page image (OCR + editor coordinate space)
├── cleaned/page-N.jpg      after inpainting (original text removed)
├── rendered/page-N.jpg     after Persian text is drawn
├── outputs/*.pdf|*.zip      final exports
├── sources/page-N.jpg      preserved page collection (when edited)
└── sources_original/       exact uploaded originals (archival)
```

**Coordinate invariant:** every `TextBlock` coordinate is expressed in the
pixel space of the corresponding `pages/page-N.jpg` image. The editor, OCR,
inpainting, and renderer all share this single space. Violating it is the most
common source of "text appears in the wrong place" bugs.

---

## 5. The job system (how long work runs)

### 5.1 Why background jobs?

OCR, inpainting, and LLM translation take seconds to minutes. A normal HTTP
request cannot block that long. The API therefore returns `202 Accepted` with a
`Job` object immediately, and the frontend polls `GET /api/jobs/<id>`.

### 5.2 Submission algorithm (`studio/jobs.py`)

```
submit_job(project, kind, params, run):
    lock = per_project_lock(project.id)
    if not lock.acquire(blocking=False):
        raise "Another job is already running"
    job = Job.objects.create(project, kind, params)
    cancel_event = event_for(job.id)
    start daemon thread:
        ctx = JobContext(job)
        try:
            ctx.progress(0, "Starting")
            result = run(ctx)
            ctx.check_cancelled()
            ctx.succeed(result)
        except JobCancelled:
            finalize row as CANCELLED
            settle project to READY or CREATED
        except Exception as e:
            sync counters from partial work
            ctx.fail(traceback)
            project.status = FAILED
        finally:
            remove cancel event
            release lock
```

Important engineering details worth teaching:

- **One heavy job per project** — enforced by an in-process lock, not a DB flag.
- **SQLite write contention** — progress/log writes retry with exponential
  backoff (`_db_write_retry`).
- **Cancel is cooperative** — `ctx.check_cancelled()` is called at choke points
  (progress reports, per page, per inpaint component). A blocked HTTP call to an
  LLM cannot be interrupted, but the job stops as soon as it returns.
- **Terminal-state protection** — `progress()` updates only rows still in
  `PENDING`/`RUNNING`, so a late progress write cannot resurrect a cancelled job.
- **Zombie healing** — if the server restarts, an active job row remains but no
  worker holds the lock. `settle_zombie()` detects "active row + free lock" and
  marks it failed instead of bricking the project forever.

### 5.3 Job kinds

| Kind | Runner | Effect |
|---|---|---|
| `extract` | `run_extract` | PDF → pages + OCR blocks |
| `translate` | `run_translate` | Blocks → Persian translations |
| `clean` | `run_clean` | Inpaint only (no text drawing) |
| `render` | `run_render_text` | Draw text only (clean first if needed) |
| `apply` | `run_apply` | Inpaint + render + export files |
| `reocr` | `run_reocr` | Re-run OCR for one page at higher quality |
| `import_pages` | `run_import_pages` | Download chapter images as pages |

---

## 6. End-to-end pipeline

### 6.1 Overview diagram

```
INPUT: PDF or page images
   │
   ▼
[0] Project / pages created
   │
   ▼
[1] Document analysis ──► mode = STANDARD | LONG_STRIP
   │
   ├──────── STANDARD ────────┐        ├──────── LONG_STRIP + manhwa ────────┐
   │                          │        │                                      │
   ▼                          │        ▼                                      │
render page @ DPI             │      render pages @ DPI                        │
resize width 1200             │      resize to 1200 wide                       │
cap max side                  │      concatenate ALL pages → one tall image    │
   │                          │      split tall image into chunks              │
   ▼                          │      (overlap for OCR, none for storage)       │
split into chunks             │      OCR each chunk                            │
OCR each chunk                │      dedup                                     │
restore coordinates           │      cluster                                   │
   │                          │      map tall coords → chunk-local coords      │
   ▼                          │        │                                      │
dedup                         │        │                                      │
DBSCAN cluster                │        │                                      │
merge nearby lines            │        │                                      │
   │                          │        │                                      │
   ▼                          │        │                                      │
cross-page pass (optional) ───┘        │                                      │
   │                                   │                                      │
   ▼◄──────────────────────────────────┘
[2] Persist Pages + TextBlocks to DB
   │
   ▼
[3] User edits boxes/translations (or auto-translate)
   │
   ▼
[4] Inpaint: erase original text (blocks with translation, non-SFX + brush marks)
   │
   ▼
[5] Render: draw Persian text into the same boxes
   │
   ▼
[6] Export: PDF / ZIP (WebP) / PSD
```

### 6.2 Stage 0 — Project creation

Three input paths:

1. **PDF upload** — stored at `projects/pdfs/…`; pages are rendered on demand.
2. **Page sources** (`uses_page_sources=True`) — images uploaded/imported into
   `sources/`; the PDF is never modified. Deleting/cropping/appending pages is
   allowed only in this mode.
3. **Link import** — `chapter_import.py` discovers `<img>` URLs on a public
   chapter page, downloads them with SSRF protection, standardizes them, and
   stores them as pages.

At creation, the project copies the **global config** into its own snapshots
(`ocr_config`, `font_config`, `inpaint_config`, `translator_config`,
`chapter_id_config`). Later jobs merge global + project values.

---

## 7. Stage 1 — Document analysis and mode selection

File: `comic_translator/document_analyzer.py`

### 7.1 Purpose

Manhwa is often published as very tall "webtoon" images. OCR and inpainting on
a single 1200×20000 image is both slow and memory-heavy. The analyzer decides
whether to treat the document as one long strip or as normal pages.

### 7.2 Algorithm

For every PDF page, read only its dimensions (`page.rect`, no rendering):

```
widths[i], heights[i] = page dimensions
avg_w = mean(widths)
avg_h = mean(heights)
avg_ar = avg_h / avg_w

areas[i] = widths[i] * heights[i]
size_consistency = std(areas) / mean(areas)      # coefficient of variation

long_strip_count = count(pages where height/width > long_strip_aspect_ratio_threshold)
long_strip_ratio = long_strip_count / total_pages

mode = LONG_STRIP
       if long_strip_ratio > 0.6 AND avg_ar > threshold
       else STANDARD
```

Default threshold: `1.5`. The 60% rule avoids flipping to long-strip mode
because of a few unusual pages.

### 7.3 Why this matters

| Mode | Page meaning | OCR strategy |
|---|---|---|
| `STANDARD` | `Page` = PDF page | per-page split + OCR + cross-page merge |
| `LONG_STRIP` + manga | `Page` = PDF page | falls back to standard path |
| `LONG_STRIP` + manhwa | `Page` = **display chunk**, not PDF page | one concatenated strip, chunked OCR |

**Teaching point:** in manhwa mode, the number of `Page` rows is the number of
*chunks*, not the number of PDF pages. This is intentional: the editor and
exporter work on manageable image slices, while the final WebP ZIP reassembles
them into one long strip.

---

## 8. Stage 2 — OCR pipeline

### 8.1 Standard mode (`detect_page`)

File: `ocr_pipeline/ocr_pipeline.py`, `ocr_pipeline/page_splitter.py`

```
detect_page(image, config, page_index):
    splitter = PageSplitter(max_chunk_height=config.max_chunk_height,
                            chunk_overlap=0)
    chunks = splitter.split(image)

    all_items = []
    for chunk in chunks:
        items = OCRProcessor.process_chunk(chunk.image, config,
                                           y_offset=chunk.y_offset)
        all_items.extend(items)

    items = DuplicateDetector.remove(all_items)
    dialogs = _cluster_items(items, config)       # DBSCAN + merge
    blocks = [Block.from_ocr(page_index, d) for d in dialogs]
    return PageResult(page, blocks, items)
```

#### 8.1.1 Chunking algorithm (`PageSplitter`)

```
if height <= max_chunk_height: return [whole image]

step = max_chunk_height - overlap
y = 0
while y < height:
    end = y + max_chunk_height
    if end >= height: emit (y, height); break
    emit (y, end)
    y += step
ensure last chunk reaches height
```

Overlap exists so a text line cut by a chunk boundary is fully visible in at
least one chunk. Duplicate detections from the overlap are removed later.
For standard per-page OCR the overlap is **0**; for manhwa concatenation it is
`manhwa_concat_overlap` (default **500 px**).

#### 8.1.2 OCR call (`ocr.py`)

```
_ocr_single_image(image, config):
    if max_side > config.ocr_max_side:
        downscale, remember scale
    contrast × 1.3
    save temporary JPEG (quality 95)
    result = PaddleOCR.predict(path)
    convert PaddleOCR 3.x dict → [(box, (text, score))]
    filter score < confidence_min or empty text
    map boxes back to original scale
    return [OCRItem(text, box, cx, cy, height, confidence)]
```

Why save to a temporary JPEG? PaddleOCR's Python API is most reliable with a
file path. The temp file is deleted in `finally`.

#### 8.1.3 Coordinate restoration (`CoordinateRestorer`)

For each chunk, add `chunk.y_offset` to every `y` coordinate and to `cy`.
After this step, all items live in **full-page coordinates**.

#### 8.1.4 Duplicate detection (`DuplicateDetector`)

Overlapping chunks detect the same line twice. The detector uses a greedy
NMS-like algorithm:

```
sort items by confidence descending
accepted = []
for candidate in sorted_items:
    if suppressed: continue
    accepted.append(candidate)
    for other in remaining:
        if is_duplicate(candidate, other):
            suppress(other)

is_duplicate(a, b):
    if IoU(a, b) >= iou_threshold: return True
    if center_distance(a, b) <= 30
       and text_similarity(a.text, b.text) >= 0.85
       and confidences within 30%: return True
    return False
```

Text similarity is normalized Levenshtein similarity:
`1 - edit_distance / max(len(a), len(b))`, after lowercasing, stripping
punctuation, and collapsing whitespace.

**Teaching point:** this is a multi-signal deduplicator, not just IoU. OCR boxes
jitter, so geometric overlap alone misses real duplicates.

#### 8.1.5 Clustering into speech blocks (`_cluster_items`)

Raw OCR returns **text lines**, but the user edits **speech bubbles**. We must
group lines that belong to the same bubble.

```
median_h = median(item.height)
points = [(item.cx / median_h, item.cy / median_h) for item in items]
labels = DBSCAN(eps=config.dbscan_eps, min_samples=1).fit_predict(points)
```

Normalizing by median line height makes `eps` scale-invariant: text in a large
image and in a small image clusters with the same parameter. Default `eps=2.5`
means "lines whose centers are within ~2.5 line heights".

For each cluster:

- bounding box = union of all line boxes;
- text = lines joined with `\n`, sorted by `cy` (reading order);
- `text_boxes` = every line polygon (kept for precise masking later).

Then `_merge_nearby_dialogs` applies **conservative** merging. ALL conditions
must pass:

1. horizontal overlap ≥ 30% of the narrower box;
2. vertical gap ≤ `max(20, median_h)`;
3. center distance ≤ `max(50, 3 * median_h)`;
4. line-height ratio ≥ 0.85 (similar font size);
5. angle difference ≤ threshold (disabled when threshold is 0);
6. the upper block's text does **not** end in terminal punctuation
   (`. ! ?`), i.e. it looks unfinished.

If merged, the text is concatenated with a newline and boxes are unioned.

### 8.2 Manhwa long-strip mode (`run_manhwa_concat_ocr`)

File: `ocr_pipeline/ocr_pipeline.py`

```
1. Render all non-skipped PDF pages at config.dpi.
2. Separate covers (excluded from OCR) from content.
3. Resize every content page to config.manhwa_concat_width (1200).
4. Concatenate vertically → ONE tall image.
5. Split tall image twice:
      OCR chunks:      height = standard_max_side (3000), overlap = 500
      storage chunks:  height = standard_max_side (3000), overlap = 0
6. OCR each OCR chunk, restore y offset into tall-image coordinates.
7. Deduplicate + DBSCAN cluster → blocks in tall-image coordinates.
8. Save tall.jpg + page_offsets.json + display chunks page-1.jpg…
```

Then `services.run_extract` remaps each block from tall-image coordinates to
chunk-local coordinates:

```
center_y = (block.y1 + block.y2) / 2
find chunk index i such that offsets[i] <= center_y < offsets[i] + height[i]
local_x = clamp(block.x, 0, chunk_width)
local_y = block.y - offsets[i]
drop block if local width/height < 4 px
```

Blocks are re-keyed as `p{chunk}_d{n}`. The tall strip is reassembled at export
time for the WebP ZIP.

**Teaching point:** this is a "global reasoning, local storage" design. OCR
sees the whole chapter at once (so bubbles spanning original page borders are
naturally whole), but the user edits and renders in small chunks.

### 8.3 Cross-page merge (standard mode only)

Files: `ocr_pipeline/cross_page_ocr.py`, `dialogue_merger.py`

Some bubbles start at the bottom of page N and continue at the top of page N+1.
After per-page OCR they appear as two blocks. The cross-page pass tries to join
them.

#### 8.3.1 Boundary gating (optimization)

Cross-page OCR is expensive, so it runs only for page pairs where at least one
page has a block touching the top/bottom boundary region:

```
boundary_region = 200 px
block qualifies if:
    (block.y1 < 200 or block.y2 > page_height - 200)
    and block.height >= 20 px
```

If neither page has such a block, the pair is skipped with a debug log.

#### 8.3.2 Strip OCR

```
bottom = crop bottom `cross_page_height` (600) of page N
top    = crop top `cross_page_height` of page N+1
resize both to common width
combined = vertical stack(bottom, top)
items = OCR(combined)

for item:
    if box crosses the seam:        → span (coordinates in page N space)
    elif item ends above seam:      → page N item
    else:                           → page N+1 item (shift y by -seam)
```

#### 8.3.3 Merge decision (all conditions required)

1. bottom block near page N bottom (≤ 100 px gap) and top block near page N+1
   top (≤ 100 px);
2. horizontal alignment (overlap or centers within 80 px);
3. height difference ≤ 30%;
4. not already merged (`continuation_page == 0`);
5. page-N text looks incomplete (no terminal punctuation);
6. if span detections exist, at least one span shares ≥ 40% of words
   (Jaccard) with **both** halves.

If merged:

- text = page N text + `\n` + page N+1 text (or the span text if it is longer
  and more complete);
- bounding box = union;
- `pages = [N, N+1]`, `continuation_page = N+1`;
- the merged block is attached to page N.

---

## 9. Stage 3 — Persistence

`_persist_blocks` writes one `TextBlock` per engine block:

| Engine field | DB field |
|---|---|
| `text` | `original_text` |
| `x1..y2` | `x1..y2` |
| `text_boxes` | `text_boxes` |
| `text_type` | `block_type` (validated against choices) |
| `translation_fa` | `translation` |
| loop index | `order` |

Then `_refresh_counts` recomputes `total_blocks` and `translated_count` from
actual rows. These counters gate the UI's Render button.

**Teaching point:** counters are denormalized for UI speed but are always
recomputed from the source of truth after mutations. Never trust a counter
after a crash or cancellation.

---

## 10. Stage 4 — Translation

File: `studio/services.py::run_translate`, `comic_translator/translator/`

### 10.1 Selection of work

```
query = blocks ordered by (page_number, order)
if block_ids given: restrict to those ids
if not force: only blocks with empty translation
if nothing: return immediately
```

### 10.2 Batching algorithm

```
chunk_size = config.translator.chunk_size  (default 12)
for each chunk:
    batch = {block.key: block.original_text}
    context = "Context: Page 3, 4. Previous dialogs: …"
    prompt = system_prompt + glossary lines
    result = provider.translate_batch(batch, prompt, context)
    for block in chunk:
        if result[key] non-empty: save translation
        append to previous-dialogs window (last 3)
        possibly update glossary
```

Why JSON? The model receives a dictionary `{key: text}` and must return the
same keys. This preserves the mapping between translation and box without
fragile text splitting. `parse_json_response` handles:

1. markdown code fences (` ```json … ``` `),
2. leading/trailing prose around the JSON object,
3. missing keys (filled with empty string),
4. malformed JSON → regex extraction of `"key": "value"` pairs.

### 10.3 Context and glossary

- **Context**: page numbers plus the last three translations. This helps the
  model keep pronouns and tone consistent across nearby bubbles.
- **Glossary**: names/terms appended to the system prompt as
  `English → Persian`. After each chunk, `_update_glossary` adds a mapping when:
  - translation is at most 8 words,
  - source looks like a single capitalized word,
  - translation is at most 3 words.

### 10.4 Provider behavior

| Provider | Endpoint | Fallback strategy |
|---|---|---|
| OpenRouter | `/api/v1/chat/completions` | Try configured model, then up to 7 free chat models; honor `Retry-After` on 429 |
| Gemini | Google Generative Language API | Try configured model, then `gemini-2.0-flash`, `-flash-lite`, `1.5-flash` |
| Custom | Any OpenAI-compatible `/chat/completions` | 3 attempts (LM Studio, Ollama, vLLM, relays) |

### 10.5 Partial failure policy

A chunk failure is logged and the loop continues. If **zero** blocks were
translated overall, the job is marked failed with an actionable error message
(check key/endpoint). This is better than failing the whole job on one
rate-limited chunk while other chunks succeed.

---

## 11. Stage 5 — Manual editing (the studio)

### 11.1 Frontend pieces

| File | Responsibility |
|---|---|
| `app/projects/[id]/page.tsx` | Workspace: page list, panels, job bar, polling |
| `components/workspace/CanvasEditor.tsx` | Rendering boxes, drag/resize/rotate, inline text editing, brush/eraser/crop tools |
| `components/workspace/usePreviewLayout.ts` | Debounced call to backend layout preview |
| `components/workspace/useHistory.ts` | Undo/redo stack with coalescing |
| `lib/block-geometry.ts` | Pure resize math (axis-aligned and rotated) |
| `lib/api.ts` | Fetch wrapper with CSRF + error normalization |

### 11.2 Editor interaction algorithm

- **Move**: pointer delta in image pixels is added to the block's bbox.
- **Resize**: `resizeRectHandle` keeps the opposite side pinned and enforces a
  minimum size. For rotated boxes, `resizeRotatedHandle` projects the pointer
  delta onto the box's local axes before resizing, then recomputes the center so
  the pinned edge stays fixed.
- **Inline editing**: double-click opens a textarea. `Tab` commits and moves to
  the next/previous dialog.
- **Brush**: collects points (min distance 3 px), simplifies/downsamples to
  ≤ 950 centerline points, converts the stroke to a polygon (~2× points), and
  stores it as a `MaskMark`.
- **Eraser**: deletes mask marks whose bbox is within 10 px of the stroke.

### 11.3 Live preview contract

The editor must show exactly what the renderer will draw. It sends draft blocks
to `POST /api/projects/:id/preview-layout`. The backend calls the **same**
`resolve_style` + `text_layout` functions as the renderer and returns:

```
{
  block_id: {
    font_size, font_url, align, rotation,
    lines: [{ text, x, y }, …]
  }
}
```

The browser loads the font via `FontFace`, then draws each line at the returned
position. This avoids duplicating Persian shaping/measuring logic in JavaScript.

**Teaching point:** one measurement implementation, two consumers. This is the
single best defense against "preview looks right, export looks wrong".

### 11.4 Geometry ↔ mask consistency

When a block is moved/resized, `TextBlocksView.partial_update` rescales the
stored OCR `text_boxes` polygons by the same factors:

```
sx = new_width / old_width
sy = new_height / old_height
new_point = (new_x1 + (old_x - old_x1) * sx,
             new_y1 + (old_y - old_y1) * sy)
```

Without this, inpainting would erase the old location while the text is drawn
at the new location.

---

## 12. Stage 6 — Inpainting (erase original text)

Files: `studio/services.py::_inpaint_pages`, `comic_translator/inpaint.py`,
`inpainters/`

### 12.1 Which regions are erased?

A region is erased when:

- the block has a non-empty translation, **and**
- the block is **not** `sfx` (sound effects are intentionally preserved), **and**
- plus every freehand `MaskMark` polygon on that page.

If a page has neither, it is skipped entirely.

### 12.2 Mask building (the three-mask architecture)

`inpainters/common.py` distinguishes:

| Mask | Purpose | Passed to model? |
|---|---|---|
| **Core mask** | Exact text pixels, from OCR polygons (or bbox fallback) | No |
| **Inference mask** | Core dilated by a few px (anti-aliasing + context) | Yes, binary |
| **Blend mask** | Feathered mask for final compositing only | No |

Invariant: **neural models receive binary masks**; feathering is only used when
compositing the result back into the page (1–2 px) to avoid halos.

Polygons are preferred over rectangles because speech bubbles are often curved
or overlapping artwork. When a block is moved, its polygons move with it
(section 11.4).

### 12.3 Backend selection and fallback

```
_create_backend(method):
    try requested method
    else fallback chain:
        cv_auto  → opencv → color
        pcleaner → opencv → color
    cache backend by (method, device, pcleaner config)
```

The job log explicitly reports if the requested backend was unavailable and a
fallback was used. This is important: silent quality degradation is a bug.

### 12.4 `cv_auto` — the default algorithm

File: `inpainters/cv_auto.py`

This is the most interesting algorithm in the project. It analyzes the local
background before deciding how to fill.

```
build combined mask from all regions
components = connected components of the mask

for each component:
    find overlapping original text bboxes → anchor box
    ring_thickness = clamp(3% of max(anchor size), 2, 24)
    ring = pixels in a ring just outside the anchor box
    (modal_color, fraction) = dominant quantized color of ring

    if fraction >= solid_ratio (0.85):
        # background is a flat color (typical speech bubble)
        fill component with modal_color
        blend edges with a feathered mask
    else:
        # complex background (artwork, gradient, texture)
        try SimpleLaMa on a 512×512 crop:
            scale crop uniformly to fit 512, mirror-pad remainder
            run model, crop back, resize to original
        else if hole <= 60,000 px:
            push-in fill: repeatedly copy colors from filled neighbors
                          into the hole boundary (texture flows inward)
        else:
            cv2.inpaint (Telea) because push-in would be too slow
```

Why the ring, not the interior? The interior contains the text itself. The
ring just outside the text bbox is (usually) pure bubble background, so it is
ground truth for the fill color.

Why component-by-component? Two bubbles on different backgrounds need different
fill decisions.

Why push-in instead of blur? Blur produces a visible smudge on halftone or
line-art. Push-in copies actual neighboring pixels inward, preserving texture.

### 12.5 Other backends

| Backend | Algorithm | When |
|---|---|---|
| `color` | Fill with sampled interior/background color | Fastest, lowest quality; also final fallback |
| `opencv` | Navier–Stokes (`ns`) or Telea | Always available |
| `pcleaner` | External PanelCleaner CLI: text detection + masking + inpainting | If installed |

### 12.6 Service-level fallback

If `inpaint_batch` raises for a page, `_inpaint_pages` catches it, logs the
error, and calls `erase_region` with color fill for each region. The job
continues; the page is still usable.

---

## 13. Stage 7 — Rendering Persian text

File: `comic_translator/renderer.py`

### 13.1 Style resolution

`resolve_style(text_type, config, overrides…)` merges, in order:

1. **Type defaults** (`TYPE_DEFAULTS`):
   - `dialog`: Mikhak, 100% size, centered
   - `sfx`: AFSANEH, 100%
   - `think`: AFSANEH, 85%, thinner stroke
   - `action`: Mikhak, 120%, thicker stroke
   - `whisper`: Vazir, 80%, gray
   - `narration`: Mikhak, RTL/right alignment
   - `system`: Vazir, 90%, blue
2. **Per-block overrides**: color, stroke color/size, alignment, font, max size.
3. **Global font config**: size bounds and default stroke width.

### 13.2 Font fitting algorithm

```
text_layout(text, box_w, box_h, …):
    font_path = explicit override or find_font(type)
    effective_max = max(size_min, size_max * size_factor or block override)
    font, lines, size = fit_font_size(text, font_path, box_w, box_h,
                                      effective_max, size_min)
    measure multiline bbox with the final font
    return font, lines, size, spacing, bbox, alignment
```

`fit_font_size`:

```
size = size_max
while size > size_min:
    font = load_font(path, size)
    lines = wrap(text, font, box_w - 8)
    estimated_height = spacing*(n-1) + n*size*2
    if estimated_height <= box_h: return
    size -= 2
return size_min (hard floor, even if it overflows)
```

Design choice: **start at the user's configured maximum** and shrink only when
needed. The typography specification says font reduction should be a last
resort; this implementation encodes that as "start big, shrink gradually".

### 13.3 Wrapping

`_wrap_text` is greedy word wrapping based on **measured pixel width**, not
character count, and preserves explicit `\n` breaks. It does not hyphenate.

### 13.4 Text color algorithm

Priority:

1. per-block explicit color;
2. type default fixed color (e.g. whisper gray, system blue);
3. original-text color sampled **before inpainting**;
4. black/white contrast fallback.

Original color sampling (`_sample_original_text_color`):

```
estimate background color (interior if uniform, else surrounding ring)
restrict pixels to OCR text polygons (or inset bbox)
keep only "ink" pixels: |pixel - background| sum > 90
quantize ink colors, return the most frequent bucket
```

Then `_adjust_color_contrast` ensures WCAG contrast ≥ 3.0 against the cleaned
background, changing **lightness only** (preserving hue) in small steps. If no
lightness reaches the target, black or white is chosen by luminance.

### 13.5 Stroke

If stroke width > 0:

- use explicit stroke color if provided;
- otherwise pick white for dark text and black for light text.

This keeps text readable over artwork.

### 13.6 Rotation and vertical text

- **Rotation**: draw text on a transparent layer of the box size, rotate by
  `-rotation` (PIL is counter-clockwise; the UI is clockwise), paste centered.
  Rotating the whole box preserves alignment better than rotating a tight glyph
  crop.
- **Vertical**: draw normally on a transparent layer, rotate `-90°`, paste
  centered. (True vertical Persian column typesetting is out of scope.)

> Note for advanced students: `_reshape_persian` is currently a no-op. Correct
> Persian shaping comes from Pillow's FreeType build with libraqm support. If
> glyphs appear disconnected, the first thing to check is the Pillow/libraqm
> build, not this function.

### 13.7 Chapter ID stamp

`draw_chapter_id` tries corners in order: top-right, top-left, bottom-right,
bottom-left. It chooses the first that does not overlap any rendered text box.
This is a small but nice example of constraint-based placement.

### 13.8 Memory strategy for tall pages

`render_on_tall_image` groups pages into chunks of
`config.font.tall_render_chunk_size` (default 3), concatenates only that chunk,
renders regions intersecting it, splits it back, and frees the chunk. Only a
fraction of the chapter is in memory at once. `_np_array_cache` is cleared
after each chunk because `id()` values can be reused.

---

## 14. Stage 8 — Export

File: `studio/services.py::run_apply`, `comic_translator/pdf.py`,
`psd_export.py`

### 14.1 Apply pipeline

```
1. Build engine blocks from DB + translations map.
2. For each page, reuse rendered/page-N.jpg if it exists (Export must not redo
   work already previewed). Only pages without a rendered image are processed.
3. Load page images (from pages/ or re-render from PDF if missing).
4. Inpaint regions that will receive text (progress 5–40%).
5. Render text (progress 45–90%).
6. Save rendered/page-N.jpg previews.
7. Export requested formats.
8. project.status = DONE
```

Important subtlety: `render_page_list` maps images **positionally**
(`page_num = index + 1`). Because `run_apply` may process only a subset of
pages, it renumbers the pending subset into a temporary `sub_map` so block
lookups stay aligned.

### 14.2 Output formats

| Format | How | Notes |
|---|---|---|
| **PDF** | Each image → JPEG quality 75 → PyMuPDF page | One PDF page per pipeline page |
| **ZIP (normal)** | Resize each page to 1200 wide, WebP quality 95, split height > 8192 | Files `page_001.webp`, … |
| **ZIP (webtoon)** | Concatenate all rendered chunks into one tall image, then WebP ZIP | Preserves the long-strip reading experience |
| **PSD** | One PSD per page: `Background` (cleaned page) + one RGBA text layer per translated block + optional `Chapter` stamp | Layers named by block key; page resized to 1200 wide |

### 14.3 PSD layer algorithm

```
resize page to 1200 wide; scale all block coordinates by the same factor
PSD = new RGB document
add Background layer   ← the CLEANED page (inpainted, no Persian text)
for each translated block:
    render only that text on a transparent RGBA layer
    crop the layer to its non-transparent bbox (keep the offset)
    add it as a layer named by block key
if chapter_id is set:
    draw the chapter stamp on its own transparent layer ("Chapter")
save all PSDs into a ZIP
```

The background **must** be the cleaned page, not the rendered page. Using
`rendered/page-N.jpg` here bakes the Persian text into the background *and*
draws it again as text layers, so the export looks doubled and the layers
cannot be hidden cleanly.

`run_apply` therefore keeps the in-memory cleaned images from the inpaint
step and passes those to `create_page_psd`. For pages whose rendered preview
was reused, it loads `cleaned/page-N.jpg`; if that file does not exist, it
rebuilds the cleaned art by inpainting before creating the PSD.

The per-block layers are **raster** RGBA layers (pixel data), not native PSD
type layers: `psd-tools` can read text layers but has no public API for
authoring them. Editors can still move, hide, or restyle them as images.

---

## 15. Configuration system

File: `studio_engine/config.py`, `studio/services.py`

### 15.1 Two levels

```
GlobalSetting("config")  ──►  defaults for every new project
        │
        └──► copied into Project.{ocr,font,inpaint,translator,chapter_id}_config
                          │
                          └──► merged at job time with global (project wins)
```

Merge rule (`_get_config`):

- nested dicts merge key-by-key;
- an **empty string** in a project snapshot does not overwrite a non-empty
  global value (so adding an API key later still applies to old projects);
- everything is rebuilt into immutable dataclasses by `_build_config`.

### 15.2 Environment precedence

`OPENROUTER_API_KEY`, `GEMINI_API_KEY`, `CUSTOM_*`, `OCR_DPI`,
`INPAINT_METHOD`, `TRANSLATOR_PROVIDER` override file/DB values when non-empty.

### 15.3 Security hardening

`PUT /api/config` accepts raw JSON from the browser. Before merging:

1. **whitelist** keys derived from the engine dataclasses (`_allowed_config_sections`);
2. **prune** unknown keys (`_prune_config`);
3. **validate paths** (`_validate_config_paths`): font paths must be inside
   `FONTS_DIR`; a PanelCleaner CLI path must be an existing file.

Without this, a client could set `inpaint.pcleaner_cli_path` to an arbitrary
executable. This is a good teaching example of "never trust a config blob".

---

## 16. Concurrency, consistency, caching

### 16.1 Locks

| Lock | Scope | Protects |
|---|---|---|
| `_project_locks[id]` | One process | One heavy job per project |
| `_locks_guard` | One process | Lock map creation |
| `_cancel_guard` | One process | Cancel-event map |
| `_lama_lock` | One process | Lazy SimpleLaMa construction |
| `_gemini_global_lock` | One process | `genai.configure()` is process-global |

### 16.2 Transactions

- Project deletion and block persistence use `transaction.atomic()`.
- The extraction wipe is transactional; the long OCR phase is deliberately
  **outside** the transaction so SQLite is not locked for minutes.
- File cleanup that must follow a DB commit uses `transaction.on_commit`.

### 16.3 Caches

| Cache | Key | Invalidation |
|---|---|---|
| `_ocr_cache` | OCR settings tuple | Process lifetime (settings change → new key) |
| `_backend_cache` | method + device + pcleaner settings | Process lifetime |
| `_font_cache` | (path, size) | Process lifetime; failures are **not** cached |
| `_np_array_cache` | `id(image)` + identity check | Explicit invalidation after drawing/erasing; cleared after tall chunks |
| `_free_models` | Provider instance | Per instance |

**Teaching point:** caching by `id()` is dangerous because Python reuses ids
after garbage collection. The code stores `(image_reference, array)` and checks
identity before using the cache — a pattern worth discussing.

---

## 17. Failure modes and debugging guide

| Symptom | Likely cause | Where to look |
|---|---|---|
| Extract job fails immediately | AI dependencies missing (`paddleocr`, `paddlepaddle`, `PyMuPDF`) | job `error_message`; `requirements-ai.txt` |
| OCR finds nothing | Page is a cover/skipped, confidence thresholds too high, image too small | `confidence_min`, `text_rec_score_thresh`, `ocr_det_*`; re-OCR with `high`/`ultra` |
| Text lines split into many blocks | DBSCAN `eps` too small; merge rules too strict | `dbscan_eps`, `_merge_nearby_dialogs` |
| One bubble became two blocks across pages | Cross-page gate skipped or merge criteria failed | `boundary_region_height`, `enable_cross_page_merge`, `DialogueMerger` rules |
| Persian text in wrong place after moving a box | `text_boxes` not rescaled | `TextBlocksView.partial_update` geometry branch |
| Original text still visible after clean | Inpaint backend fell back to color fill; mask too small; region not translated | job log `Inpaint backend` / `inpaint FAILED`; `padding_x/y` |
| Smudged/ghosted bubble after clean | Complex background went through push-in/OpenCV; LaMa not installed | install `simple_lama_inpainting`; check `cv_auto` log lines |
| Persian glyphs disconnected or tofu boxes | Font lacks glyphs, or Pillow built without libraqm; wrong font override | `backend/fonts/`, `FONT` env, Pillow build |
| Preview and export differ | Preview used stale block state or a different font | `preview-layout` payload; `usePreviewLayout` debounce; font URL |
| Translation returns nothing | Bad API key/model/endpoint; JSON parse failure | job log per-chunk errors; `parse_json_response`; Settings → Test Provider |
| Job stuck at running forever | Server restarted mid-job (zombie) | `GET /api/jobs/<id>` heals it; `settle_zombie` |
| `database is locked` | SQLite contention between worker and request | `_db_write_retry`; avoid long transactions |
| Out of memory on webtoon | Huge concatenated strip | reduce `standard_max_side` / `manhwa_concat_width`; tall render chunk size |
| PDF locked on Windows | Open PyMuPDF handle | all `render_page` paths close `doc` in `finally` |

### Debugging workflow for students

1. Open the job log in the UI. Every stage logs page numbers, counts, and
   backend names.
2. Reproduce with **one page** (`reocr`, `clean?page_number=N`,
   `render?page_number=N`).
3. Check the files, not just the DB:
   `pages/page-N.jpg`, `cleaned/page-N.jpg`, `rendered/page-N.jpg`.
4. Compare coordinates: overlay the block bbox in the editor with the actual
   image. If the box is right but the text is wrong, the bug is in rendering.
   If the box is wrong, the bug is in OCR or coordinate restoration.
5. Only then read code, starting from the log line's stage.

---

## 18. Complexity and performance notes

| Stage | Typical complexity | Practical note |
|---|---|---|
| PDF rendering | O(pages) | PyMuPDF is fast; DPI dominates cost |
| Chunk OCR | O(total pixels / chunk area) | PaddleOCR dominates wall time |
| DBSCAN | ~O(n²) worst case | n = text lines per page, usually < 200 |
| Duplicate removal | O(n²) + Levenshtein O(L²) on close pairs | Fine for page-sized inputs |
| Cross-page merge | O(candidates²) per page pair | Candidates are only blocks near the boundary |
| Translation | O(blocks / chunk_size) API calls | Network latency dominates; chunking reduces calls |
| Inpainting | O(components × region pixels) | Push-in fill is the slow path; LaMa is GPU-friendly |
| Rendering | O(regions × text layout) | Font fitting loops from max size downward |
| Export | O(pages × pixels) | WebP/PDF compression dominates |

Memory rules of thumb:

- Never hold the entire chapter as one image unless you must (manhwa OCR does,
  at 1200 px wide — that is the compromise).
- Render tall pages in chunks (`tall_render_chunk_size`).
- Free numpy caches after large operations.

---

## 19. Whiteboard summary (one slide)

```
PDF ──► analyze ──► standard? ──yes──► per-page OCR ──► dedup ──► cluster ──► merge
                        │                                             │
                        no (manhwa)                                   │
                        ▼                                             │
              concat tall strip ──► chunk OCR ──► remap to chunks     │
                        │                                             │
                        └──────────────┬──────────────────────────────┘
                                       ▼
                              TextBlocks in SQLite
                                       │
                     ┌─────────────────┼─────────────────┐
                     ▼                 ▼                 ▼
                auto-translate    manual editor     re-OCR page
                     │                 │                 │
                     └─────────────────┼─────────────────┘
                                       ▼
                       inpaint translated non-SFX regions + marks
                                       ▼
                         render Persian text (fit/color/stroke)
                                       ▼
                            PDF / WebP ZIP / layered PSD
```

Three invariants to memorize:

1. **One coordinate space** — everything is in `pages/page-N.jpg` pixels.
2. **One job per project** — background threads + per-project lock.
3. **One layout implementation** — preview and render call the same functions.

---

## 20. Classroom exercises

### Exercise 1 — Trace a page (beginner)

Pick one page and write down, for each stage, the exact data structure:

```
PDF page → PIL Image (W×H) → chunks [(y_offset, Image)] → OCRItems
→ deduped OCRItems → OCRDialog → Block → TextBlock → inpainted image
→ rendered image
```

Include one concrete coordinate and show how it changes (or does not) at each
step.

### Exercise 2 — Break the deduplicator (intermediate)

Construct three OCR items that a naive IoU-only detector would keep but the
multi-criteria detector removes. Explain which condition fires.

### Exercise 3 — Why JSON? (intermediate)

Explain what would go wrong if translations were returned as a plain list of
strings in reading order. Give two concrete failure cases (empty translation,
model merging two lines).

### Exercise 4 — Inpaint decision (advanced)

For a bubble on a white background and a bubble over a gradient sky, walk
through `cv_auto` and state which branch runs, what the ring fraction is
approximately, and what the fallback is if LaMa is missing.

### Exercise 5 — Add a feature (advanced)

Add a `clean?page_number=N` button that only cleans the current page, end to
end:

1. frontend button → API call;
2. view → `submit_job(CLEAN, {page_number})`;
3. service already supports it — verify;
4. UI polls the job and refreshes the cleaned image with a cache-busting
   revision string.

### Exercise 6 — Debug a deliberately broken invariant (advanced)

Change `_persist_blocks` to store coordinates in the chunk-local space for
standard pages. Predict the visible symptoms in the editor, clean, and render.
Then fix it and explain why the bug was hard to see on page 1.

---

## 21. Glossary

| Term | Meaning |
|---|---|
| **Block** | One speech bubble / text region the user edits |
| **Chunk** | A horizontal slice of a tall page used to bound OCR/render memory |
| **Long strip** | Webtoon-style very tall page |
| **Inpainting** | Filling a masked region with plausible background |
| **Mask** | Binary image marking pixels to erase/fill |
| **Core / inference / blend mask** | Exact text pixels / dilated model input / feathered compositing mask |
| **IoU** | Intersection over Union of two boxes |
| **DBSCAN** | Density-based clustering; groups text lines into bubbles |
| **Levenshtein ratio** | Text similarity based on edit distance |
| **RTL** | Right-to-left script rendering (Persian) |
| **Zombie job** | A job row left active after its worker process died |
| **Snapshot config** | Per-project copy of global settings taken at creation |

---

## 22. Quick file-to-concept index

| Concept | File |
|---|---|
| Project/page/block/job schema | `backend/studio/models.py` |
| Job threads, locks, cancellation | `backend/studio/jobs.py` |
| All pipeline orchestration | `backend/studio/services.py` |
| REST endpoints | `backend/studio/views.py` |
| Config merge + whitelist | `backend/studio/services.py` (`_get_config`, `_allowed_config_sections`) |
| Engine config dataclasses | `backend/studio_engine/comic_translator/config.py` |
| Block/OCR data classes | `backend/studio_engine/comic_translator/models.py` |
| PDF render + export | `backend/studio_engine/comic_translator/pdf.py` |
| Mode detection | `backend/studio_engine/comic_translator/document_analyzer.py` |
| Chunking | `backend/studio_engine/comic_translator/ocr_pipeline/page_splitter.py` |
| Dedup | `backend/studio_engine/comic_translator/ocr_pipeline/duplicate_detector.py` |
| Clustering | `backend/studio_engine/comic_translator/ocr.py` (`_cluster_items`) |
| Cross-page OCR | `backend/studio_engine/comic_translator/ocr_pipeline/cross_page_ocr.py` |
| Cross-page merge | `backend/studio_engine/comic_translator/ocr_pipeline/dialogue_merger.py` |
| OCR orchestrator | `backend/studio_engine/comic_translator/ocr_pipeline/ocr_pipeline.py` |
| Translation providers | `backend/studio_engine/comic_translator/translator/` |
| Inpaint dispatch | `backend/studio_engine/comic_translator/inpaint.py` |
| cv_auto algorithm | `backend/studio_engine/comic_translator/inpainters/cv_auto.py` |
| Mask construction | `backend/studio_engine/comic_translator/inpainters/common.py` |
| Rendering | `backend/studio_engine/comic_translator/renderer.py` |
| Typography rules | `backend/studio_engine/comic_translator/typography.py` |
| PSD export | `backend/studio_engine/comic_translator/psd_export.py` |
| Editor canvas | `frontend/components/workspace/CanvasEditor.tsx` |
| Preview layout hook | `frontend/components/workspace/usePreviewLayout.ts` |
| Undo/redo | `frontend/components/workspace/useHistory.ts` |
| Geometry math | `frontend/lib/block-geometry.ts` |
| API client | `frontend/lib/api.ts` |

---

*End of document.*
