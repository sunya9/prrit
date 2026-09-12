use crate::plan::{Entry, Pr};

pub fn format_entry(entry: &Entry) -> String {
    let sha = &entry.sha[..7.min(entry.sha.len())];
    let pr_col = match &entry.pr {
        None => "(new)".to_string(),
        Some(p) => format!("#{} {}", p.number, p.state),
    };
    let pushed = if entry.pushed { "" } else { " *" };
    format!("  {sha}  {pr_col:<12} {}{pushed}", entry.title)
}

pub fn format_orphans(orphans: &[Pr]) -> Vec<String> {
    if orphans.is_empty() {
        return vec![];
    }
    let mut lines =
        vec!["Open PRs from this account that are not in the current stack:".to_string()];
    lines.extend(
        orphans
            .iter()
            .map(|p| format!("  #{} {}  {}", p.number, p.title, p.url)),
    );
    lines
}
