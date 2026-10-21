import { createHash } from "node:crypto";
import { copyFileSync, mkdirSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
import { dirname, isAbsolute, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../../..");

export function createScoopManifest({ version, repository, hash }) {
  if (!/^\d+\.\d+\.\d+$/u.test(version)) throw new Error("Invalid release version");
  if (!/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/u.test(repository)) throw new Error("Invalid GitHub repository");
  if (!/^[a-f0-9]{64}$/u.test(hash)) throw new Error("Invalid portable SHA-256");
  const homepage = `https://github.com/${repository}`;
  return {
    version,
    description: "Lightweight Windows selection assistant with AI and dictionary support.",
    homepage,
    license: "Unknown",
    architecture: {
      "64bit": {
        url: `${homepage}/releases/download/v${version}/Popper_${version}_x64-portable.zip`,
        hash,
      },
    },
    pre_install: [
      "if (Get-Process -Name popper -ErrorAction SilentlyContinue) { throw 'Exit Popper before installing or updating it.' }",
      "foreach ($name in @('config', 'logs')) {",
      "    $legacy = Join-Path $persist_dir \"data/$name\"",
      "    $destination = Join-Path $persist_dir $name",
      "    if ((Test-Path -LiteralPath $legacy -PathType Container) -and !(Test-Path -LiteralPath $destination)) {",
      "        $staging = Join-Path $persist_dir ('.migrate-' + [Guid]::NewGuid().ToString('N'))",
      "        try {",
      "            Copy-Item -LiteralPath $legacy -Destination $staging -Recurse -Force -ErrorAction Stop",
      "            [System.IO.Directory]::Move($staging, $destination)",
      "        } finally {",
      "            if (Test-Path -LiteralPath $staging) { Remove-Item -LiteralPath $staging -Recurse -Force -ErrorAction Stop }",
      "        }",
      "    }",
      "    if (!(Test-Path -LiteralPath (Join-Path $dir $name))) { New-Item -ItemType Directory -Path (Join-Path $dir $name) -ErrorAction Stop | Out-Null }",
      "}",
    ],
    pre_uninstall: "if (Get-Process -Name popper -ErrorAction SilentlyContinue) { throw 'Exit Popper before updating or uninstalling it.' }",
    bin: [["popper.exe", "popper"]],
    shortcuts: [["popper.exe", "Popper"]],
    persist: ["config", "logs"],
    notes: [
      "Settings and local API keys persist in persist/popper/config; logs persist in persist/popper/logs.",
      "WebView data stays beside the executable and is not persisted across upgrades.",
      "Legacy persist/popper/data/config and logs are copied on upgrade when the new directories do not exist; legacy data is kept.",
      "Microsoft Edge WebView2 Runtime is required: https://developer.microsoft.com/microsoft-edge/webview2/",
      "Exit Popper before updates. See the included README.txt for importing installed settings.",
    ],
    checkver: { github: homepage },
    autoupdate: {
      architecture: {
        "64bit": {
          url: `${homepage}/releases/download/v$version/Popper_$version_x64-portable.zip`,
          hash: { url: `${homepage}/releases/download/v$version/SHA256SUMS.txt`, regex: "$sha256\\s+Popper_$version_x64-portable\\.zip" },
        },
      },
    },
  };
}

export function prepareReleaseAssets({ releaseRoot, version, repository }) {
  // Validate path/URL components before reading any build artifacts.
  createScoopManifest({ version, repository, hash: "0".repeat(64) });
  const installerName = `Popper_${version}_x64-setup.exe`;
  const portableName = `Popper_${version}_x64-portable.zip`;
  const sourceFiles = [
    [installerName, join(releaseRoot, "bundle/nsis", installerName)],
    [portableName, join(releaseRoot, "bundle/portable", portableName)],
  ];
  const hashes = new Map(sourceFiles.map(([name, path]) => {
    if (!statSync(path).isFile() || !statSync(path).size) throw new Error(`Missing or empty artifact: ${path}`);
    return [name, createHash("sha256").update(readFileSync(path)).digest("hex")];
  }));
  const manifest = createScoopManifest({ version, repository, hash: hashes.get(portableName) });
  const output = join(releaseRoot, "bundle/distribution");
  // This generated directory may have been restored from the Rust cache.
  // Publish only this build's four assets, never stale files from older releases.
  rmSync(output, { recursive: true, force: true });
  mkdirSync(output, { recursive: true });
  for (const [name, path] of sourceFiles) copyFileSync(path, join(output, name));
  const manifestContents = `${JSON.stringify(manifest, null, 4)}\n`;
  writeFileSync(join(output, "popper.json"), manifestContents);
  hashes.set("popper.json", createHash("sha256").update(manifestContents).digest("hex"));
  writeFileSync(join(output, "SHA256SUMS.txt"), [...hashes].map(([name, hash]) => `${hash}  ${name}\n`).join(""));
  return output;
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  const { version } = JSON.parse(readFileSync(join(root, "package.json"), "utf8"));
  const target = process.env.CARGO_TARGET_DIR;
  const targetRoot = target ? (isAbsolute(target) ? target : resolve(root, target)) : join(root, "src-tauri/target");
  console.log(prepareReleaseAssets({
    releaseRoot: join(targetRoot, "x86_64-pc-windows-msvc/release"),
    version,
    repository: process.env.GITHUB_REPOSITORY || "AruoTan/popper",
  }));
}
