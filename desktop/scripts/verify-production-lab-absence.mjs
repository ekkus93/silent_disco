import { readdir, readFile } from "node:fs/promises";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const distRoot = fileURLToPath(new URL("../dist/", import.meta.url));
const forbidden = [
  "Developer testing tool. Multi-node scenarios run",
  "lab_get_state",
  "lab_open_scenario_file",
  "lab_save_scenario_file",
  "lab_set_link_faults",
  "lab_run_loaded_scenario",
  "lab_pause_loaded_scenario",
  "lab_resume_loaded_scenario",
  "lab_advance_virtual_time",
  "lab_start_node",
  "lab_stop_node",
  "lab_stop_all_nodes",
  "lab_export_recording_file",
];

async function filesBelow(directory) {
  const entries = await readdir(directory, { withFileTypes: true });
  const files = [];
  for (const entry of entries) {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) files.push(...(await filesBelow(path)));
    else files.push(path);
  }
  return files;
}

const files = (await filesBelow(distRoot)).filter((path) => /\.(?:html|js|css)$/.test(path));
if (files.length === 0) {
  throw new Error("production bundle verification found no HTML/JS/CSS output");
}

for (const path of files) {
  const text = await readFile(path, "utf8");
  for (const marker of forbidden) {
    if (text.includes(marker)) {
      throw new Error(
        `production bundle contains Lab-only marker ${JSON.stringify(marker)} in ${path}`,
      );
    }
  }
}

console.log(`production bundle excludes Lab-only frontend code (${files.length} files checked)`);
