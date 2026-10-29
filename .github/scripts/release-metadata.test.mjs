import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import { cpSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { projectRoot } from "../../scripts/version.mjs";

test("release metadata requires the exact source tag and pins it across build jobs", (t) => {
  const root = mkdtempSync(join(tmpdir(), "popper-release-version-"));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  const git = (...args) => execFileSync("git", args, { cwd: root, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] }).trim();
  git("init");
  git("config", "user.name", "Release tests");
  git("config", "user.email", "tests@example.invalid");
  for (const path of ["scripts/version.mjs", ".github/scripts/release-metadata.mjs"]) {
    mkdirSync(join(root, path, ".."), { recursive: true });
    cpSync(join(projectRoot, path), join(root, path));
  }
  writeFileSync(join(root, ".gitignore"), ".popper-version.json\n");
  git("add", ".");
  git("commit", "-m", "Source");
  git("tag", "-a", "v3.2.1", "-m", "Release");
  const commit = git("rev-parse", "HEAD");
  const run = (env = {}) => spawnSync(process.execPath, [join(root, ".github/scripts/release-metadata.mjs")], {
    cwd: root, encoding: "utf8", env: { ...process.env, GITHUB_SHA: commit, RELEASE_VERSION: "3.2.1", RELEASE_TAG: "v3.2.1", ...env },
  });
  const result = run();
  assert.equal(result.status, 0, result.stderr || result.error?.message);
  assert.equal(result.stdout, "version=3.2.1\ntag=v3.2.1\n");
  assert.match(run({ GITHUB_SHA: "0".repeat(40) }).stderr, /workflow commit/u);
  assert.match(run({ RELEASE_VERSION: "3.2.2" }).stderr, /changed during the release/u);
  assert.match(run({ RELEASE_TAG: "v3.2.2" }).stderr, /changed during the release/u);
  git("commit", "--allow-empty", "-m", "After release");
  assert.match(run({ GITHUB_SHA: git("rev-parse", "HEAD") }).stderr, /exactly to HEAD/u);
});
