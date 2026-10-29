import { resolveVersion } from "../../scripts/version.mjs";

const { version, tag, commit } = resolveVersion(undefined, { release: true });
if (process.env.GITHUB_SHA && commit !== process.env.GITHUB_SHA) {
  throw new Error("Checked-out source does not match the workflow commit");
}
for (const [name, actual] of [["RELEASE_VERSION", version], ["RELEASE_TAG", tag]]) {
  if (process.env[name] && process.env[name] !== actual) {
    throw new Error(name + " changed during the release; refusing to publish different artifacts");
  }
}
console.log("version=" + version + "\ntag=" + tag);
