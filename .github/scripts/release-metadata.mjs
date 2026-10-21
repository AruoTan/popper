import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";

const read = (path) => readFileSync(path, "utf8");
const version = JSON.parse(read("package.json")).version;
if (typeof version !== "string" || !/^\d+\.\d+\.\d+$/u.test(version)) {
  throw new Error("Release requires a three-part version in package.json");
}

const cargoVersion = read("src-tauri/Cargo.toml")
  .split(/(?=^\[)/mu)
  .find((section) => section.startsWith("[package]"))
  ?.match(/^version\s*=\s*"([^"]+)"/mu)?.[1];
const lockPackage = read("src-tauri/Cargo.lock")
  .split(/\n(?=\[\[package\]\])/u)
  .find((block) => /^name\s*=\s*"popper"$/mu.test(block));
const lockVersion = lockPackage?.match(/^version\s*=\s*"([^"]+)"/mu)?.[1];
for (const [path, actual] of [
  ["src-tauri/tauri.conf.json", JSON.parse(read("src-tauri/tauri.conf.json")).version],
  ["src-tauri/tauri.windows.conf.json", JSON.parse(read("src-tauri/tauri.windows.conf.json")).version],
  ["src-tauri/Cargo.toml", cargoVersion],
  ["src-tauri/Cargo.lock", lockVersion],
]) {
  if (actual !== version) {
    throw new Error(`${path} version ${actual ?? "missing"} does not match ${version}`);
  }
}

const tag = `v${version}`;
const git = (...args) => execFileSync("git", args, { encoding: "utf8" }).trim();
const source = git("rev-parse", "HEAD");
if (process.env.GITHUB_SHA && source !== process.env.GITHUB_SHA) {
  throw new Error("Checked-out source does not match the workflow commit");
}
if (git("tag", "--list", tag)) {
  const target = git("rev-parse", `refs/tags/${tag}^{commit}`);
  if (target !== source) {
    throw new Error(`${tag} already points to a different commit; update the project version`);
  }
}

console.log(`version=${version}\ntag=${tag}`);
