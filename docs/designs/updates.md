# Updates from GitHub releases (M6)

Brief: `files_project.md` section 11. Decisions by the owner (2026-11-11):
SHA-256 plus a minisign signature; the signing key is a GitHub Actions secret.

## Release (done first)

*   **Trigger:** pushing a tag `v*` runs `.github/workflows/release.yml`.
    The tag must equal the version in `crates/files-ui-gpui/Cargo.toml`.
*   **Build:** tests, then `cargo build --release --locked` of `files.exe`
    and `rdppaste.exe`.
*   **Assets:** both `.exe` files, each with its own `.minisig` (what the
    updater checks), plus `SHA256SUMS.txt` and its `.minisig` for people
    checking by hand. The workflow verifies every signature with the public
    key in the repo, so a wrong secret fails the release instead of shipping.
*   **Secret safety:**
    *   the key lives in the `release` environment, which only `v*` tags may
        use, so pull requests and branches never see it;
    *   forks never get secrets;
    *   the key is passwordless: a password would be one more secret in the
        same place;
    *   the key file exists only for the signing step and is deleted after.
*   **Remaining risk:** a taken-over GitHub account, or a compromised action.
    Only GitHub's own actions are used, and `rsign2` is pinned
    (`--locked --version`).

## Install and update (done in v0.2.0)

*   **Install:** `files --install` copies both programs to
    `%LOCALAPPDATA%\Programs\FilesProject` and adds that folder to the
    user's PATH (HKCU only).
*   **Check:** once per start of the installed copy, in the background:
    the tag that `/releases/latest` redirects to, read with Windows'
    `curl.exe` (no API token, no JSON). Show a quiet "Update to vX" button;
    never update without a click.
*   **Apply:**
    1.  download the assets to a temporary folder;
    2.  verify each file's own `.minisig` against the public key compiled
        into the program (`minisign-verify`, which hashes the file itself);
    3.  rename the running `files.exe` to `files.exe.old` (a running `.exe`
        can be renamed, not overwritten), move the new files in, restart;
    4.  the previous version stays as `files.exe.old` for rollback.
*   **Not planned:** delta updates, auto-update without a click, code
    signing (Q7; SmartScreen warns about unsigned downloads, not about our
    own update).
