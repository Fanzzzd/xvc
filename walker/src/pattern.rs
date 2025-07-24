use std::path::{Path, PathBuf};

/// The result of matching a path against a set of ignore patterns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MatchResult {
    /// The path did not match any pattern.
    NoMatch,
    /// The path matched an ignore pattern.
    Ignore,
    /// The path matched a whitelist (negation) pattern.
    Whitelist,
}

/// Describes how a pattern's path is interpreted.
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub enum PatternRelativity {
    /// The pattern can match anywhere in the directory tree.
    Anywhere,
    /// The pattern is relative to a specific directory.
    RelativeTo {
        /// The directory to which the pattern is relative.
        directory: String,
    },
}

/// The type of path a pattern can match.
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub enum PathKind {
    /// The pattern can match a file or a directory.
    Any,
    /// The pattern specifically matches a directory.
    Directory,
}

/// The effect of a pattern when it matches a path.
#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub enum PatternEffect {
    /// The matched path should be ignored.
    Ignore,
    /// The matched path should be included (negated ignore).
    Whitelist,
}

/// The origin of a pattern.
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub enum Source {
    /// The pattern is from a global configuration.
    Global,
    /// The pattern was read from a file.
    File {
        /// The path to the file containing the pattern.
        path: PathBuf,
        /// The line number in the file where the pattern was found.
        line: usize,
    },
    /// The pattern was provided via the command line.
    CommandLine {
        /// The current working directory when the command was invoked.
        current_dir: PathBuf,
    },
}

impl Source {
    /// Returns the directory path of the source, if applicable.
    pub fn dir_path(&self) -> Option<PathBuf> {
        match self {
            Source::File { path, .. } => path.parent().map(Path::to_path_buf),
            Source::Global => Some(PathBuf::from("")),
            Source::CommandLine { current_dir } => Some(current_dir.clone()),
        }
    }
}

/// Represents a single ignore pattern and its properties.
#[derive(Debug)]
pub struct Pattern {
    /// The compiled glob pattern string.
    pub glob: String,
    /// The original, unmodified pattern string.
    pub original: String,
    /// The source of the pattern.
    pub source: Source,
    /// The effect of the pattern (ignore or whitelist).
    pub effect: PatternEffect,
    /// The relativity of the pattern's path.
    pub relativity: PatternRelativity,
    /// The kind of path this pattern applies to (file, directory, or any).
    pub path_kind: PathKind,
}

impl Pattern {
    /// Creates a new `Pattern` from a source and an original string.
    pub fn new(source: Source, original: &str) -> Self {
        let original_owned = original.to_owned();
        let mut current_dir = match &source {
            Source::Global => "".to_string(),
            Source::File { path, .. } => {
                let parent = path.parent().unwrap_or_else(|| "".as_ref());
                parent.to_string_lossy().to_string()
            }
            Source::CommandLine { current_dir } => current_dir.to_string_lossy().to_string(),
        };

        if current_dir.ends_with('/') {
            current_dir = current_dir[..current_dir.len() - 1].to_string();
        }

        let begin_exclamation = original.starts_with('!');
        let mut line = if original.starts_with(r"\!") {
            original[1..].to_owned()
        } else if begin_exclamation {
            original[1..].to_owned()
        } else {
            original.to_owned()
        };

        if !line.ends_with("\\ ") {
            line = line.trim_end().to_string();
        }

        let end_slash = line.ends_with('/');
        if end_slash {
            line = line[..line.len() - 1].to_string()
        }

        let begin_slash = line.starts_with('/');
        if begin_slash {
            line = line[1..].to_string();
        }

        let contains_slash = line.contains('/');

        let effect = if begin_exclamation {
            PatternEffect::Whitelist
        } else {
            PatternEffect::Ignore
        };

        let mut path_kind = if end_slash {
            PathKind::Directory
        } else {
            PathKind::Any
        };

        if line.ends_with("**") {
            path_kind = PathKind::Directory;
        }

        let relativity = if begin_slash || contains_slash {
            PatternRelativity::RelativeTo {
                directory: current_dir.clone(),
            }
        } else {
            PatternRelativity::Anywhere
        };

        let mut glob = if begin_slash || contains_slash {
            if current_dir.is_empty() {
                line.to_string()
            } else {
                format!("{current_dir}/{line}")
            }
        } else if current_dir.is_empty() {
            format!("**/{line}")
        } else {
            format!("{current_dir}/**/{line}")
        };

        if path_kind == PathKind::Directory {
            glob.push('/');
        }

        Pattern {
            glob,
            original: original_owned,
            source,
            effect,
            relativity,
            path_kind,
        }
    }
}

/// Builds a list of `Pattern`s from a vector of strings.
pub fn build_pattern_list(patterns: Vec<String>, source: Source) -> Vec<Pattern> {
    patterns
        .iter()
        .map(|p| Pattern::new(source.clone(), p))
        .collect()
}