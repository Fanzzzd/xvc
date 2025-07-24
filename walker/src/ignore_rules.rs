use crate::{pattern::PatternEffect, Result, Source};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use crate::glob::glob_match;
use crate::pattern::{MatchResult, Pattern, PathKind};

/// A set of rules to determine whether a path should be ignored.
#[derive(Debug, Clone)]
pub struct IgnoreRules {
    /// The root directory for which these ignore rules apply.
    pub root: PathBuf,
    /// The name of the ignore file (e.g., `.gitignore`).
    pub ignore_filename: Option<String>,
    /// A list of patterns that define the ignore rules.
    pub patterns: Arc<RwLock<Vec<Pattern>>>,
}

/// A thread-safe, reference-counted pointer to `IgnoreRules`.
pub type SharedIgnoreRules = Arc<RwLock<IgnoreRules>>;

fn pattern_has_wildcard(p: &str) -> bool {
    p.contains('*') || p.contains('?') || p.contains('[')
}

impl IgnoreRules {
    /// Creates an empty set of ignore rules for a given directory.
    pub fn empty(dir: &Path, ignore_filename: Option<&str>) -> Self {
        IgnoreRules {
            root: PathBuf::from(dir),
            ignore_filename: ignore_filename.map(|s| s.to_string()),
            patterns: Arc::new(RwLock::new(Vec::<Pattern>::new())),
        }
    }

    /// Creates ignore rules from a string of global patterns.
    pub fn from_global_patterns(
        ignore_root: &Path,
        ignore_filename: Option<&str>,
        given: &str,
    ) -> Self {
        let mut given_patterns = Vec::<Pattern>::new();
        for line in given.lines() {
            let pattern = Pattern::new(Source::Global, line);
            given_patterns.push(pattern);
        }
        IgnoreRules::from_patterns(ignore_root, ignore_filename, given_patterns)
    }

    /// Creates ignore rules from a vector of `Pattern`s.
    pub fn from_patterns(
        ignore_root: &Path,
        ignore_filename: Option<&str>,
        patterns: Vec<Pattern>,
    ) -> Self {
        IgnoreRules {
            root: PathBuf::from(ignore_root),
            ignore_filename: ignore_filename.map(|s| s.to_string()),
            patterns: Arc::new(RwLock::new(patterns)),
        }
    }

    /// Checks if a given path matches any of the ignore rules.
    pub fn check(&self, path: &Path) -> MatchResult {
        let relative_path = path.strip_prefix(&self.root).expect("path must be within root");
        let mut path_str = relative_path.to_string_lossy().to_string();
        if path_str.is_empty() && path.is_dir() {
            path_str = "/".to_string();
        } else if path.is_dir() && !path_str.ends_with('/') {
            path_str.push('/');
        }

        let patterns = self.patterns.read().unwrap();

        let mut ignore_match: Option<&Pattern> = None;
        let mut whitelist_match: Option<&Pattern> = None;

        for pattern in patterns.iter().rev() {
            if ignore_match.is_some() && whitelist_match.is_some() {
                break;
            }

            if let Source::File {
                path: ignore_file_path,
                ..
            } = &pattern.source
            {
                if let Some(ignore_file_dir) = ignore_file_path.parent() {
                    if ignore_file_dir == relative_path {
                        continue;
                    }
                }
            }

            let matches = if path.is_dir() {
                if pattern.glob.ends_with("/*") {
                    if let Some(glob_prefix) = pattern.glob.strip_suffix("/*") {
                        if relative_path.to_string_lossy() == glob_prefix {
                            false
                        } else {
                            glob_match(&pattern.glob, &path_str)
                                || glob_match(&pattern.glob, path_str.trim_end_matches('/'))
                        }
                    } else {
                        // This case should not be reachable
                        glob_match(&pattern.glob, &path_str)
                            || glob_match(&pattern.glob, path_str.trim_end_matches('/'))
                    }
                } else {
                    glob_match(&pattern.glob, &path_str)
                        || glob_match(&pattern.glob, path_str.trim_end_matches('/'))
                }
            } else {
                glob_match(&pattern.glob, &path_str)
            };

            if matches {
                if pattern.path_kind == PathKind::Directory && !path.is_dir() {
                    continue;
                }
                match pattern.effect {
                    PatternEffect::Ignore if ignore_match.is_none() => {
                        ignore_match = Some(pattern);
                    }
                    PatternEffect::Whitelist if whitelist_match.is_none() => {
                        whitelist_match = Some(pattern);
                    }
                    _ => {}
                }
            }
        }

        match (ignore_match, whitelist_match) {
            (None, None) => MatchResult::NoMatch,
            (Some(_), None) => MatchResult::Ignore,
            (None, Some(_)) => MatchResult::Whitelist,
            (Some(im_pattern), Some(wm_pattern)) => {
                let im_source_dir = im_pattern.source.dir_path();
                let wm_source_dir = wm_pattern.source.dir_path();

                if let (Some(isd), Some(wsd)) = (im_source_dir, wm_source_dir) {
                    if wsd.starts_with(&isd) && wsd != isd {
                        let has_slash = wm_pattern.original.contains('/');
                        let has_wildcard = pattern_has_wildcard(&wm_pattern.original);
                        if !has_slash && !has_wildcard {
                            return MatchResult::Ignore;
                        }
                    }
                }
                if patterns
                    .iter()
                    .position(|p| p.original == wm_pattern.original)
                    > patterns
                        .iter()
                        .position(|p| p.original == im_pattern.original)
                {
                    MatchResult::Whitelist
                } else {
                    MatchResult::Ignore
                }
            }
        }
    }

    /// Merges another set of ignore rules into this one.
    pub fn merge_with(&self, other: &IgnoreRules) -> Result<()> {
        assert_eq!(self.root, other.root);

        {
            let mut patterns = self.patterns.write().unwrap();
            let mut other_patterns = other.patterns.write().unwrap();
            other_patterns.drain(..).for_each(|p| patterns.push(p));
        }

        Ok(())
    }
    /// Adds a vector of `Pattern`s to the existing rules.
    pub fn add_patterns(&self, patterns: Vec<Pattern>) -> Result<()> {
        let other = IgnoreRules::from_patterns(&self.root, None, patterns);
        self.merge_with(&other)
    }
}

/// convert a set of rules in `content` to glob patterns.
pub fn content_to_patterns(
    ignore_root: &Path,
    source: Option<&Path>,
    content: &str,
) -> Vec<Pattern> {
    content
        .lines()
        .enumerate()
        .filter(|(_, line)| !(line.trim().is_empty() || line.starts_with('#')))
        .map(|(i, line)| {
            if !line.ends_with("\\ ") {
                (i, line.trim_end())
            } else {
                (i, line)
            }
        })
        .map(|(i, line)| {
            (
                line,
                match source {
                    Some(p) => Source::File {
                        path: p
                            .strip_prefix(ignore_root)
                            .expect("path must be within ignore_root")
                            .to_path_buf(),
                        line: (i + 1),
                    },
                    None => Source::Global,
                },
            )
        })
        .map(|(line, source)| Pattern::new(source, line))
        .collect()
}