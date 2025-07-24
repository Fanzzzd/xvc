//! Xvc walker traverses directory trees with ignore rules.
//!
//! Ignore rules are similar to [.gitignore](https://git-scm.com/docs/gitignore) and child
//! directories are not traversed if ignored.
//!
//! [walk_parallel] function is the most useful element in this module.
//! It walks and sends [PathMetadata] through a channel, also updating the ignore rules and sending
//! them.
#![warn(missing_docs)]
#![forbid(unsafe_code)]
pub mod abspath;
pub mod error;
mod glob;
/// Rules for ignoring paths during directory traversal.
pub mod ignore_rules;
pub mod notify;
/// Defines patterns for ignore rules.
pub mod pattern;
pub mod sync;
/// Parallel directory traversal.
pub mod walk_parallel;
pub mod walk_serial;

pub use pattern::MatchResult;
pub use pattern::PathKind;
pub use pattern::Pattern;
pub use pattern::PatternEffect;
pub use pattern::PatternRelativity;
pub use pattern::Source;

pub use walk_parallel::walk_parallel;
pub use walk_serial::walk_serial;

pub use walk_serial::path_metadata_map_from_file_targets;

pub use abspath::AbsolutePath;
pub use error::{Error, Result};

pub use ignore_rules::content_to_patterns;
pub use ignore_rules::IgnoreRules;
pub use ignore_rules::SharedIgnoreRules;

pub use std::hash::Hash;
pub use sync::{PathSync, PathSyncSingleton};
use xvc_logging::warn;

pub use notify::make_polling_watcher;
pub use notify::make_watcher;
pub use notify::PathEvent;
pub use notify::RecommendedWatcher;

use std::{
    fmt::Debug,
    fs::{self, Metadata},
    path::{Path, PathBuf},
};

use anyhow::anyhow;

static MAX_THREADS_PARALLEL_WALK: usize = 8;

/// Combine a path and its metadata in a single struct
#[derive(Debug, Clone)]
pub struct PathMetadata {
    /// path
    pub path: PathBuf,
    /// metadata
    pub metadata: Metadata,
}

/// Options to configure directory walking.
#[derive(Debug, Clone)]
pub struct WalkOptions {
    /// The ignore filename (`.gitignore`, `.xvcignore`, `.ignore`, etc.) or `None` for not
    /// ignoring anything.
    pub ignore_filename: Option<String>,
    /// Whether to ignore the `.git` directory.
    pub ignore_dot_git: bool,
}

impl WalkOptions {
    /// Instantiate a Git repository walker that uses `.gitignore` as ignore file name.
    pub fn gitignore() -> Self {
        Self {
            ignore_filename: Some(".gitignore".into()),
            ignore_dot_git: true,
        }
    }

    /// Instantiate a Xvc repository walker that uses `.xvcignore` as ignore file name.
    pub fn xvcignore() -> Self {
        Self {
            ignore_filename: Some(".xvcignore".into()),
            ignore_dot_git: true,
        }
    }
}

/// Build the ignore rules with the given directory
pub fn build_ignore_patterns(
    given: &str,
    ignore_root: &Path,
    ignore_filename: &str,
) -> Result<IgnoreRules> {
    let ignore_rules = IgnoreRules::from_global_patterns(ignore_root, Some(ignore_filename), given);

    let mut dir_stack: Vec<PathBuf> = vec![ignore_root.to_path_buf()];
    let ignore_fn = ignore_rules.ignore_filename.as_deref().unwrap();

    while let Some(dir) = dir_stack.pop() {
        let ignore_file = dir.join(ignore_fn);
        if ignore_file.is_file() {
            let ignore_content = fs::read_to_string(&ignore_file)?;
            let new_patterns =
                content_to_patterns(ignore_root, Some(&ignore_file), &ignore_content);
            ignore_rules.add_patterns(new_patterns)?;
        }

        if !dir.is_dir() {
            continue;
        }

        let mut subdirs: Vec<PathBuf> = dir
            .read_dir()?
            .filter_map(std::result::Result::ok)
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect();

        subdirs.sort_by(|a, b| b.cmp(a));

        let filtered_subdirs: Vec<_> = subdirs
            .into_iter()
            .filter(|p| {
                matches!(
                    ignore_rules.check(p),
                    MatchResult::NoMatch | MatchResult::Whitelist
                )
            })
            .collect();

        dir_stack.extend(filtered_subdirs.into_iter());
    }

    Ok(ignore_rules)
}

/// Updates the ignore rules from a given directory.
pub fn update_ignore_rules(dir: &Path, ignore_rules: &IgnoreRules) -> Result<()> {
    if let Some(ref ignore_filename) = ignore_rules.ignore_filename {
        let ignore_root = &ignore_rules.root;
        let ignore_path = dir.join(ignore_filename);
        if ignore_path.is_file() {
            let new_patterns: Vec<Pattern> = {
                let content = fs::read_to_string(&ignore_path)?;
                content_to_patterns(ignore_root, Some(ignore_path).as_deref(), &content)
            };

            ignore_rules.add_patterns(new_patterns)?;
        }
    }
    Ok(())
}
/// Return all childs of a directory regardless of any ignore rules
pub fn directory_list(dir: &Path) -> Result<Vec<Result<PathMetadata>>> {
    let elements = dir
        .read_dir()
        .map_err(|e| anyhow!("Error reading directory: {:?}, {:?}", dir, e))?;
    let mut child_paths = Vec::<Result<PathMetadata>>::new();

    for entry in elements {
        match entry {
            Err(err) => child_paths.push(Err(Error::from(anyhow!(
                "Error reading entry in dir {:?} {:?}",
                dir,
                err
            )))),
            Ok(entry) => match entry.metadata() {
                Err(err) => child_paths.push(Err(Error::from(anyhow!(
                    "Error getting metadata {:?} {:?}",
                    entry,
                    err
                )))),
                Ok(md) => {
                    child_paths.push(Ok(PathMetadata {
                        path: entry.path(),
                        metadata: md.clone(),
                    }));
                }
            },
        }
    }
    Ok(child_paths)
}