import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import { cpSync, mkdirSync, mkdtempSync, readFileSync, realpathSync, rmSync, statSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { pathToFileURL } from "node:url";
import { projectRoot, resolveVersion, versionFile, writeVersion } from "./version.mjs";

function git(root, ...args) {
  return execFileSync("git", args, { cwd: root, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] }).trim();
}

function fixture(t) {
  const temporary = mkdtempSync(join(tmpdir(), "popper-version-"));
  t.after(() => rmSync(temporary, { recursive: true, force: true }));
  const root = join(temporary, "source");
  mkdirSync(root);
  git(root, "init", "-b", "main");
  git(root, "config", "user.name", "Version tests");
  git(root, "config", "user.email", "tests@example.invalid");
  writeFileSync(join(root, ".gitignore"), `${versionFile}\n`);
  git(root, "add", ".gitignore");
  git(root, "commit", "-m", "Initial source");
  return { root, temporary };
}

test("lightweight and annotated tags determine version without source edits", (t) => {
  const { root } = fixture(t);
  git(root, "tag", "v2.3.4");
  assert.deepEqual(resolveVersion(root, { release: true }), {
    version: "2.3.4", tag: "v2.3.4", commit: git(root, "rev-parse", "HEAD"), distance: 0,
  });
  git(root, "commit", "--allow-empty", "-m", "Next release");
  git(root, "tag", "-a", "v2.3.5", "-m", "Release");
  assert.equal(resolveVersion(root, { release: true }).version, "2.3.5");
});

test("development uses the nearest reachable tag, not the highest tag elsewhere", (t) => {
  const { root } = fixture(t);
  git(root, "tag", "v2.3.4");
  git(root, "checkout", "-b", "unrelated-release");
  git(root, "commit", "--allow-empty", "-m", "Other branch");
  git(root, "tag", "v9.0.0");
  git(root, "checkout", "main");
  git(root, "commit", "--allow-empty", "-m", "Development");
  assert.equal(resolveVersion(root).version, "2.3.4");
  assert.equal(resolveVersion(root).distance, 1);
  assert.throws(() => resolveVersion(root, { release: true }), /exactly to HEAD/u);
});

test("unversioned and ambiguous commits fail rather than invent a version", (t) => {
  const { root } = fixture(t);
  for (const tag of ["release", "v01.2.3", "v2.3.4-rc.1"]) git(root, "tag", tag);
  assert.throws(() => resolveVersion(root), /No reachable/u);
  git(root, "tag", "v2.3.4");
  git(root, "tag", "v2.3.5");
  assert.throws(() => resolveVersion(root), /Multiple version tags/u);
});

test("release rejects source modifications and untracked files", (t) => {
  const { root } = fixture(t);
  git(root, "tag", "v2.3.4");
  writeFileSync(join(root, "unexpected.txt"), "uncommitted");
  assert.throws(() => resolveVersion(root, { release: true }), /clean working tree/u);
  assert.equal(resolveVersion(root).version, "2.3.4");
  rmSync(join(root, "unexpected.txt"));
  writeFileSync(join(root, ".gitignore"), "modified\n");
  assert.throws(() => resolveVersion(root, { release: true }), /clean working tree/u);
});

test("generated metadata is ignored, stable, and never overrides Git", (t) => {
  const { root } = fixture(t);
  git(root, "tag", "v2.3.4");
  writeVersion(root);
  const before = statSync(join(root, versionFile)).mtimeMs;
  writeVersion(root);
  assert.equal(statSync(join(root, versionFile)).mtimeMs, before);
  assert.equal(git(root, "status", "--porcelain"), "");
  git(root, "commit", "--allow-empty", "-m", "New version");
  git(root, "tag", "v2.4.0");
  assert.equal(resolveVersion(root, { release: true }).version, "2.4.0");
  assert.equal(writeVersion(root).version, "2.4.0");
});

test("source exports preserve provenance even inside a different repository", (t) => {
  const { root, temporary } = fixture(t);
  git(root, "tag", "v2.3.4");
  const archive = join(root, "archive");
  mkdirSync(archive);
  const expected = resolveVersion(root);
  writeVersion(archive, expected);
  assert.deepEqual(resolveVersion(archive), expected);
  assert.throws(() => resolveVersion(archive, { release: true }), /Git checkout/u);
  const standalone = join(temporary, "standalone");
  cpSync(archive, standalone, { recursive: true });
  assert.deepEqual(resolveVersion(standalone), expected);
});

test("missing or malformed export provenance cannot supply an arbitrary version", (t) => {
  const { temporary } = fixture(t);
  const root = join(temporary, "archive");
  mkdirSync(root);
  assert.throws(() => resolveVersion(root), /No Git checkout/u);
  for (const metadata of [
    { version: "2.3.5", tag: "v2.3.4", commit: "a".repeat(40), distance: 0 },
    { version: "2.3.4", tag: "v2.3.4", commit: "unknown", distance: 0 },
    { version: "2.3.4", tag: "v2.3.4", commit: "a".repeat(40), distance: -1 },
  ]) {
    writeFileSync(join(root, versionFile), JSON.stringify(metadata));
    assert.throws(() => resolveVersion(root), /Invalid Git-derived/u);
  }
});

test("shallow clones fail with instructions to fetch full history", (t) => {
  const { root, temporary } = fixture(t);
  git(root, "tag", "v2.3.4");
  const shallow = join(temporary, "shallow");
  git(temporary, "clone", "--depth=1", pathToFileURL(root).href, shallow);
  assert.throws(() => resolveVersion(shallow), /git fetch --unshallow --tags/u);
});

test("linked worktrees derive their own commit and share version tags", (t) => {
  const { root, temporary } = fixture(t);
  git(root, "tag", "v2.3.4");
  const worktree = join(temporary, "worktree");
  git(root, "worktree", "add", "--detach", worktree, "HEAD");
  assert.equal(resolveVersion(worktree, { release: true }).version, "2.3.4");
});

test("repository aliases resolve the same release and preserve nested export boundaries", (t) => {
  const { root, temporary } = fixture(t);
  git(root, "tag", "v2.3.4");
  const alias = join(temporary, "repository alias");
  symlinkSync(root, alias, process.platform === "win32" ? "junction" : "dir");
  const expected = resolveVersion(root, { release: true });
  assert.deepEqual(resolveVersion(alias, { release: true }), expected);
  // On Windows the native form expands the runner's RUNNER~1 temp path.
  assert.deepEqual(resolveVersion(realpathSync.native(root), { release: true }), expected);
  const archive = join(alias, "source export");
  mkdirSync(archive);
  writeVersion(archive, expected);
  assert.deepEqual(resolveVersion(archive), expected);
  assert.throws(() => resolveVersion(archive, { release: true }), /Git checkout/u);
});

test("version components must fit platform resource fields", (t) => {
  const { root } = fixture(t);
  git(root, "tag", "v65536.0.0");
  assert.throws(() => resolveVersion(root), /65535/u);
});

test("direct Cargo integration emits the same version and watches Git changes", (t) => {
  const { root } = fixture(t);
  mkdirSync(join(root, "scripts"));
  cpSync(join(projectRoot, "scripts/version.mjs"), join(root, "scripts/version.mjs"));
  git(root, "add", "scripts");
  git(root, "commit", "-m", "Version resolver");
  git(root, "tag", "v2.3.4");
  const output = execFileSync(process.execPath, [join(root, "scripts/version.mjs"), "--write", "--cargo"], { encoding: "utf8" });
  assert.match(output, /cargo:rustc-env=POPPER_VERSION=2\.3\.4/u);
  assert.match(output, /cargo:rerun-if-changed=.*HEAD/u);
  assert.match(output, /cargo:rerun-if-changed=.*refs/u);
  assert.equal(JSON.parse(readFileSync(join(root, versionFile), "utf8")).version, "2.3.4");
  assert.equal(git(root, "status", "--porcelain"), "");
});

test("Windows artifact checks and Scoop packaging follow the tag and reject stale installers", (t) => {
  const { root, temporary } = fixture(t);
  for (const path of [
    "scripts/version.mjs", "package.json", "src-tauri/Cargo.toml",
    "src-tauri/tauri.conf.json", "src-tauri/tauri.windows.conf.json",
    "apps/windows/scripts/verify-artifacts.mjs", "apps/windows/scripts/prepare-release-assets.mjs",
    "THIRD_PARTY_NOTICES.md", "LICENSE.selection-hook", "LICENSE.capture", "LICENSE",
  ]) {
    mkdirSync(join(root, path, ".."), { recursive: true });
    cpSync(join(projectRoot, path), join(root, path));
  }
  git(root, "add", ".");
  git(root, "commit", "-m", "Packaging source");
  git(root, "tag", "v4.5.6");
  const target = join(temporary, "target");
  const release = join(target, "x86_64-pc-windows-msvc/release");
  mkdirSync(join(release, "bundle/nsis"), { recursive: true });
  mkdirSync(join(release, "bundle/portable"), { recursive: true });
  // Only the existing PE/packaging validation contract is exercised here;
  // these small fixtures are not executable applications or actual installers.
  const pe = Buffer.alloc(134);
  pe.write("MZ");
  pe.writeUInt32LE(128, 0x3c);
  pe.write("PE\0\0", 128);
  pe.writeUInt16LE(0x8664, 132);
  writeFileSync(join(release, "popper.exe"), pe);
  writeFileSync(join(release, "bundle/nsis/Popper_4.5.6_x64-setup.exe"), pe);
  writeFileSync(join(release, "bundle/portable/Popper_4.5.6_x64-portable.zip"), "portable fixture");
  const run = (script) => spawnSync(process.execPath, [join(root, "apps/windows/scripts", script)], {
    cwd: root, encoding: "utf8", env: { ...process.env, CARGO_TARGET_DIR: target, GITHUB_REPOSITORY: "example/popper" },
  });
  const verification = run("verify-artifacts.mjs");
  assert.equal(verification.status, 0, verification.stderr);
  assert.match(verification.stdout, /Popper_4\.5\.6_x64-setup/u);
  const packaging = run("prepare-release-assets.mjs");
  assert.equal(packaging.status, 0, packaging.stderr);
  const manifest = JSON.parse(readFileSync(join(release, "bundle/distribution/popper.json"), "utf8"));
  assert.equal(manifest.version, "4.5.6");
  assert.match(manifest.architecture["64bit"].url, /\/v4\.5\.6\/Popper_4\.5\.6_x64-portable\.zip$/u);
  git(root, "tag", "-d", "v4.5.6");
  git(root, "tag", "v4.5.7");
  const stale = run("verify-artifacts.mjs");
  assert.notEqual(stale.status, 0);
  assert.match(stale.stderr, /Popper_4\.5\.7_x64-setup/u);
});
