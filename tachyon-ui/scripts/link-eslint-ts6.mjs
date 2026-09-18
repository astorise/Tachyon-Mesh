#!/usr/bin/env node
// @typescript-eslint/parser and several packages it depends on
// (typescript-estree, project-service, tsconfig-utils, ts-api-utils, ...)
// each `require("typescript")` themselves, and hard-refuse or crash against
// TypeScript >=7: some have an explicit `if (versionMajor >= 7) throw` with
// no config escape hatch (tracked at
// https://github.com/typescript-eslint/typescript-eslint/issues/10940),
// others (ts-api-utils, as of the version pulled in here) just read an enum
// member TS7 renamed/removed and crash with a bare TypeError. This project
// pins `typescript@^7`, so the whole affected subtree needs its own
// TypeScript 6.x to run against, side by side with the real TS7 the rest of
// the project (tsc, vite) uses. Microsoft publishes exactly this for the
// transition
// (https://devblogs.microsoft.com/typescript/announcing-typescript-7-0/#running-side-by-side-with-typescript-6.0):
// `@typescript/typescript6` is TS6 packaged so `require("typescript")`
// resolves to it, and it in turn requires `@typescript/old` (itself an
// alias for real `typescript@^6`).
//
// The documented way to wire this up is an npm `overrides` entry aliasing
// `typescript` to `@typescript/typescript6` for the affected packages. That
// does not work here: npm's peer-dependency override resolution collapses
// the aliased target back to the hoisted root `typescript@7` and marks the
// whole tree "invalid" (reproduced against npm 10, both with and without
// `--install-strategy=nested`) — the alias is recorded but never actually
// placed on disk, so the affected packages still resolve TS7 and still
// break. This script is the workaround: copy the two packages npm
// downloads correctly as ordinary top-level devDependencies into every
// package Node's `require("typescript")` would otherwise resolve past to
// the real root copy.
//
// Rather than hardcode which packages need this (fragile — it broke once
// already going from a two-package reproduction to this project's full
// tree, and ts-api-utils needing it too was found by testing, not by
// reading typescript-eslint's docs), this walks @typescript-eslint/parser's
// own declared dependency graph and patches every package in it that
// declares a `typescript` dependency or peerDependency, wherever npm
// actually put it on disk (hoisted to the root or nested under another
// package — not guaranteed, and observed to differ between a minimal
// reproduction and this project).
//
// Idempotent and safe to run on every `npm install`/`npm ci` (wired as
// "postinstall"): it only copies files already present in the top-level
// node_modules, does no network access itself, and no-ops quietly if
// @typescript-eslint/parser isn't installed (e.g. a production-only
// install that skips devDependencies).
import { existsSync, cpSync, rmSync, symlinkSync, readlinkSync, readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { createRequire } from "node:module";

const here = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(here, "..");
const nodeModules = path.join(root, "node_modules");
const require = createRequire(import.meta.url);

const ts6Src = path.join(nodeModules, "@typescript", "typescript6");
const tsOldSrc = path.join(nodeModules, "@typescript", "old");
if (!existsSync(ts6Src) || !existsSync(tsOldSrc)) {
  // devDependencies weren't installed (e.g. a production-only install), or
  // package.json changed without this script being updated to match.
  process.exit(0);
}

const parserPkgJson = path.join(nodeModules, "@typescript-eslint", "parser", "package.json");
if (!existsSync(parserPkgJson)) {
  process.exit(0);
}

// Walk the real, installed dependency graph (not just declared ranges) so
// this tracks whatever npm actually resolved, including nested copies.
// Resolves the dependency's main entry point rather than its package.json
// path directly: several packages here (ts-api-utils included) declare an
// `exports` map that doesn't expose `./package.json`, so
// `require.resolve(`${name}/package.json`)` throws ERR_PACKAGE_PATH_NOT_EXPORTED
// even though the package itself resolves fine.
function resolveDep(name, fromDir) {
  let entry;
  try {
    entry = require.resolve(name, { paths: [fromDir] });
  } catch {
    return undefined;
  }
  let dir = path.dirname(entry);
  while (true) {
    if (existsSync(path.join(dir, "package.json"))) return dir;
    const parent = path.dirname(dir);
    if (parent === dir) return undefined;
    dir = parent;
  }
}

const needsTypescriptFix = new Set(); // installed package dirs to patch
const visited = new Set();

function walk(pkgDir) {
  if (visited.has(pkgDir)) return;
  visited.add(pkgDir);

  const pkgJsonPath = path.join(pkgDir, "package.json");
  if (!existsSync(pkgJsonPath)) return;
  const pkg = JSON.parse(readFileSync(pkgJsonPath, "utf8"));

  const declaresTypescript =
    (pkg.dependencies && "typescript" in pkg.dependencies) ||
    (pkg.peerDependencies && "typescript" in pkg.peerDependencies);
  if (declaresTypescript) {
    needsTypescriptFix.add(pkgDir);
  }

  const deps = { ...pkg.dependencies };
  for (const depName of Object.keys(deps)) {
    if (depName === "typescript") continue;
    const depDir = resolveDep(depName, pkgDir);
    if (depDir) walk(depDir);
  }
}

const parserDir = path.dirname(parserPkgJson);
walk(parserDir);

let fixedAny = false;
for (const pkgDir of needsTypescriptFix) {
  const pkgNodeModules = path.join(pkgDir, "node_modules");
  const ts6Dest = path.join(pkgNodeModules, "typescript");
  const tsOldDest = path.join(pkgNodeModules, "@typescript", "old");

  rmSync(ts6Dest, { recursive: true, force: true });
  cpSync(ts6Src, ts6Dest, { recursive: true });

  rmSync(tsOldDest, { recursive: true, force: true });
  cpSync(tsOldSrc, tsOldDest, { recursive: true });
  fixedAny = true;
}

// @typescript/old is real typescript@6 installed under an alias so it can
// sit next to the real typescript@7 without npm treating them as the same
// package. It still declares the ordinary `tsc`/`tsserver` bin names
// though, so npm's own bin-linking (which runs before this script, as part
// of the same `npm install`) nondeterministically points
// node_modules/.bin/tsc at whichever of the two same-named bins it links
// last — verified this lands on @typescript/old's TS6 binary, silently
// swapping the `tsc --noEmit` the build script runs for the wrong major
// version. Force it back to the real, root typescript package's tsc here,
// since this script always runs last.
const binDir = path.join(nodeModules, ".bin");
const realTsc = path.join(nodeModules, "typescript", "bin", "tsc");
if (existsSync(realTsc)) {
  const tscLink = path.join(binDir, "tsc");
  const desired = path.relative(binDir, realTsc);
  let current;
  try {
    current = readlinkSync(tscLink);
  } catch {
    current = undefined;
  }
  if (current !== desired) {
    rmSync(tscLink, { force: true });
    symlinkSync(desired, tscLink);
  }
}
// Real TS7 has no tsserver bin at all; a tsserver symlink here only exists
// because @typescript/old (TS6) declares one, and it's never the right
// binary for this project's actual TypeScript version.
rmSync(path.join(binDir, "tsserver"), { force: true });

if (fixedAny) {
  console.log(
    `link-eslint-ts6: linked TypeScript 6 side-by-side for ${needsTypescriptFix.size} package(s) ` +
      "in @typescript-eslint/parser's dependency graph, and restored " +
      "node_modules/.bin/tsc to the real typescript@7 package",
  );
}
