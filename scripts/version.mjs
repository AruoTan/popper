import { execFileSync } from "node:child_process";
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

export const projectRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
export const versionFile = ".popper-version.json";
const tagPattern = /^v(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/u;

function git(root, ...args) {
  return execFileSync("git", args, { cwd: root, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] }).trim();
}

function validate(metadata) {
  if (!tagPattern.test(metadata.tag) || metadata.version !== metadata.tag.slice(1)
    || !/^(?:[a-f0-9]{40}|[a-f0-9]{64})$/u.test(metadata.commit)
    || !Number.isSafeInteger(metadata.distance) || metadata.distance < 0) {
    throw new Error("Invalid Git-derived version metadata");
  }
  // The same version must fit Windows PE resources and macOS bundle metadata.
  if (metadata.version.split(".").some((part) => Number(part) > 65535)) {
    throw new Error("Version components must be at most 65535");
  }
  return metadata;
}

export function resolveVersion(root = projectRoot, { release = false } = {}) {
  let prefix;
  try {
    // Let Git identify its root. Path strings can differ for the same Windows
    // directory (8.3 aliases, case, or junctions), even after realpath().
    prefix = git(root, "rev-parse", "--show-prefix");
  } catch (error) {
    if (existsSync(join(root, ".git"))) {
      throw new Error("Cannot read Git metadata; install Git and restore repository access", { cause: error });
    }
  }
  if (prefix !== "") {
    if (release) throw new Error("Release requires a Git checkout with an exact version tag");
    const snapshot = join(root, versionFile);
    if (!existsSync(snapshot)) throw new Error("No Git checkout or exported version snapshot; clone with tags or use an exported source package");
    return validate(JSON.parse(readFileSync(snapshot, "utf8")));
  }
  if (git(root, "rev-parse", "--is-shallow-repository") === "true") {
    throw new Error("Version requires full Git history and tags; run git fetch --unshallow --tags");
  }
  const tags = git(root, "tag", "--merged", "HEAD").split("\n").filter((tag) => tagPattern.test(tag));
  if (!tags.length) throw new Error("No reachable vX.Y.Z tag; fetch tags or create a version tag before building");
  const exact = git(root, "tag", "--points-at", "HEAD").split("\n").filter((tag) => tagPattern.test(tag));
  if (exact.length > 1) throw new Error("Multiple version tags point to HEAD; keep one release version per commit");
  if (release && exact.length !== 1) throw new Error("Release requires a vX.Y.Z tag pointing exactly to HEAD");
  const tag = exact[0] ?? git(root, "describe", "--tags", "--abbrev=0", ...tags.flatMap((entry) => ["--match", entry]), "HEAD");
  const metadata = validate({
    version: tag.slice(1), tag,
    commit: git(root, "rev-parse", "HEAD"),
    distance: Number(git(root, "rev-list", "--count", `${tag}..HEAD`)),
  });
  if (release && git(root, "status", "--porcelain", "--untracked-files=normal")) {
    throw new Error("Release requires a clean working tree");
  }
  return metadata;
}

export function writeVersion(root = projectRoot, metadata = resolveVersion(root)) {
  const path = join(root, versionFile);
  const content = `${JSON.stringify(metadata, null, 2)}\n`;
  if (!existsSync(path) || readFileSync(path, "utf8") !== content) writeFileSync(path, content);
  return metadata;
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  const flags = new Set(process.argv.slice(2));
  if ([...flags].some((flag) => !["--write", "--json", "--release", "--cargo"].includes(flag))) {
    throw new Error("Usage: node scripts/version.mjs [--write] [--json] [--release] [--cargo]");
  }
  const metadata = resolveVersion(projectRoot, { release: flags.has("--release") });
  if (flags.has("--write")) writeVersion(projectRoot, metadata);
  if (flags.has("--cargo")) {
    console.log(`cargo:rustc-env=POPPER_VERSION=${metadata.version}`);
    console.log(`cargo:rerun-if-changed=../${versionFile}`);
    if (existsSync(join(projectRoot, ".git"))) {
      for (const name of ["HEAD", "refs", "packed-refs", "shallow"]) {
        const path = git(projectRoot, "rev-parse", "--path-format=absolute", "--git-path", name);
        if (existsSync(path)) console.log(`cargo:rerun-if-changed=${path}`);
      }
      const branch = git(projectRoot, "rev-parse", "--symbolic-full-name", "HEAD");
      if (branch.startsWith("refs/")) {
        const path = git(projectRoot, "rev-parse", "--path-format=absolute", "--git-path", branch);
        if (existsSync(path)) console.log(`cargo:rerun-if-changed=${path}`);
      }
    }
  } else {
    console.log(flags.has("--json") ? JSON.stringify(metadata) : metadata.version);
  }
}
