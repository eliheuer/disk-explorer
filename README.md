# disk-explorer

A small, read-only disk usage map for the terminal. Scan a folder, then browse its contents sorted by space used. Built with Rust and ratatui.

```sh
cd ~/GH/repos/disk-explorer
cargo run --release --                 # scan your home directory
cargo run --release -- ~/Downloads     # scan one folder
cargo run --release -- /               # scan the whole filesystem (may take longer)
```

| Key | Action |
| --- | --- |
| ↑ / ↓ or j / k | Select an item |
| Enter / → or l | Open a directory |
| ← / Backspace or h | Go back |
| Home / End | Jump to first / last item |
| q / Esc | Quit (including during a scan) |

Sizes are **allocated disk space**, not logical file length. Directory sizes include their contents; hard-linked files count only once within the scan. Symlinks are shown but not followed. APFS clones and snapshots may mean displayed totals differ from space you can actually reclaim. Unreadable entries are skipped and flagged; macOS privacy permissions can prevent a complete scan of some folders. A full scan finishes before browsing begins, with a live entry counter while it runs. No deletion or file modification commands are provided.
