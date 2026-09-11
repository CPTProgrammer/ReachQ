//! One-shot port of Zed's `edit_file` line-level fuzzy matching algorithm
//! (crates/agent/src/tools/edit_session/streaming_fuzzy_matcher.rs, `finish()`
//! semantics only — no streaming) plus the indentation reindent helper and a
//! unified-diff builder used by the edit tool's approval UI.
//!
//! Pure algorithm module: depends only on `strsim` and `imara-diff` (plus
//! serde derives on the structured diff wire types).

use std::ops::Range;

use serde::{Deserialize, Serialize};

/// Cost of pairing a query line with a similar (but not equal) buffer line.
const REPLACEMENT_COST: u32 = 1;
/// Cost of skipping a buffer line (moving Left in the matrix).
const INSERTION_COST: u32 = 3;
/// Cost of skipping a query line (moving Up in the matrix).
const DELETION_COST: u32 = 10;
/// Minimum fraction of matched lines for a candidate range to be accepted.
const MATCH_RATIO_THRESHOLD: f32 = 0.8;
/// Similarity threshold for `fuzzy_eq`.
const FUZZY_EQ_THRESHOLD: f64 = 0.8;

/// Direction of the cheapest incoming edge. Declaration order is the
/// tie-break priority when costs are equal: Up < Left < Diagonal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Dir {
    Up,
    Left,
    Diagonal,
}

/// One cell of the DP matrix. Field order is the comparison order:
/// cheaper cost wins; equal costs are tie-broken by `dir`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Cell {
    cost: u32,
    dir: Dir,
}

/// Returns all best-match byte ranges of `old_text` inside `buffer`
/// (0, 1, or N ranges). The caller is responsible for normalizing `\r\n`
/// to `\n` in `buffer` first.
pub fn fuzzy_find(buffer: &str, old_text: &str) -> Vec<Range<usize>> {
    if old_text.is_empty() {
        return vec![];
    }

    // finish() semantics: split the query into complete lines plus a
    // possibly-incomplete trailing line, and match the complete lines first.
    let (complete_lines, tail): (Vec<&str>, &str) = match old_text.rfind('\n') {
        Some(ix) => (old_text[..ix].split('\n').collect(), &old_text[ix + 1..]),
        None => (Vec::new(), old_text),
    };

    let mut ranges = fuzzy_find_lines(buffer, &complete_lines);

    if !tail.is_empty() {
        if ranges.len() == 1 {
            // Fast path: a single match whose next buffer line starts with
            // the incomplete tail extends the range directly.
            let start = ranges[0].end + 1;
            if start <= buffer.len() && buffer[start..].starts_with(tail) {
                ranges[0].end += 1 + tail.len();
                return ranges;
            }
        }
        // Otherwise treat the tail as a complete final line and re-run.
        let mut all_lines = complete_lines;
        all_lines.push(tail);
        ranges = fuzzy_find_lines(buffer, &all_lines);
    }

    ranges
}

/// Runs the DP over `query_lines` (all treated as complete lines) and
/// returns every accepted best-match range.
fn fuzzy_find_lines(buffer: &str, query_lines: &[&str]) -> Vec<Range<usize>> {
    // split('\n') (not .lines()) keeps a trailing empty line.
    let buffer_lines: Vec<&str> = buffer.split('\n').collect();
    let query_count = query_lines.len();
    let buffer_count = buffer_lines.len();

    let mut line_starts = Vec::with_capacity(buffer_count);
    let mut offset = 0usize;
    for line in &buffer_lines {
        line_starts.push(offset);
        offset += line.len() + 1;
    }

    // Matrix: (query_count + 1) rows x (buffer_count + 1) columns.
    let mut matrix = vec![
        vec![
            Cell {
                cost: 0,
                dir: Dir::Diagonal
            };
            buffer_count + 1
        ];
        query_count + 1
    ];
    // Row 0 is all (0, Diagonal): skipping any buffer prefix is free.
    // Column 0: each skipped query line costs DELETION_COST.
    for r in 0..query_count {
        matrix[r + 1][0] = Cell {
            cost: (r as u32 + 1) * DELETION_COST,
            dir: Dir::Up,
        };
    }

    for r in 0..query_count {
        let ql = query_lines[r].trim();
        for c in 0..buffer_count {
            let bl = buffer_lines[c].trim();
            let up = Cell {
                cost: matrix[r][c + 1].cost + DELETION_COST,
                dir: Dir::Up,
            };
            let left = Cell {
                cost: matrix[r + 1][c].cost + INSERTION_COST,
                dir: Dir::Left,
            };
            let diag = if ql == bl {
                Cell {
                    cost: matrix[r][c].cost,
                    dir: Dir::Diagonal,
                }
            } else if fuzzy_eq(ql, bl) {
                Cell {
                    cost: matrix[r][c].cost + REPLACEMENT_COST,
                    dir: Dir::Diagonal,
                }
            } else {
                Cell {
                    cost: matrix[r][c].cost + DELETION_COST + INSERTION_COST,
                    dir: Dir::Diagonal,
                }
            };
            matrix[r + 1][c + 1] = up.min(left).min(diag);
        }
    }

    // Scan the last row for all columns sharing the minimum cost.
    let last_row = &matrix[query_count];
    let Some(min_cost) = (1..=buffer_count).map(|c| last_row[c].cost).min() else {
        return vec![];
    };

    let mut ranges = Vec::new();
    for col in 1..=buffer_count {
        if last_row[col].cost != min_cost {
            continue;
        }
        // Backtrack from (query_count, col) to row 0.
        let mut qr = query_count;
        let mut br = col;
        let mut matched = 0usize;
        while qr > 0 {
            match matrix[qr][br].dir {
                Dir::Diagonal => {
                    qr -= 1;
                    br -= 1;
                    matched += 1;
                }
                Dir::Up => qr -= 1,
                Dir::Left => br -= 1,
            }
        }
        let end_row = col;
        let ratio = matched as f32 / (end_row - br).max(query_count) as f32;
        if ratio >= MATCH_RATIO_THRESHOLD {
            ranges.push(line_starts[br]..line_starts[end_row - 1] + buffer_lines[end_row - 1].len());
        }
    }
    ranges
}

/// Near-equality for trimmed lines: byte-length early exit, then
/// normalized Levenshtein >= 0.8.
fn fuzzy_eq(a: &str, b: &str) -> bool {
    let min_len = a.len().min(b.len()) as f64;
    let max_len = a.len().max(b.len()) as f64;
    // Similarity can never exceed min/max, so bail out early.
    if max_len > 0.0 && min_len / max_len < FUZZY_EQ_THRESHOLD {
        return false;
    }
    strsim::normalized_levenshtein(a, b) >= FUZZY_EQ_THRESHOLD
}

/// (tabs, spaces): leading-indent counts (`'\t'` -> tabs+1, `' '` -> spaces+1,
/// anything else stops the scan).
pub fn line_indent(line: &str) -> (u32, u32) {
    let mut tabs = 0u32;
    let mut spaces = 0u32;
    for ch in line.chars() {
        match ch {
            '\t' => tabs += 1,
            ' ' => spaces += 1,
            _ => break,
        }
    }
    (tabs, spaces)
}

/// Human-readable description of a leading-whitespace run, for the
/// indent-mismatch error message.
pub fn describe_indent(indent: &str) -> String {
    let (tabs, spaces) = line_indent(indent);
    if tabs == 0 && spaces == 0 {
        return "no indentation".to_string();
    }
    let mut parts = Vec::new();
    if tabs > 0 {
        parts.push(format!("{} tab{}", tabs, if tabs > 1 { "s" } else { "" }));
    }
    if spaces > 0 {
        parts.push(format!("{} space{}", spaces, if spaces > 1 { "s" } else { "" }));
    }
    parts.join(" + ")
}

/// Details when the model's old_text first-line indentation differs from
/// the file's actual indentation at the match site.
pub struct IndentMismatch {
    /// 1-based line number where the match starts.
    pub line: usize,
    pub file_indent: String,
    pub old_text_indent: String,
}

/// Compare the leading whitespace of the matched range's first line with
/// the first line of `old_text`. A difference means the model retyped the
/// indentation rather than copying it exactly; the edit must be rejected
/// loudly instead of silently re-indenting (project decision: no
/// reindent-on-write, unlike Zed).
pub fn first_line_indent_mismatch(
    buffer: &str,
    range: &Range<usize>,
    old_text: &str,
) -> Option<IndentMismatch> {
    let range_text = &buffer[range.start..range.end];
    let file_first_line = range_text.split('\n').next().unwrap_or("");
    let old_first_line = old_text.split('\n').next().unwrap_or("");
    let file_indent: String = file_first_line
        .chars()
        .take_while(|c| *c == '\t' || *c == ' ')
        .collect();
    let old_indent: String = old_first_line
        .chars()
        .take_while(|c| *c == '\t' || *c == ' ')
        .collect();
    if file_indent == old_indent {
        return None;
    }
    // 1-based line number of the match start.
    let line = buffer[..range.start].bytes().filter(|b| *b == b'\n').count() + 1;
    Some(IndentMismatch {
        line,
        file_indent,
        old_text_indent: old_indent,
    })
}

// ---------------------------------------------------------------------------
// Structured diff (wire format shared with the frontend)
// ---------------------------------------------------------------------------

/// Structured diff preview. `path`/`is_new_file` are envelope context, not
/// part of the diff computation; the Sink below only produces `hunks`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffPreview {
    pub path: String,
    pub is_new_file: bool,
    pub hunks: Vec<DiffHunk>,
}

/// One hunk: a run of changes plus DIFF_CONTEXT lines around them.
/// `old_start`/`new_start` are 1-based line numbers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffHunk {
    pub old_start: u32,
    pub new_start: u32,
    pub lines: Vec<DiffLine>,
}

/// One diff line (1-based numbers): del/ctx carry `old_no`, add/ctx carry
/// `new_no`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffLine {
    pub kind: DiffLineKind,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub old_no: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub new_no: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiffLineKind {
    Add,
    Del,
    Ctx,
}

impl DiffLine {
    fn add(text: &str, new_no: u32) -> Self {
        Self {
            kind: DiffLineKind::Add,
            text: text.to_string(),
            old_no: None,
            new_no: Some(new_no),
        }
    }

    fn del(text: &str, old_no: u32) -> Self {
        Self {
            kind: DiffLineKind::Del,
            text: text.to_string(),
            old_no: Some(old_no),
            new_no: None,
        }
    }

    fn ctx(text: &str, old_no: u32, new_no: u32) -> Self {
        Self {
            kind: DiffLineKind::Ctx,
            text: text.to_string(),
            old_no: Some(old_no),
            new_no: Some(new_no),
        }
    }
}

/// Structured diff of `old_text` vs `new_text` using imara-diff's Histogram
/// algorithm with 3 lines of context per hunk. This is the single source of
/// truth: `unified_diff` is these hunks plus text formatting.
pub fn structured_diff(old_text: &str, new_text: &str) -> Vec<DiffHunk> {
    let input = imara_diff::intern::InternedInput::new(old_text, new_text);
    imara_diff::diff(
        imara_diff::Algorithm::Histogram,
        &input,
        UnifiedDiffBuilder {
            input: &input,
            changes: Vec::new(),
        },
    )
}

/// Unified diff of `old_text` vs `new_text` (text form of `structured_diff`).
pub fn unified_diff(old_text: &str, new_text: &str) -> String {
    format_hunks(&structured_diff(old_text, new_text))
}

/// Format structured hunks as unified-diff text. Hunk line counts are derived
/// from the lines themselves: within a hunk the old side covers exactly the
/// del+ctx lines and the new side exactly the add+ctx lines.
pub fn format_hunks(hunks: &[DiffHunk]) -> String {
    let mut out = String::new();
    for hunk in hunks {
        let old_count = hunk.lines.iter().filter(|l| l.old_no.is_some()).count();
        let new_count = hunk.lines.iter().filter(|l| l.new_no.is_some()).count();
        out.push_str(&format!(
            "@@ -{},{} +{},{} @@\n",
            hunk.old_start, old_count, hunk.new_start, new_count
        ));
        for line in &hunk.lines {
            out.push(match line.kind {
                DiffLineKind::Add => '+',
                DiffLineKind::Del => '-',
                DiffLineKind::Ctx => ' ',
            });
            out.push_str(&line.text);
            out.push('\n');
        }
    }
    out
}

/// Number of context lines on each side of a change within a hunk.
const DIFF_CONTEXT: u32 = 3;

/// Collects changed ranges via the `Sink` trait, then groups them into
/// hunks in `finish`: two changes belong to the same hunk when the number
/// of unchanged lines between them is <= 2 * DIFF_CONTEXT.
struct UnifiedDiffBuilder<'a> {
    input: &'a imara_diff::intern::InternedInput<&'a str>,
    changes: Vec<(Range<u32>, Range<u32>)>,
}

impl imara_diff::Sink for UnifiedDiffBuilder<'_> {
    type Out = Vec<DiffHunk>;

    fn process_change(&mut self, before: Range<u32>, after: Range<u32>) {
        self.changes.push((before, after));
    }

    fn finish(self) -> Vec<DiffHunk> {
        let before_total = self.input.before.len() as u32;
        let mut hunks = Vec::new();

        let mut i = 0;
        while i < self.changes.len() {
            // Extend the hunk while the gap to the next change fits in the
            // combined context of both sides.
            let mut j = i;
            while j + 1 < self.changes.len()
                && self.changes[j + 1].0.start - self.changes[j].0.end <= 2 * DIFF_CONTEXT
            {
                j += 1;
            }
            let group = &self.changes[i..=j];

            let before_start = group[0].0.start.saturating_sub(DIFF_CONTEXT);
            let before_end = (group.last().unwrap().0.end + DIFF_CONTEXT).min(before_total);
            let after_start = group[0].1.start.saturating_sub(DIFF_CONTEXT);
            // (The after-side hunk end equals after_start + hunk height:
            // unchanged regions have equal length on both sides, so the `a`
            // cursor below reaches it exactly when `b` reaches before_end.)

            let mut lines = Vec::new();
            // `a` tracks the after-side counterpart of before-line `b`:
            // unchanged regions between changes have equal length on both
            // sides, so the two cursors stay in lockstep through ctx runs.
            let mut b = before_start;
            let mut a = after_start;
            for (before, after) in group {
                while b < before.start {
                    lines.push(DiffLine::ctx(
                        &self.input.interner[self.input.before[b as usize]],
                        b + 1,
                        a + 1,
                    ));
                    b += 1;
                    a += 1;
                }
                for k in before.start..before.end {
                    lines.push(DiffLine::del(
                        &self.input.interner[self.input.before[k as usize]],
                        k + 1,
                    ));
                }
                for k in after.start..after.end {
                    lines.push(DiffLine::add(
                        &self.input.interner[self.input.after[k as usize]],
                        k + 1,
                    ));
                }
                b = before.end;
                a = after.end;
            }
            while b < before_end {
                lines.push(DiffLine::ctx(
                    &self.input.interner[self.input.before[b as usize]],
                    b + 1,
                    a + 1,
                ));
                b += 1;
                a += 1;
            }

            hunks.push(DiffHunk {
                old_start: before_start + 1,
                new_start: after_start + 1,
                lines,
            });
            i = j + 1;
        }
        hunks
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------
    // fuzzy_find
    // -----------------------------------------------------------------

    #[test]
    fn exact_match() {
        let buffer = "fn main() {\n    println!(\"hi\");\n}\n";
        let ranges = fuzzy_find(buffer, "println!(\"hi\");");
        assert_eq!(ranges.len(), 1);
        assert_eq!(&buffer[ranges[0].clone()], "    println!(\"hi\");");
        assert_eq!(ranges[0], 12..31);
    }

    #[test]
    fn empty_query_returns_nothing() {
        assert!(fuzzy_find("hello\nworld\n", "").is_empty());
    }

    #[test]
    fn fuzzy_match_differing_by_one_char() {
        let buffer = "let x = 1;\nlet total = compute_value(a, b, c);\nlet y = 2;\n";
        let ranges = fuzzy_find(buffer, "let total = compute_value(a, b, d);");
        assert_eq!(ranges.len(), 1);
        assert_eq!(
            &buffer[ranges[0].clone()],
            "let total = compute_value(a, b, c);"
        );
    }

    #[test]
    fn fuzzy_match_with_different_indentation() {
        // The query is unindented; the buffer line is indented with tabs.
        // Trimming makes them equal, so this is an exact (cost-0) match.
        let buffer = "fn main() {\n\t\tcall_me();\n}\n";
        let ranges = fuzzy_find(buffer, "call_me();");
        assert_eq!(ranges.len(), 1);
        assert_eq!(&buffer[ranges[0].clone()], "\t\tcall_me();");
    }

    #[test]
    fn no_match_returns_empty() {
        let buffer = "alpha\nbeta\ngamma\n";
        assert!(fuzzy_find(buffer, "something entirely different").is_empty());
    }

    #[test]
    fn multiple_matches_return_all_ranges() {
        let buffer = "foo\nbar\nbaz\nfoo\nbar\n";
        let ranges = fuzzy_find(buffer, "foo\nbar");
        assert_eq!(ranges.len(), 2);
        assert_eq!(&buffer[ranges[0].clone()], "foo\nbar");
        assert_eq!(&buffer[ranges[1].clone()], "foo\nbar");
        assert_eq!(ranges[0], 0..7);
        assert_eq!(ranges[1], 12..19);
    }

    #[test]
    fn match_ratio_rejects_two_line_query_missing_one_line() {
        // 2-line query, 1 line missed (skipped with Up): matched 1/2 = 0.5.
        let buffer = "foo\nbar\n";
        assert!(fuzzy_find(buffer, "foo\nxyz_missing").is_empty());
    }

    #[test]
    fn match_ratio_accepts_five_line_query_missing_one_line() {
        // 5-line query, 1 line missed: matched 4/5 = 0.8, accepted.
        let buffer = "aaa\nbbb\nccc\nddd\neee\n";
        let ranges = fuzzy_find(buffer, "aaa\nbbb\nxxx\nddd\neee");
        assert_eq!(ranges.len(), 1);
        assert_eq!(&buffer[ranges[0].clone()], "aaa\nbbb\nccc\nddd\neee");
    }

    #[test]
    fn incomplete_tail_extends_single_match() {
        // The tail "baz" is a prefix of the next buffer line, so the match
        // extends without re-running the DP.
        let buffer = "foo\nbar\nbazaar\n";
        let ranges = fuzzy_find(buffer, "foo\nbar\nbaz");
        assert_eq!(ranges.len(), 1);
        assert_eq!(&buffer[ranges[0].clone()], "foo\nbar\nbaz");
    }

    #[test]
    fn incomplete_tail_reruns_as_full_line() {
        // The tail does not prefix-match the next buffer line, so it is
        // appended as a full query line and fuzzy-matches (1-char diff).
        let buffer = "foo\nbar\nlet result = compute(t);\n";
        let ranges = fuzzy_find(buffer, "foo\nbar\nlet result = compute();");
        assert_eq!(ranges.len(), 1);
        assert_eq!(
            &buffer[ranges[0].clone()],
            "foo\nbar\nlet result = compute(t);"
        );
    }

    #[test]
    fn crlf_normalized_buffer_matches_with_lf_ranges() {
        // Callers normalize \r\n -> \n before calling fuzzy_find.
        let raw = "fn a() {\r\n    body();\r\n}\r\n";
        let buffer = raw.replace("\r\n", "\n");
        let ranges = fuzzy_find(&buffer, "body();");
        assert_eq!(ranges.len(), 1);
        assert_eq!(&buffer[ranges[0].clone()], "    body();");
    }

    // -----------------------------------------------------------------
    // first_line_indent_mismatch
    // -----------------------------------------------------------------

    #[test]
    fn indent_mismatch_detects_tab_vs_space() {
        let buffer = "fn main() {\n\t\tcall_me();\n}\n";
        let ranges = fuzzy_find(buffer, "call_me();");
        assert_eq!(ranges.len(), 1);
        // old_text without indentation -> mismatch (file has 2 tabs).
        let m = first_line_indent_mismatch(buffer, &ranges[0], "call_me();").unwrap();
        assert_eq!(m.line, 2);
        assert_eq!(m.file_indent, "\t\t");
        assert_eq!(m.old_text_indent, "");
        // Exact indentation -> no mismatch.
        assert!(first_line_indent_mismatch(buffer, &ranges[0], "\t\tcall_me();").is_none());
    }

    #[test]
    fn describe_indent_human_readable() {
        assert_eq!(describe_indent(""), "no indentation");
        assert_eq!(describe_indent("\t"), "1 tab");
        assert_eq!(describe_indent("    "), "4 spaces");
        assert_eq!(describe_indent("\t  "), "1 tab + 2 spaces");
    }

    // -----------------------------------------------------------------
    // unified_diff
    // -----------------------------------------------------------------

    #[test]
    fn unified_diff_header_and_body_format() {
        let diff = unified_diff("a\nb\nc\n", "a\nX\nc\n");
        assert_eq!(diff, "@@ -1,3 +1,3 @@\n a\n-b\n+X\n c\n");
    }

    #[test]
    fn unified_diff_uses_three_context_lines() {
        let old = "1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n";
        let new = "1\n2\n3\n4\nX\n6\n7\n8\n9\n10\n";
        let diff = unified_diff(old, new);
        assert_eq!(
            diff,
            "@@ -2,7 +2,7 @@\n 2\n 3\n 4\n-5\n+X\n 6\n 7\n 8\n"
        );
    }

    #[test]
    fn unified_diff_splits_distant_changes_into_two_hunks() {
        let old: String = (1..=20).map(|i| format!("l{i}\n")).collect();
        let new = old.replacen("l2\n", "X2\n", 1).replacen("l15\n", "X15\n", 1);
        let diff = unified_diff(&old, &new);
        let hunks: Vec<&str> = diff.split("@@ ").skip(1).collect();
        assert_eq!(hunks.len(), 2);
        assert!(hunks[0].starts_with("-1,5 +1,5 @@"), "{}", hunks[0]);
        assert!(hunks[0].contains("-l2\n+X2\n"));
        assert!(!hunks[0].contains("l15"));
        assert!(hunks[1].starts_with("-12,7 +12,7 @@"), "{}", hunks[1]);
        assert!(hunks[1].contains("-l15\n+X15\n"));
    }

    #[test]
    fn unified_diff_merges_close_changes_into_one_hunk() {
        // Changes 4 lines apart (<= 2 * context) share a hunk.
        let old: String = (1..=12).map(|i| format!("l{i}\n")).collect();
        let new = old.replacen("l3\n", "X3\n", 1).replacen("l7\n", "X7\n", 1);
        let diff = unified_diff(&old, &new);
        assert_eq!(diff.matches("@@ ").count(), 1);
        assert!(diff.starts_with("@@ -1,10 +1,10 @@\n"));
    }

    #[test]
    fn unified_diff_no_changes_is_empty() {
        assert_eq!(unified_diff("a\nb\n", "a\nb\n"), "");
    }

    // -----------------------------------------------------------------
    // structured_diff
    // -----------------------------------------------------------------

    #[test]
    fn structured_diff_line_numbers() {
        let hunks = structured_diff("a\nb\nc\n", "a\nX\nc\n");
        assert_eq!(hunks.len(), 1);
        let hunk = &hunks[0];
        assert_eq!((hunk.old_start, hunk.new_start), (1, 1));
        assert_eq!(
            hunk.lines,
            vec![
                DiffLine::ctx("a", 1, 1),
                DiffLine::del("b", 2),
                DiffLine::add("X", 2),
                DiffLine::ctx("c", 3, 3),
            ]
        );
    }

    #[test]
    fn structured_diff_tracks_new_side_after_insert() {
        // Insertions before a context line shift its new-side number.
        let hunks = structured_diff("a\n", "x\ny\na\n");
        assert_eq!(hunks.len(), 1);
        assert_eq!((hunks[0].old_start, hunks[0].new_start), (1, 1));
        assert_eq!(
            hunks[0].lines,
            vec![
                DiffLine::add("x", 1),
                DiffLine::add("y", 2),
                DiffLine::ctx("a", 1, 3),
            ]
        );
    }

    #[test]
    fn structured_diff_distant_changes_split_hunks() {
        let old: String = (1..=20).map(|i| format!("l{i}\n")).collect();
        let new = old.replacen("l2\n", "X2\n", 1).replacen("l15\n", "X15\n", 1);
        let hunks = structured_diff(&old, &new);
        assert_eq!(hunks.len(), 2);
        assert_eq!((hunks[0].old_start, hunks[0].new_start), (1, 1));
        assert_eq!((hunks[1].old_start, hunks[1].new_start), (12, 12));
        // The text form stays the formatting of the same hunks.
        assert_eq!(unified_diff(&old, &new), format_hunks(&hunks));
    }

    #[test]
    fn diff_preview_json_wire_shape() {
        let preview = DiffPreview {
            path: "/tmp/a".to_string(),
            is_new_file: false,
            hunks: structured_diff("a\nb\nc\n", "a\nX\nc\n"),
        };
        let v = serde_json::to_value(&preview).unwrap();
        assert_eq!(v["path"], "/tmp/a");
        assert_eq!(v["isNewFile"], false);
        assert_eq!(v["hunks"][0]["oldStart"], 1);
        assert_eq!(v["hunks"][0]["newStart"], 1);
        assert_eq!(
            v["hunks"][0]["lines"][1],
            serde_json::json!({ "kind": "del", "text": "b", "oldNo": 2 })
        );
        assert_eq!(
            v["hunks"][0]["lines"][2],
            serde_json::json!({ "kind": "add", "text": "X", "newNo": 2 })
        );
        assert_eq!(
            v["hunks"][0]["lines"][0],
            serde_json::json!({ "kind": "ctx", "text": "a", "oldNo": 1, "newNo": 1 })
        );
    }
}
