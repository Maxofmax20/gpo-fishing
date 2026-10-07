/**
 * START THE ONTOLOGY REVIEW WORKSTATION
 *
 * `npm run review`
 *
 * Why this exists: the review page is served by a local HTTP server, and until now
 * the only way to start it was a hand-typed `cargo run --example` with three
 * environment variables set. That is not a reasonable thing to ask of someone who
 * just wants to open a page.
 *
 * This is a launcher, not a second server. It builds and runs the same
 * `ontology_review_server` example, which calls the same `ontology_web::spawn()`
 * the desktop application calls at launch.
 *
 * Two build settings are applied here because this repository trips over both on a
 * modest Windows machine, and both produce errors that look like code defects:
 *
 *   CARGO_BUILD_JOBS=1
 *     Linking pulls in ~150 crates. With parallel link jobs on a low-memory machine
 *     the linker dies with `LNK1102: out of memory`, after which cargo reports
 *     nonsense such as "found staticlib instead of rlib" for unrelated crates.
 *
 *   CARGO_TARGET_DIR (only if you set it)
 *     A very long workspace path overflows the Windows linker command line for the
 *     binaries that link the whole dependency graph. An explicit short target
 *     directory avoids it. Left alone when unset, so the default layout still
 *     applies.
 *
 * Ctrl+C stops the server. It writes nothing: the review artefacts are already on
 * disk, and the only write this tool can perform is a human decision appended to
 * `ontology_decisions.jsonl`.
 */

import { spawn } from "node:child_process";
import { existsSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const repo = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const crateDir = join(repo, "src-tauri");

if (!existsSync(join(crateDir, "Cargo.toml"))) {
  console.error("  src-tauri/Cargo.toml not found - run this from the repository root.");
  process.exit(1);
}

const env = {
  ...process.env,
  // The server resolves the report artefacts and the Python module from here.
  GPO_REPO_ROOT: repo,
  // See the note above: one link job at a time.
  CARGO_BUILD_JOBS: "1",
  PYTHONIOENCODING: "utf-8",
};

function run(args, label) {
  return new Promise((resolveRun, rejectRun) => {
    const p = spawn("cargo", args, {
      cwd: crateDir,
      env,
      stdio: "inherit",
      shell: process.platform === "win32",
    });
    p.on("error", rejectRun);
    p.on("exit", (code) =>
      code === 0 ? resolveRun() : rejectRun(new Error(`${label} exited with ${code}`)),
    );
  });
}

try {
  console.log("  Building the review server (first run takes a few minutes)...");
  await run(["build", "-j", "1", "--example", "ontology_review_server"], "build");
} catch (e) {
  console.error(`\n  Build failed: ${e.message}`);
  process.exit(1);
}

// The example sits in the target directory cargo chose, which this script does not
// control - so read it back rather than assuming.
const targetDir = process.env.CARGO_TARGET_DIR || join(crateDir, "target");
const exe = join(targetDir, "debug", "examples", "ontology_review_server.exe");

console.log("");
console.log("  Ontology Review Workstation");
console.log("  ----------------------------");
console.log("  http://127.0.0.1:17890/ontology-review");
console.log("  loopback only; no game, macro, or updater involved");
console.log("");
console.log("  Ctrl+C to stop.");
console.log("");

const server = spawn(exe, [], { cwd: repo, env, stdio: "inherit" });
server.on("exit", (code) => process.exit(code ?? 0));