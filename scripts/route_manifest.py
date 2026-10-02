#!/usr/bin/env python3
"""Dump the axum route table of the server as a stable, diffable manifest.

Why this exists
---------------
`cargo test` proves the handlers still pass; it does not prove *no route moved*.
During the `server/` -> split-crate refactor the acceptance bar is "the (method, path)
set is identical before and after", and the cheapest honest way to check that is to
extract the table from the source and diff two runs.

It is a source-level extractor on purpose: `axum::Router` does not expose its route
table at runtime, and the alternative (an extra test that rebuilds the router) would
have to live inside the crate being moved.

Usage
-----
    python scripts/route_manifest.py [--root .] [--out manifest.txt]

Exit code is always 0 -- comparing is the caller's job (`diff` the two dumps).

What it does *not* catch
------------------------
Handler bodies, middleware ordering, or `.route_layer(...)` attachment semantics.
Those are covered by the router unit tests; this tool is only a structural guard.
"""

from __future__ import annotations

import argparse
import os
import re
import sys
from collections import Counter

# Method helpers axum exposes at the call site, in the spellings this repo uses:
#   get(h) / post(h) / put(h) / delete(h) / patch(h) / axum::routing::delete(h)
METHOD_CALL = re.compile(
    r"\b(get|post|put|delete|patch|head|options|trace)\s*\(\s*([A-Za-z_][\w:]*)\s*\)"
)

# A bare handler (no method helper) means "any method" in axum's `route`.
BARE_HANDLER = re.compile(r"^[A-Za-z_][\w:]*$")

# Router-level directives worth diffing even though they carry no (method, path).
DIRECTIVE = re.compile(r"\.(nest|merge|fallback|route_layer|with_state|layer)\s*\(")


def balanced_call(text: str, open_paren: int) -> tuple[str, int]:
    """Return (contents, index_after_close) for the `(` at `open_paren`.

    Skips over string literals and char literals so a `)` inside a `"..."` does not
    terminate the call early -- route paths and regexes both contain them.
    """
    depth = 0
    i = open_paren
    n = len(text)
    while i < n:
        ch = text[i]
        if ch == '"':
            i += 1
            while i < n and text[i] != '"':
                i += 2 if text[i] == "\\" else 1
        elif ch == "'":
            # char literal or lifetime; both are safe to skip naively
            i += 1
            while i < n and text[i] != "'":
                i += 2 if text[i] == "\\" else 1
        elif ch == "(":
            depth += 1
        elif ch == ")":
            depth -= 1
            if depth == 0:
                return text[open_paren + 1 : i], i + 1
        i += 1
    raise ValueError("unbalanced parentheses")


def split_top_level(args: str) -> list[str]:
    """Split on commas that are not nested inside brackets or string literals."""
    parts: list[str] = []
    depth = 0
    current: list[str] = []
    i = 0
    n = len(args)
    while i < n:
        ch = args[i]
        if ch == '"':
            current.append(ch)
            i += 1
            while i < n and args[i] != '"':
                if args[i] == "\\" and i + 1 < n:
                    current.append(args[i])
                    current.append(args[i + 1])
                    i += 2
                    continue
                current.append(args[i])
                i += 1
            if i < n:
                current.append(args[i])
                i += 1
            continue
        if ch in "([{":
            depth += 1
        elif ch in ")]}":
            depth -= 1
        elif ch == "," and depth == 0:
            parts.append("".join(current))
            current = []
            i += 1
            continue
        current.append(ch)
        i += 1
    parts.append("".join(current))
    return [p.strip() for p in parts if p.strip()]


def normalise(expr: str) -> str:
    return re.sub(r"\s+", " ", expr).strip()


def normalise_handler(name: str) -> str:
    """Drop crate-rooting prefixes so the manifest survives module moves.

    `super::stickers::pack_delete`, `crate::stickers::pack_delete` and
    `super::super::stickers::pack_delete` all describe the same handler, and which one
    is written depends only on where the `router()` fn happens to live. Without this the
    route diff would show dozens of false changes during the module split.
    """
    for prefix in ("super::", "crate::", "yekitrim_server::", "yekitrim_cli::", "yekitrim_service::"):
        while name.startswith(prefix):
            name = name[len(prefix) :]
    return name


def routes_from(text: str, source: str) -> tuple[list[str], list[str]]:
    """Extract `(method, path) -> handler` lines plus router directives."""
    routes: list[tuple[str, str, str]] = []
    directives: list[str] = []

    for match in re.finditer(r"\.route\s*\(", text):
        open_paren = text.index("(", match.start())
        try:
            contents, _ = balanced_call(text, open_paren)
        except ValueError as exc:  # pragma: no cover - malformed source
            raise SystemExit(f"{source}: {exc} near offset {match.start()}") from exc

        args = split_top_level(contents)
        if len(args) != 2:
            raise SystemExit(
                f"{source}: .route() with {len(args)} args at offset {match.start()}"
            )
        path = normalise(args[0])
        if path.startswith('"') and path.endswith('"'):
            path = path[1:-1]
        handler_expr = normalise(args[1])

        found = METHOD_CALL.findall(handler_expr)
        if found:
            for method, name in found:
                routes.append((method.upper(), path, normalise_handler(name)))
        elif BARE_HANDLER.match(handler_expr):
            routes.append(("ANY", path, normalise_handler(handler_expr)))
        else:
            routes.append(("?", path, normalise_handler(handler_expr)))

    for match in DIRECTIVE.finditer(text):
        # Only the directive name matters; its arguments are checked by the unit tests.
        directives.append(match.group(1))

    return (
        [f"{m:<7} {p:<46} -> {h}" for m, p, h in routes],
        directives,
    )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", default=".", help="repository root")
    parser.add_argument(
        "--out", default=None, help="write the manifest here instead of stdout"
    )
    options = parser.parse_args()

    # Three layouts, newest first: the split module tree (`server/api/mod.rs` +
    # `server/api/*.rs`) that S4 introduces, and the two monolithic forms that
    # came before it (`server/api.rs`, `cli/server/api.rs`). The first entry of
    # `candidates` that exists wins; when it is a directory module, every
    # sibling `.rs` is folded in below.
    candidates = [
        "crates/yekitrim-server/src/api/mod.rs",
        "crates/yekitrim-server/src/api.rs",
        "crates/yekitrim-cli/src/server/api.rs",
    ]
    sources: list[str] = []
    for candidate in candidates:
        full = os.path.join(options.root, candidate.replace("/", os.sep))
        if os.path.isfile(full):
            sources.append(candidate)
            break
    else:
        raise SystemExit("找不到 api.rs（试过: " + ", ".join(candidates) + ")")

    first_dir = os.path.dirname(sources[0].replace("/", os.sep))
    # `.../api/mod.rs` names the directory module itself -- its siblings live next
    # to it, not in an `api/` subdirectory. The monolithic forms sit one level up.
    if os.path.basename(sources[0]) == "mod.rs":
        api_dir = os.path.join(options.root, first_dir)
        sibling_dir = first_dir
    else:
        api_dir = os.path.join(options.root, first_dir, "api")
        sibling_dir = os.path.join(first_dir, "api")
    if os.path.isdir(api_dir):
        for name in sorted(os.listdir(api_dir)):
            if name.endswith(".rs") and name != "mod.rs":
                sources.append(
                    os.path.join(sibling_dir, name).replace(os.sep, "/")
                )

    all_routes: list[str] = []
    all_directives: list[str] = []
    for source in sources:
        full = os.path.join(options.root, source.replace("/", os.sep))
        with open(full, encoding="utf-8") as handle:
            text = handle.read()
        routes, directives = routes_from(text, source)
        all_routes.extend(routes)
        all_directives.extend(directives)

    lines = [f"# sources: {', '.join(sources)}", "", "## routes (sorted)"]
    lines.extend(sorted(set(all_routes)))
    lines.append("")
    lines.append(f"## route count: {len(set(all_routes))}")
    lines.append("")
    lines.append("## router directives (order preserved)")
    lines.extend(f"{i:>3} {d}" for i, d in enumerate(all_directives))
    lines.append("")
    lines.append(f"## directive counts: {sorted({d: all_directives.count(d) for d in all_directives}.items())}")

    output = "\n".join(lines) + "\n"
    if options.out:
        parent = os.path.dirname(os.path.abspath(options.out))
        os.makedirs(parent, exist_ok=True)
        with open(options.out, "w", encoding="utf-8") as handle:
            handle.write(output)
        print(f"wrote {options.out} ({len(set(all_routes))} routes)", file=sys.stderr)
    else:
        sys.stdout.write(output)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
