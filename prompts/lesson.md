# Your task: write lesson {n} of {total}

The course plan is below. Write lesson {n}, "{title}", as JSON matching the schema.

<<<PLAN
{plan}
PLAN>>>

- Teach exactly this lesson's objectives and key ideas. Assume the reader has done the earlier lessons; don't repeat them, and don't teach later lessons.
- The prose comes first, then the figure if there is one, then the lab. End the prose by leading into the lab: tell the reader what to try and what to watch for.
- Build the planned lab; you may improve on the idea. It must make sense without reading its code.
- `figure` is "{figure}" as planned, or '' if it wouldn't help. `figure_note` says what to look at in it.

## JSON structure (field names are exact)

Reply with only the JSON object, no markdown fences, no commentary. The object has exactly these fields:

- `prose_html`: string. The lesson's explanation as an HTML fragment: use `<p>`, `<h3>` sparingly, inline `<em>`/`<strong>`, and KaTeX math in `\(...\)` / `\[...\]` where it helps. No `<script>`, no whole-page markup, no markdown.
- `figure`: string. The figure key planned for this lesson (e.g. `F1`), or `''` if none.
- `figure_note`: string. If `figure` is set: one sentence telling the reader what to look at in it. Otherwise `''`.
- `lab_html`: string. The lab's markup: one `<div id="lab">` containing the controls (sliders, buttons, selects, canvas) as planned. Never empty, never `''`.
- `lab_css`: string. Extra CSS for the lab only, or `''`. No global page styles.
- `lab_js`: string. The lab's script: plain browser JavaScript in an IIFE, using the helpers described in the course plan (plots, heatmaps, sliders). Compute the output from the controls; never hardcode results. No `document.write`, no network calls, no external files.

Do not add, rename, or omit any field. Do not wrap the JSON in backticks or a code block. Escape HTML and JS inside string values as valid JSON.
