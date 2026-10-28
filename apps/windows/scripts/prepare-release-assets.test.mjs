import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { mkdtempSync, mkdirSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { createScoopManifest, prepareReleaseAssets } from "./prepare-release-assets.mjs";

test("Scoop manifest pins the portable version and hash and persists only config and logs", () => {
  const manifest = createScoopManifest({ version: "1.2.3", repository: "example/popper", hash: "a".repeat(64) });
  assert.equal(manifest.architecture["64bit"].url, "https://github.com/example/popper/releases/download/v1.2.3/Popper_1.2.3_x64-portable.zip");
  assert.equal(manifest.architecture["64bit"].hash, "a".repeat(64));
  assert.equal(manifest.license, "GPL-3.0-only");
  assert.deepEqual(manifest.persist, ["config", "logs"]);
  assert.deepEqual(manifest.shortcuts, [["popper.exe", "Popper"]]);
  assert.deepEqual(manifest.bin, [["popper.exe", "popper"]]);
  assert.equal(manifest.env_set, undefined);
  assert.match(manifest.autoupdate.architecture["64bit"].hash.regex, /\$sha256/u);
});

test("all release assets have matching SHA-256 entries and the manifest uses the actual ZIP digest", () => {
  const releaseRoot = mkdtempSync(join(tmpdir(), "popper-assets-"));
  try {
    for (const directory of ["nsis", "portable"]) mkdirSync(join(releaseRoot, "bundle", directory), { recursive: true });
    writeFileSync(join(releaseRoot, "bundle/nsis/Popper_1.2.3_x64-setup.exe"), "test installer");
    const zip = Buffer.from("test portable archive");
    writeFileSync(join(releaseRoot, "bundle/portable/Popper_1.2.3_x64-portable.zip"), zip);
    mkdirSync(join(releaseRoot, "bundle/distribution"));
    writeFileSync(join(releaseRoot, "bundle/distribution/Popper_0.0.1_x64-portable.zip"), "stale cached archive");
    const output = prepareReleaseAssets({ releaseRoot, version: "1.2.3", repository: "example/popper" });
    const sums = readFileSync(join(output, "SHA256SUMS.txt"), "utf8").trim().split("\n");
    assert.equal(sums.length, 3);
    assert.deepEqual(readdirSync(output).sort(), ["Popper_1.2.3_x64-portable.zip", "Popper_1.2.3_x64-setup.exe", "SHA256SUMS.txt", "popper.json"]);
    for (const line of sums) {
      const [hash, name] = line.split("  ");
      assert.equal(hash, createHash("sha256").update(readFileSync(join(output, name))).digest("hex"));
    }
    const manifest = JSON.parse(readFileSync(join(output, "popper.json"), "utf8"));
    assert.equal(manifest.architecture["64bit"].hash, createHash("sha256").update(zip).digest("hex"));
    assert.deepEqual(readFileSync(join(output, "Popper_1.2.3_x64-portable.zip")), zip);
  } finally { rmSync(releaseRoot, { recursive: true, force: true }); }
});

test("missing artifacts fail before a Scoop manifest is published", () => {
  const releaseRoot = mkdtempSync(join(tmpdir(), "popper-missing-assets-"));
  try {
    assert.throws(() => prepareReleaseAssets({ releaseRoot, version: "1.2.3", repository: "example/popper" }), /ENOENT/u);
  } finally { rmSync(releaseRoot, { recursive: true, force: true }); }
});

test("invalid release metadata cannot generate download URLs", () => {
  const valid = { version: "1.2.3", repository: "example/popper", hash: "a".repeat(64) };
  for (const change of [{ version: "../1.2.3" }, { repository: "example/popper/other" }, { hash: "skip" }]) {
    assert.throws(() => createScoopManifest({ ...valid, ...change }), /Invalid/u);
  }
});
