//! Externalized prompts — loaded from `prompts/*.md` so they can be edited without
//! recompiling. Defaults are embedded in the binary (include_dir, so a standalone
//! binary still works); an on-disk `prompts/` dir overrides them and is re-read on
//! every call, so editing a `.md` takes effect on the NEXT request (no rebuild/restart).
//!
//! Override search order: $MORI_CANVAS_PROMPTS, ./prompts, ~/.mori/mori-canvas/prompts.
//! Compose with a `{{include:NAME}}` line (one level), which inlines `NAME.md`.
use include_dir::{include_dir, Dir};
use std::path::PathBuf;

static EMBEDDED: Dir = include_dir!("$CARGO_MANIFEST_DIR/../prompts");

fn override_dirs() -> Vec<PathBuf> {
    let mut v = vec![];
    if let Ok(d) = std::env::var("MORI_CANVAS_PROMPTS") {
        if !d.trim().is_empty() {
            v.push(PathBuf::from(d));
        }
    }
    v.push(PathBuf::from("prompts"));
    if let Ok(home) = std::env::var("HOME") {
        v.push(PathBuf::from(home).join(".mori/mori-canvas/prompts"));
    }
    v
}

fn read_raw(name: &str) -> String {
    let file = format!("{}.md", name);
    for base in override_dirs() {
        if let Ok(s) = std::fs::read_to_string(base.join(&file)) {
            return s;
        }
    }
    EMBEDDED
        .get_file(&file)
        .and_then(|f| f.contents_utf8())
        .unwrap_or("")
        .to_string()
}

fn strip_comments(s: &str) -> String {
    let mut out = s.to_string();
    while let (Some(a), Some(b)) = (out.find("<!--"), out.find("-->")) {
        if b > a {
            out.replace_range(a..b + 3, "");
        } else {
            break;
        }
    }
    out
}

/// Load a prompt by name, resolving `{{include:other}}` lines (one level) and
/// stripping editor-note `<!-- ... -->` comments.
pub fn prompt(name: &str) -> String {
    let raw = read_raw(name);
    let mut resolved = String::new();
    for line in raw.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("{{include:") {
            if let Some(inc) = rest.strip_suffix("}}") {
                resolved.push_str(&read_raw(inc.trim()));
                resolved.push('\n');
                continue;
            }
        }
        resolved.push_str(line);
        resolved.push('\n');
    }
    strip_comments(&resolved).trim().to_string()
}
