#!/usr/bin/env python3
"""Re-record the engine asset digests in engines/lock.toml from GitHub's release API.

Run this after changing `pinned_tag`. It is deliberately not part of the verify gates: a gate that
reaches the network is a gate that fails for reasons unrelated to the repo. It refuses to write
unless every locked asset exists in the release with the size the lock already claims, so the
digests it records are GitHub's own `assets[].digest` values rather than anything invented here.

  python3 scripts/refresh-engine-digests.py            # fill in what is missing
  python3 scripts/refresh-engine-digests.py --check    # exit non-zero if any digest is stale
"""

import json
import re
import subprocess
import sys

LOCK = "engines/lock.toml"
API = "https://api.github.com/repos/{repo}/releases/tags/{tag}"


def main():
    check = "--check" in sys.argv
    raw = open(LOCK).read()
    tag = re.search(r'^pinned_tag = "([^"]+)"', raw, re.M).group(1)
    repo = re.search(r'^repo = "([^"]+)"', raw, re.M).group(1)
    url = API.format(repo=repo, tag=tag)

    # curl rather than urllib: the system Python on macOS has no trust store, so urllib fails the
    # handshake for reasons that have nothing to do with the lock file.
    fetched = subprocess.run(
        ["curl", "-sSf", "--max-time", "60", "-H", "Accept: application/vnd.github+json", url],
        capture_output=True,
        text=True,
    )
    if fetched.returncode != 0:
        sys.exit(f"could not read {url}: {fetched.stderr.strip()}")
    release = json.loads(fetched.stdout)
    if release["tag_name"] != tag:
        sys.exit(f"API returned {release['tag_name']!r}, lock pins {tag!r}")

    assets = {a["name"]: a for a in release["assets"]}
    lines = raw.split("\n")
    out = []
    stale = []
    checked = 0
    i = 0
    while i < len(lines):
        line = lines[i]
        m = re.match(r'^asset = "([^"]+)"$', line)
        if not m:
            out.append(line)
            i += 1
            continue

        name = m.group(1)
        # The rest of this [[target]] block: the contiguous `key = value` lines after `asset`.
        j = i + 1
        while j < len(lines) and re.match(r"^\w[\w.]* = ", lines[j]):
            j += 1
        block = lines[i + 1 : j]
        size_line = next((l for l in block if l.startswith("size_bytes = ")), None)
        digest_line = next((l for l in block if l.startswith("sha256 = ")), None)

        asset = assets.get(name)
        if asset is None:
            sys.exit(f"{name}: absent from release {tag}")
        checked += 1
        if size_line is None:
            sys.exit(f"{name}: locked without a size_bytes line")
        size = int(re.match(r"^size_bytes = ([\d_]+)$", size_line).group(1).replace("_", ""))
        if size != asset["size"]:
            stale.append(f"{name}: lock says {size} bytes, release says {asset['size']}")
        if not str(asset.get("digest", "")).startswith("sha256:"):
            sys.exit(f"{name}: release publishes no sha256 digest")
        want = f'sha256 = "{asset["digest"][len("sha256:"):].lower()}"'
        if digest_line is not None and digest_line != want:
            stale.append(f"{name}: lock digest != release digest")

        out.append(line)
        for l in block:
            out.append(l)
            if l == size_line and digest_line is None:
                out.append(want)
        i = j

    print(f"release {tag}: {len(assets)} assets, {checked} locked")
    if stale:
        for s in stale:
            print(f"STALE  {s}")
        sys.exit("re-record the digests by re-running without --check")
    if check:
        print("OK — every locked asset matches the release")
        return
    open(LOCK, "w").write("\n".join(out))
    print("wrote", LOCK)


main()
