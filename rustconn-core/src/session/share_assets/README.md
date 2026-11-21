# share_assets: vendored browser viewer assets

Served by the session share pages (`session/share.rs`, `session/share_quic.rs`)
to a browser that opens a share link. Nothing here is built; the files are
embedded with `include_str!`.

| File | Origin | Notes |
|------|--------|-------|
| `xterm.min.js` | `lib/xterm.js` of npm package `@xterm/xterm` **5.5.0** | Upstream minified file minus its last line, `//# sourceMappingURL=xterm.js.map` (the map is not shipped). |
| `xterm.css` | `css/xterm.css` of `@xterm/xterm` **5.5.0** | Unmodified. |
| `xterm.LICENSE` | `LICENSE` of `@xterm/xterm` 5.5.0 | MIT. Keep it next to the code it covers. |

Upstream: <https://github.com/xtermjs/xterm.js> (tag `5.5.0`), package
tarball <https://registry.npmjs.org/@xterm/xterm/-/xterm-5.5.0.tgz>
(tarball sha256 `bd954fa721872170188cc5d7e83e88db3c83c9a18a4e8d24c2783d26491f59d2`).

## Checksums of the files in this directory

```
7a88e46a58847d3f17109e3d48cc8aa056a01997d00ceb76035f63370712d858  xterm.min.js
ba8e6985669488981ccf40c0cefe3aba80722cb6c92de7ad628b0bd717faf2b6  xterm.css
b569f629d00f2626a8100df2a1798210535621e42164dfd426a6fe5aac7b0ccd  xterm.LICENSE
```

## Verify or update

```bash
curl -sLO https://registry.npmjs.org/@xterm/xterm/-/xterm-5.5.0.tgz
tar xzf xterm-5.5.0.tgz
# xterm.min.js is lib/xterm.js without the sourceMappingURL line:
head -c "$(stat -c %s xterm.min.js)" package/lib/xterm.js | cmp - xterm.min.js
cmp package/css/xterm.css xterm.css
sha256sum xterm.min.js xterm.css xterm.LICENSE
```

To update: fetch the new version, strip the final `//# sourceMappingURL=` line,
replace the files, and edit the version, tarball hash and checksums above.

## Package name

The project moved from the npm name `xterm` to the scoped `@xterm/xterm`. The
old name stops at 5.3.0, so searching npm for `xterm` finds an older build than
the one here. The vendored CSS equals the `@xterm/xterm` file (the old
`xterm@5.3.0` CSS differs). Take updates from `@xterm/xterm`.

## Notes

The script is static and embedded; the server never builds it from user input.
The viewer page sets `disableStdin: true`, so a viewer cannot type into the
shared session. `cargo deny` does not see these files (they are not a crate), so
a version change is a manual review.
