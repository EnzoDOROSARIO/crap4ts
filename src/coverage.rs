use std::collections::{BTreeMap, HashMap};
use std::path::Path;

/// Line coverage parsed from an LCOV trace file.
///
/// Only `SF`, `DA`, and `end_of_record` affect the result. Repeated reports for
/// the same line are combined: the line is covered if any reported hit count
/// is greater than zero.
#[derive(Debug, Clone, Default)]
pub struct Coverage {
    files: Vec<FileCoverage>,
}

#[derive(Debug, Clone)]
struct FileCoverage {
    path: String,
    lines: BTreeMap<usize, bool>,
}

impl Coverage {
    /// Parse LCOV data.
    ///
    /// Unknown LCOV records are ignored. A malformed `DA` record, a `DA`
    /// outside an `SF` record, line zero, and a negative hit count are errors.
    pub fn parse(input: &str) -> Result<Self, String> {
        let mut files: Vec<FileCoverage> = Vec::new();
        let mut current: Option<usize> = None;

        for (index, raw) in input.lines().enumerate() {
            let line_number = index + 1;
            let line = raw.strip_suffix('\r').unwrap_or(raw);
            if let Some(path) = line.strip_prefix("SF:") {
                if path.is_empty() {
                    return Err(format!("empty SF path at line {line_number}"));
                }
                files.push(FileCoverage {
                    path: path.to_owned(),
                    lines: BTreeMap::new(),
                });
                current = Some(files.len() - 1);
            } else if let Some(data) = line.strip_prefix("DA:") {
                let file_index = current
                    .ok_or_else(|| format!("DA record outside SF record at line {line_number}"))?;
                let mut fields = data.split(',');
                let source_line = fields
                    .next()
                    .and_then(|value| value.parse::<usize>().ok())
                    .filter(|value| *value > 0)
                    .ok_or_else(|| format!("invalid DA line number at line {line_number}"))?;
                let hits = fields
                    .next()
                    .and_then(|value| value.parse::<i64>().ok())
                    .filter(|value| *value >= 0)
                    .ok_or_else(|| format!("invalid DA hit count at line {line_number}"))?;
                // LCOV permits one optional checksum after the hit count.
                if fields.next().is_some_and(str::is_empty) || fields.next().is_some() {
                    return Err(format!("malformed DA record at line {line_number}"));
                }
                files[file_index]
                    .lines
                    .entry(source_line)
                    .and_modify(|covered| *covered |= hits > 0)
                    .or_insert(hits > 0);
            } else if line == "end_of_record" {
                current = None;
            }
        }

        Ok(Self { files })
    }

    /// Return the covered fraction of reported lines in the inclusive range.
    ///
    /// Both the queried path and relative LCOV `SF` paths are resolved
    /// lexically against `root`; the filesystem is never consulted. Exact
    /// normalized matches take precedence. If there is no exact match, a
    /// component-suffix match is accepted only when it identifies one unique
    /// reported path. No matching path or no tracked lines returns `None`.
    pub fn for_function(
        &self,
        file: &Path,
        root: &Path,
        start_line: usize,
        end_line: usize,
    ) -> Option<f64> {
        let root = CanonPath::from_text(&root.to_string_lossy(), None)?;
        let wanted = CanonPath::from_text(&file.to_string_lossy(), Some(&root))?;
        // Preserve the root-relative spelling for portable reports whose
        // absolute build directory differs from the current checkout.
        let unrooted = CanonPath::from_text(&file.to_string_lossy(), None)?;
        let portable_components = if unrooted.prefix.is_empty() {
            unrooted.components
        } else {
            wanted
                .strip_prefix(&root)
                .unwrap_or_else(|| wanted.components.clone())
        };
        let mut grouped: HashMap<CanonPath, BTreeMap<usize, bool>> = HashMap::new();

        for report in &self.files {
            let Some(path) = CanonPath::from_text(&report.path, Some(&root)) else {
                continue;
            };
            if path != wanted && !path.ends_with_components(&portable_components) {
                continue;
            }
            let lines = grouped.entry(path).or_default();
            for (&line, &covered) in &report.lines {
                lines
                    .entry(line)
                    .and_modify(|old| *old |= covered)
                    .or_insert(covered);
            }
        }

        let lines = if let Some(exact) = grouped.get(&wanted) {
            exact
        } else {
            let mut matches = grouped
                .iter()
                .filter(|(path, _)| path.ends_with_components(&portable_components));
            let (_, first) = matches.next()?;
            if matches.next().is_some() {
                return None;
            }
            first
        };

        let mut total = 0usize;
        let mut covered = 0usize;
        if start_line <= end_line {
            for (_, is_covered) in lines.range(start_line..=end_line) {
                total += 1;
                covered += usize::from(*is_covered);
            }
        }
        (total != 0).then(|| covered as f64 / total as f64)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct CanonPath {
    prefix: String,
    components: Vec<String>,
}

impl CanonPath {
    fn from_text(text: &str, root: Option<&Self>) -> Option<Self> {
        let decoded = decode_file_uri(text)?;
        let replaced = decoded.replace('\\', "/");
        let (prefix, absolute, rest) = path_prefix(&replaced);
        let mut components = if absolute {
            Vec::new()
        } else {
            root.map(|path| path.components.clone()).unwrap_or_default()
        };
        let final_prefix = if absolute {
            prefix
        } else {
            root.map(|path| path.prefix.clone()).unwrap_or(prefix)
        };
        for component in rest.split('/') {
            match component {
                "" | "." => {}
                ".." => {
                    if components.last().is_some_and(|part| part != "..") {
                        components.pop();
                    } else if final_prefix.is_empty() {
                        components.push("..".to_owned());
                    }
                }
                part => components.push(part.to_owned()),
            }
        }
        Some(Self {
            prefix: final_prefix,
            components,
        })
    }

    fn ends_with_components(&self, suffix: &[String]) -> bool {
        !suffix.is_empty()
            && self.components.len() >= suffix.len()
            && self.components[self.components.len() - suffix.len()..] == *suffix
    }

    fn strip_prefix(&self, base: &Self) -> Option<Vec<String>> {
        (self.prefix == base.prefix && self.components.starts_with(&base.components))
            .then(|| self.components[base.components.len()..].to_vec())
    }
}

fn path_prefix(path: &str) -> (String, bool, &str) {
    if let Some(verbatim) = path.strip_prefix("//?/") {
        let bytes = verbatim.as_bytes();
        if bytes.len() >= 3
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && bytes[2] == b'/'
        {
            return (verbatim[..2].to_owned(), true, &verbatim[3..]);
        }
        if verbatim
            .get(..4)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("UNC/"))
        {
            return ("//".to_owned(), true, &verbatim[4..]);
        }
    }
    let bytes = path.as_bytes();
    if bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] == b'/' {
        return (path[..2].to_owned(), true, &path[3..]);
    }
    if let Some(rest) = path.strip_prefix("//") {
        return ("//".to_owned(), true, rest);
    }
    if let Some(rest) = path.strip_prefix('/') {
        return ("/".to_owned(), true, rest);
    }
    (String::new(), false, path)
}

fn decode_file_uri(text: &str) -> Option<String> {
    let Some(mut encoded) = text.strip_prefix("file://") else {
        return Some(text.to_owned());
    };
    if let Some(rest) = encoded.strip_prefix("localhost/") {
        encoded = rest;
        let decoded = percent_decode(encoded)?;
        return Some(format!("/{decoded}"));
    }
    let decoded = percent_decode(encoded)?;
    // file:///C:/... is a Windows drive path, not a Unix path named `C:`.
    if decoded.starts_with('/')
        && decoded.as_bytes().get(2) == Some(&b':')
        && decoded
            .as_bytes()
            .get(1)
            .is_some_and(u8::is_ascii_alphabetic)
    {
        return Some(decoded[1..].to_owned());
    }
    Some(decoded)
}

fn percent_decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let hex = bytes.get(index + 1..index + 3)?;
            let value = u8::from_str_radix(std::str::from_utf8(hex).ok()?, 16).ok()?;
            output.push(value);
            index += 3;
        } else {
            output.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(output).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn computes_fraction_with_inclusive_boundaries_and_duplicate_lines() {
        let coverage =
            Coverage::parse("SF:src/a.ts\nDA:2,0\nDA:3,0\nDA:3,4\nDA:4,0\nDA:9,7\nend_of_record\n")
                .unwrap();
        assert_eq!(
            coverage.for_function(Path::new("src/a.ts"), Path::new("/p"), 2, 3),
            Some(0.5)
        );
        assert_eq!(
            coverage.for_function(Path::new("src/a.ts"), Path::new("/p"), 3, 4),
            Some(0.5)
        );
        assert_eq!(
            coverage.for_function(Path::new("src/a.ts"), Path::new("/p"), 5, 8),
            None
        );
    }

    #[test]
    fn merges_duplicate_file_records_using_any_positive_hit() {
        let coverage = Coverage::parse(
            "SF:src/a.ts\nDA:1,0\nend_of_record\nSF:src/./a.ts\nDA:1,2\nDA:2,0\nend_of_record\n",
        )
        .unwrap();
        assert_eq!(
            coverage.for_function(Path::new("src/a.ts"), Path::new("/p"), 1, 2),
            Some(0.5)
        );
    }

    #[test]
    fn normalizes_uri_escapes_backslashes_and_relative_components() {
        let coverage = Coverage::parse(
            "SF:file:///work/my%20project/src\\lib/../a.ts\nDA:7,1\nend_of_record\n",
        )
        .unwrap();
        assert_eq!(
            coverage.for_function(Path::new("./src/a.ts"), Path::new("/work/my project"), 7, 7),
            Some(1.0)
        );
    }

    #[test]
    fn portable_suffix_match_must_be_unique_and_exact_wins() {
        let coverage = Coverage::parse(
            "SF:/build/one/src/a.ts\nDA:1,1\nend_of_record\nSF:/build/two/src/a.ts\nDA:1,0\nend_of_record\n",
        )
        .unwrap();
        assert_eq!(
            coverage.for_function(Path::new("src/a.ts"), Path::new("/local"), 1, 1),
            None
        );
        assert_eq!(
            coverage.for_function(Path::new("/build/one/src/a.ts"), Path::new("/local"), 1, 1),
            Some(1.0)
        );
    }

    #[test]
    fn portable_paths_match_by_components_and_windows_drives_normalize() {
        let coverage = Coverage::parse("SF:/build/src/a.ts\nDA:1,1\nend_of_record\nSF:/build/not-src/a.ts\nDA:1,0\nend_of_record\n").unwrap();
        assert_eq!(
            coverage.for_function(Path::new("/local/src/a.ts"), Path::new("/local"), 1, 1),
            Some(1.0)
        );
        let windows =
            Coverage::parse("SF:file:///C:/project/src/a.ts\nDA:1,0\nDA:2,1\nend_of_record\n")
                .unwrap();
        assert_eq!(
            windows.for_function(
                Path::new(r"C:\project\src\a.ts"),
                Path::new(r"C:\project"),
                1,
                2
            ),
            Some(0.5)
        );
    }

    #[test]
    fn verbatim_windows_drive_path_preserves_exact_match_precedence() {
        let coverage = Coverage::parse(
            "SF:C:/p/src/a.ts\nDA:1,0\nend_of_record\nSF:D:/build/src/a.ts\nDA:1,1\nend_of_record\n",
        )
        .unwrap();
        assert_eq!(
            coverage.for_function(
                Path::new(r"\\?\C:\p\src\a.ts"),
                Path::new(r"\\?\C:\p"),
                1,
                1
            ),
            Some(0.0)
        );
    }

    #[test]
    fn relative_and_absolute_windows_records_merge_with_verbatim_query() {
        let coverage = Coverage::parse(
            "SF:src/a.ts\nDA:1,0\nend_of_record\nSF:C:/p/src/a.ts\nDA:1,1\nend_of_record\n",
        )
        .unwrap();
        assert_eq!(
            coverage.for_function(
                Path::new(r"\\?\C:\p\src\a.ts"),
                Path::new(r"\\?\C:\p"),
                1,
                1
            ),
            Some(1.0)
        );
    }

    #[test]
    fn verbatim_and_ordinary_unc_paths_are_equivalent() {
        let coverage = Coverage::parse(
            "SF://server/share/p/src/a.ts\nDA:4,1\nend_of_record\nSF://other/share/p/src/a.ts\nDA:4,0\nend_of_record\n",
        )
        .unwrap();
        assert_eq!(
            coverage.for_function(
                Path::new(r"\\?\UNC\server\share\p\src\a.ts"),
                Path::new(r"\\?\UNC\server\share\p"),
                4,
                4
            ),
            Some(1.0)
        );
    }

    #[test]
    fn rejects_bad_da_records() {
        for input in [
            "DA:1,1\n",
            "SF:a.ts\nDA:0,1\n",
            "SF:a.ts\nDA:x,1\n",
            "SF:a.ts\nDA:1,-1\n",
            "SF:a.ts\nDA:1\n",
            "SF:a.ts\nDA:1,1,,\n",
        ] {
            assert!(Coverage::parse(input).is_err(), "accepted {input:?}");
        }
    }

    #[test]
    fn empty_coverage_has_no_result() {
        assert_eq!(
            Coverage::parse("")
                .unwrap()
                .for_function(Path::new("a.ts"), Path::new("/p"), 1, 9),
            None
        );
    }
}
