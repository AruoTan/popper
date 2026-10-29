import { createRequire } from "node:module";
import { spawn } from "node:child_process";
import { projectRoot, writeVersion } from "./version.mjs";

// Generate the JSON before the CLI parses the Tauri version-file reference.
writeVersion();
const require = createRequire(import.meta.url);
const child = spawn(process.execPath, [require.resolve("@tauri-apps/cli/tauri.js"), ...process.argv.slice(2)], {
  cwd: projectRoot,
  stdio: "inherit",
});
child.on("error", (error) => { console.error(error); process.exitCode = 1; });
child.on("exit", (code, signal) => {
  if (signal) process.kill(process.pid, signal);
  else process.exitCode = code ?? 1;
});
