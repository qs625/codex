# codex-git-utils

Helpers for applying git patches. Git repository metadata and branch-query
helpers live in `codex-git-info`; this crate re-exports them only for old-path
compatibility.

```rust,no_run
use std::path::Path;

use codex_git_utils::{apply_git_patch, ApplyGitRequest};

let repo = Path::new("/path/to/repo");

// Apply a patch (omitted here) to the repository.
let request = ApplyGitRequest {
    cwd: repo.to_path_buf(),
    diff: String::from("...diff contents..."),
    revert: false,
    preflight: false,
};
let result = apply_git_patch(&request)?;
```
