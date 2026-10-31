//! `cargo xtask reagent-parity [--update] [PATH]`: checks the feature
//! inventory kept with Reagent's plan, which this repository doesn't hold.
//!
//! It fails when a row was last checked against a version older than the
//! inventory's reference version (so raising the reference forces a
//! re-check), and when the inventory's summary table isn't the one its rows
//! produce. `--update` rewrites the summary table instead of failing on it.
//!
//! The inventory is `PATH`, else `$REAGENT_INVENTORY`, else the file ending
//! in `inventory.md` in the maintainers' folder `~/code/beatlayer-internal/docs/reagent`.
//! Without one (an outside contributor's checkout, or CI) the check says it
//! was skipped and passes.

use std::path::{Path, PathBuf};

/// The columns a summary row counts, in order: the first word of a row's
/// Investigate cell.
const KINDS: [&str; 4] = ["ready", "observe", "measure", "decide"];

pub fn run(args: &[String]) -> Result<(), String> {
    let update = args.iter().any(|a| a == "--update");
    let path = args.iter().find(|a| *a != "--update").map(PathBuf::from);
    let Some(path) = path.or_else(find_inventory) else {
        eprintln!("skipped: no inventory here (only maintainers' checkouts have it)");
        return Ok(());
    };
    let text = std::fs::read_to_string(&path)
        .map_err(|e| format!("couldn't read {}: {e}", path.display()))?;
    let inventory = parse(&text)?;
    if !inventory.stale.is_empty() {
        let lines: Vec<String> = inventory.stale.iter().map(|s| format!("  {s}")).collect();
        return Err(format!(
            "{} row(s) were checked against a version older than {}:\n{}",
            lines.len(),
            inventory.reference,
            lines.join("\n")
        ));
    }
    let summary = inventory.summary();
    let lines = summary_replaced(&text, &summary)?;
    if text.lines().ne(lines.iter().map(String::as_str)) {
        if !update {
            return Err(
                "the summary table is out of date; run `cargo xtask reagent-parity --update`"
                    .into(),
            );
        }
        let mut out = lines.join("\n");
        out.push('\n');
        std::fs::write(&path, out)
            .map_err(|e| format!("couldn't write {}: {e}", path.display()))?;
        eprintln!("updated the summary table in {}", path.display());
    }
    eprintln!(
        "ok: {} rows, all checked against {}",
        inventory
            .areas
            .iter()
            .map(|a| a.1.iter().sum::<usize>())
            .sum::<usize>(),
        inventory.reference
    );
    Ok(())
}

fn find_inventory() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("REAGENT_INVENTORY") {
        return Some(path.into());
    }
    let dir = Path::new(&std::env::var_os("HOME")?).join("code/beatlayer-internal/docs/reagent");
    std::fs::read_dir(dir)
        .ok()?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .find(|p| p.to_string_lossy().ends_with("inventory.md"))
}

#[derive(Debug)]
struct Inventory {
    reference: String,
    /// Each section's heading and its rows' counts by `KINDS`.
    areas: Vec<(String, [usize; 4])>,
    /// "ID checked against version", for each row older than the reference.
    stale: Vec<String>,
}

/// Numbers of a dotted version, so 2.1.10 is after 2.1.9.
fn version(text: &str) -> Result<Vec<u32>, String> {
    text.split('.')
        .map(|n| n.parse().map_err(|_| format!("`{text}` isn't a version")))
        .collect()
}

fn parse(text: &str) -> Result<Inventory, String> {
    // The reference is the last word of the bold span on the "Reference version" line.
    let reference = text
        .lines()
        .find_map(|l| {
            l.trim_start_matches("- ")
                .strip_prefix("**Reference version:")
        })
        .and_then(|rest| rest.split("**").next())
        .and_then(|span| span.split_whitespace().last())
        .ok_or("no `**Reference version: …**` line")?
        .to_string();
    let wanted = version(&reference)?;

    let mut inventory = Inventory {
        reference,
        areas: Vec::new(),
        stale: Vec::new(),
    };
    let mut columns = None;
    for line in text.lines() {
        if let Some(heading) = line.strip_prefix("## ") {
            if heading != "Summary" {
                inventory.areas.push((heading.to_string(), [0; 4]));
            }
            continue;
        }
        let cells: Vec<&str> = line.split('|').map(str::trim).collect();
        if cells.get(1) == Some(&"ID") {
            let at = |name| cells.iter().position(|c| *c == name);
            columns = Some((
                at("Investigate").ok_or("a table has no Investigate column")?,
                at("Checked").ok_or("a table has no Checked column")?,
            ));
            continue;
        }
        let (Some((investigate, checked)), Some(id)) = (columns, cells.get(1)) else {
            continue;
        };
        let Some((letters, digits)) = id.split_once('-') else {
            continue;
        };
        if letters.is_empty()
            || !letters.bytes().all(|b| b.is_ascii_uppercase())
            || digits.is_empty()
            || !digits.bytes().all(|b| b.is_ascii_digit())
        {
            continue;
        }
        let kind = cells[investigate]
            .split(|c: char| !c.is_alphabetic())
            .next()
            .unwrap_or("");
        let kind = KINDS.iter().position(|k| *k == kind).ok_or(format!(
            "{id}: Investigate `{}` doesn't start with {KINDS:?}",
            cells[investigate]
        ))?;
        inventory
            .areas
            .last_mut()
            .ok_or(format!("{id} is before any section"))?
            .1[kind] += 1;
        if version(cells[checked]).map_err(|e| format!("{id}: {e}"))? < wanted {
            inventory
                .stale
                .push(format!("{id} checked against {}", cells[checked]));
        }
    }
    inventory.areas.retain(|a| a.1.iter().sum::<usize>() > 0);
    Ok(inventory)
}

impl Inventory {
    /// The summary table: a row per section and a total row.
    fn summary(&self) -> Vec<String> {
        let mut rows = vec![
            "| Area | Rows | ready | observe | measure | decide |".to_string(),
            "|---|---|---|---|---|---|".to_string(),
        ];
        let mut total = [0; 4];
        for (area, counts) in &self.areas {
            let cells: Vec<String> = counts.iter().map(usize::to_string).collect();
            rows.push(format!(
                "| {area} | {} | {} |",
                counts.iter().sum::<usize>(),
                cells.join(" | ")
            ));
            for (t, c) in total.iter_mut().zip(counts) {
                *t += c;
            }
        }
        let cells: Vec<String> = total.iter().map(|t| format!("**{t}**")).collect();
        rows.push(format!(
            "| **All** | **{}** | {} |",
            total.iter().sum::<usize>(),
            cells.join(" | ")
        ));
        rows
    }
}

/// The inventory's lines with the table under `## Summary` replaced.
fn summary_replaced(text: &str, summary: &[String]) -> Result<Vec<String>, String> {
    let lines: Vec<&str> = text.lines().collect();
    let heading = lines
        .iter()
        .position(|l| *l == "## Summary")
        .ok_or("no `## Summary` section")?;
    let start = heading
        + 1
        + lines[heading + 1..]
            .iter()
            .position(|l| l.starts_with('|'))
            .ok_or("no table under `## Summary`")?;
    let end = start
        + lines[start..]
            .iter()
            .take_while(|l| l.starts_with('|'))
            .count();
    let mut out: Vec<String> = lines[..start].iter().map(|l| l.to_string()).collect();
    out.extend_from_slice(summary);
    out.extend(lines[end..].iter().map(|l| l.to_string()));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const INVENTORY: &str = "\
- **Reference version: Thing 2 2.1.5**, released in July.

## Summary

| Area | Rows | ready | observe | measure | decide |
|---|---|---|---|---|---|
| old | 0 | 0 | 0 | 0 | 0 |

Counted once.

## 1. First area

| ID | Feature | Investigate | Checked | Develop |
|---|---|---|---|---|
| AAA-001 | one | ready | 2.1.5 | todo |
| AAA-002 | two | measure: curve | 2.1.10 | todo |

## 2. Second area

| ID | Feature | Investigate | Checked | Develop |
|---|---|---|---|---|
| BBB-001 | three | observe | 2.1.5 | todo |
";

    #[test]
    fn the_summary_counts_rows_by_their_first_investigate_word() {
        let inventory = parse(INVENTORY).unwrap();
        assert_eq!(inventory.reference, "2.1.5");
        assert!(inventory.stale.is_empty());
        let table = inventory.summary();
        assert_eq!(table[2], "| 1. First area | 2 | 1 | 0 | 1 | 0 |");
        assert_eq!(table[3], "| 2. Second area | 1 | 0 | 1 | 0 | 0 |");
        assert_eq!(
            table[4],
            "| **All** | **3** | **1** | **1** | **1** | **0** |"
        );
    }

    #[test]
    fn a_row_checked_against_an_older_version_is_stale() {
        let text = INVENTORY.replace("| ready | 2.1.5 |", "| ready | 2.0.18 |");
        assert_eq!(
            parse(&text).unwrap().stale,
            ["AAA-001 checked against 2.0.18"]
        );
        // 2.1.10 is after 2.1.5, not before it.
        let text = INVENTORY.replace("Thing 2 2.1.5", "Thing 2 2.1.11");
        assert_eq!(parse(&text).unwrap().stale.len(), 3);
    }

    #[test]
    fn the_summary_table_is_replaced_in_place() {
        let summary = parse(INVENTORY).unwrap().summary();
        let lines = summary_replaced(INVENTORY, &summary).unwrap();
        assert_eq!(lines.len(), INVENTORY.lines().count() + 2);
        assert!(lines.contains(&"Counted once.".to_string()));
        assert!(!lines.contains(&"| old | 0 | 0 | 0 | 0 | 0 |".to_string()));
    }

    #[test]
    fn an_unknown_investigate_word_is_an_error() {
        let text = INVENTORY.replace("| observe |", "| maybe |");
        assert!(parse(&text).unwrap_err().contains("BBB-001"));
    }
}
