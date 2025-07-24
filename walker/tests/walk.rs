use anyhow::Result;
use crossbeam_channel::unbounded;
use git2::{Repository, Status, StatusOptions};
use log::LevelFilter;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::thread;
use xvc_test_helper::{create_temp_dir, test_logging};
use xvc_walker::{build_ignore_patterns, walk_parallel, WalkOptions};

macro_rules! assert_eq_and_print {
    ($result:expr, $expected:expr) => {
        let mut result_sorted: Vec<_> = $result.iter().cloned().collect();
        result_sorted.sort();
        let mut expected_sorted: Vec<_> = $expected.iter().cloned().collect();
        expected_sorted.sort();

        if result_sorted == expected_sorted {
            println!("\nResult (and expected): {:?}", result_sorted);
        } else {
            println!("\nResult:   {:?}", result_sorted);
            println!("Expected: {:?}", expected_sorted);
        }
        assert_eq!($result, $expected);
    };
}

fn setup_test_directory(structure: &[&str], ignore_files: &[(&str, &str)]) -> Result<PathBuf> {
    let root = create_temp_dir();
    Repository::init(&root)?;

    for path_str in structure {
        let path = root.join(path_str);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, path_str)?;
    }

    for (path_str, content) in ignore_files {
        let path = root.join(path_str);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, content)?;
    }

    Ok(root)
}

fn run_walk(root: &Path, ignore_filename: &str) -> Result<HashSet<String>> {
    let (path_sender, path_receiver) = unbounded();
    let ignore_rules = Arc::new(RwLock::new(build_ignore_patterns(
        "",
        root,
        ignore_filename,
    )?));

    let root_owned = root.to_path_buf();
    let walk_options = WalkOptions {
        ignore_filename: Some(ignore_filename.to_string()),
        ignore_dot_git: true,
    };

    let walk_thread =
        thread::spawn(move || walk_parallel(ignore_rules, &root_owned, walk_options, path_sender));

    let mut found_paths = HashSet::new();
    for path_res in path_receiver {
        let path_meta = path_res?;
        let relative_path = path_meta.path.strip_prefix(root).unwrap();
        found_paths.insert(relative_path.to_str().unwrap().replace('\\', "/"));
    }

    walk_thread.join().unwrap()?;

    let mut all_paths = HashSet::new();
    for path_str in found_paths {
        all_paths.insert(path_str.clone());

        let path = PathBuf::from(&path_str);
        let mut current = path.parent();
        while let Some(p) = current {
            if let Some(s) = p.to_str() {
                if !s.is_empty() {
                    all_paths.insert(s.replace('\\', "/"));
                }
            }
            current = p.parent();
        }
    }

    Ok(all_paths)
}

fn get_git_expected_paths(root: &Path) -> Result<HashSet<String>> {
    let repo = Repository::open(root)?;
    let mut opts = StatusOptions::new();
    opts.include_untracked(true).recurse_untracked_dirs(true);

    let statuses = repo.statuses(Some(&mut opts))?;
    let mut expected_paths = HashSet::new();

    for entry in statuses.iter() {
        if entry.status() == Status::IGNORED {
            continue;
        }

        let path_str = entry.path().unwrap().to_string().replace('\\', "/");

        let path = PathBuf::from(&path_str);
        let mut current = path.parent();
        while let Some(p) = current {
            if p.as_os_str().is_empty() {
                break;
            }
            let s = p.to_string_lossy().to_string().replace('\\', "/");
            if !s.is_empty() {
                expected_paths.insert(s);
            }
            current = p.parent();
        }

        expected_paths.insert(path_str);
    }
    Ok(expected_paths)
}

#[test]
fn test_simple_ignore() -> Result<()> {
    test_logging(LevelFilter::Trace);
    let root = setup_test_directory(&["a.js", "b.txt"], &[(".gitignore", "*.js")])?;
    let result = run_walk(&root, ".gitignore")?;
    let expected = get_git_expected_paths(&root)?;
    assert_eq_and_print!(result, expected);
    Ok(())
}

#[test]
fn test_negation() -> Result<()> {
    test_logging(LevelFilter::Trace);
    let root =
        setup_test_directory(&["a.js", "b.js", "c.txt"], &[(".gitignore", "*.js\n!b.js")])?;
    let result = run_walk(&root, ".gitignore")?;
    let expected = get_git_expected_paths(&root)?;
    assert_eq_and_print!(result, expected);
    Ok(())
}

#[test]
fn test_directory_ignore() -> Result<()> {
    test_logging(LevelFilter::Trace);
    let root =
        setup_test_directory(&["dir/a.js", "dir/b.txt", "c.txt"], &[(".gitignore", "dir/")])?;
    let result = run_walk(&root, ".gitignore")?;
    let expected = get_git_expected_paths(&root)?;
    assert_eq_and_print!(result, expected);
    Ok(())
}

#[test]
fn test_whitelisting_in_ignored_dir_is_not_traversed() -> Result<()> {
    test_logging(LevelFilter::Trace);
    let root = setup_test_directory(
        &["dir/a.js", "dir/b.txt"],
        &[(".gitignore", "dir/\n!dir/b.txt")],
    )?;
    let result = run_walk(&root, ".gitignore")?;
    let expected = get_git_expected_paths(&root)?;
    assert_eq_and_print!(result, expected);
    Ok(())
}

#[test]
fn test_nested_ignore_files() -> Result<()> {
    test_logging(LevelFilter::Trace);
    let root = setup_test_directory(
        &["a.txt", "dir1/b.txt", "dir1/c.js", "dir2/d.txt", "dir2/e.js"],
        &[(".gitignore", "*.js"), ("dir1/.gitignore", "!c.js\nb.txt")],
    )?;
    let result = run_walk(&root, ".gitignore")?;
    let expected = get_git_expected_paths(&root)?;
    assert_eq_and_print!(result, expected);
    Ok(())
}

#[test]
fn test_globstar() -> Result<()> {
    test_logging(LevelFilter::Trace);
    let root = setup_test_directory(&["a/b/c.js", "a/d.js"], &[(".gitignore", "a/**/*.js")])?;
    let result = run_walk(&root, ".gitignore")?;
    let expected = get_git_expected_paths(&root)?;
    assert_eq_and_print!(result, expected);
    Ok(())
}

#[test]
fn test_root_relative_ignore() -> Result<()> {
    test_logging(LevelFilter::Trace);
    let root = setup_test_directory(&["a.js", "dir/a.js"], &[(".gitignore", "/a.js")])?;
    let result = run_walk(&root, ".gitignore")?;
    let expected = get_git_expected_paths(&root)?;
    assert_eq_and_print!(result, expected);
    Ok(())
}

#[test]
fn test_ignore_specific_filename_anywhere() -> Result<()> {
    test_logging(LevelFilter::Trace);
    let root = setup_test_directory(
        &["config.json", "src/config.json", "app/main.js"],
        &[(".gitignore", "config.json")],
    )?;
    let result = run_walk(&root, ".gitignore")?;
    let expected = get_git_expected_paths(&root)?;
    assert_eq_and_print!(result, expected);
    Ok(())
}

#[test]
fn test_character_class_in_pattern() -> Result<()> {
    test_logging(LevelFilter::Trace);
    let root = setup_test_directory(
        &["data1.csv", "data2.csv", "dataA.csv", "other.txt"],
        &[(".gitignore", "data[0-9].csv")],
    )?;
    let result = run_walk(&root, ".gitignore")?;
    let expected = get_git_expected_paths(&root)?;
    assert_eq_and_print!(result, expected);
    Ok(())
}

#[test]
fn test_whitelisting_subdirectory_in_ignored_directory() -> Result<()> {
    test_logging(LevelFilter::Trace);
    let root = setup_test_directory(
        &["output/logs/a.log", "output/data/b.dat", "config.txt"],
        &[(".gitignore", "output/\n!output/data/")],
    )?;
    let result = run_walk(&root, ".gitignore")?;
    let expected = get_git_expected_paths(&root)?;
    assert_eq_and_print!(result, expected);
    Ok(())
}

#[test]
fn test_whitelisting_subdirectory_in_ignored_directory_2() -> Result<()> {
    test_logging(LevelFilter::Trace);
    let root = setup_test_directory(
        &["output/logs/a.log", "output/data/b.dat", "config.txt"],
        &[(".gitignore", "output/**\n!output/data/**")],
    )?;
    let result = run_walk(&root, ".gitignore")?;
    let expected = get_git_expected_paths(&root)?;
    assert_eq_and_print!(result, expected);
    Ok(())
}

#[test]
fn test_escaped_negation_pattern() -> Result<()> {
    test_logging(LevelFilter::Trace);
    let root = setup_test_directory(
        &["!important.txt", "normal.txt"],
        &[(".gitignore", r"\!important.txt")],
    )?;
    let result = run_walk(&root, ".gitignore")?;
    let expected = get_git_expected_paths(&root)?;
    assert_eq_and_print!(result, expected);
    Ok(())
}

#[test]
fn test_complex_nested_and_overriding_rules() -> Result<()> {
    test_logging(LevelFilter::Trace);
    let root = setup_test_directory(
        &[
            "logs/a.log",
            "logs/b.log",
            "src/main.rs",
            "src/lib.rs",
            "src/tests/test1.rs",
            "src/tests/data/test.dat",
            "docs/index.md",
        ],
        &[
            (".gitignore", "logs/\n*.rs\n!/src/lib.rs"),
            ("src/.gitignore", "!*.rs\n/tests/"),
            ("src/tests/.gitignore", "*.dat"),
        ],
    )?;
    let result = run_walk(&root, ".gitignore")?;
    let expected = get_git_expected_paths(&root)?;
    assert_eq_and_print!(result, expected);
    Ok(())
}

#[test]
fn test_include_directories_in_result() -> Result<()> {
    test_logging(LevelFilter::Trace);
    let root = setup_test_directory(&["dir/a.txt", "b.txt"], &[(".gitignore", "b.txt")])?;
    let result = run_walk(&root, ".gitignore")?;
    let expected = get_git_expected_paths(&root)?;
    assert_eq_and_print!(result, expected);
    Ok(())
}

#[test]
fn test_whitelisting_files_in_directory() -> Result<()> {
    test_logging(LevelFilter::Trace);
    let root = setup_test_directory(
        &[
            "a.log",
            "b.txt",
            "important/d.log",
            "important/e.txt",
            "trace.c",
        ],
        &[(".gitignore", "*.log\n!important/*.log\ntrace.*")],
    )?;
    let result = run_walk(&root, ".gitignore")?;
    let expected = get_git_expected_paths(&root)?;
    assert_eq_and_print!(result, expected);
    Ok(())
}

#[test]
fn test_complex_whitelisting() -> Result<()> {
    test_logging(LevelFilter::Trace);
    let root = setup_test_directory(
        &[
            "test1/a.txt",
            "test1/b.bin",
            "test1/c/c.txt",
            "test2/a.txt",
            "test2/b.bin",
            "test2/c/c.txt",
        ],
        &[(".gitignore", "*\n!*/\n!*.txt\n/test1/**")],
    )?;
    let result = run_walk(&root, ".gitignore")?;
    let expected = get_git_expected_paths(&root)?;
    assert_eq_and_print!(result, expected);
    Ok(())
}

#[test]
fn test_ignore_all_then_whitelist_dir() -> Result<()> {
    test_logging(LevelFilter::Trace);
    let root = setup_test_directory(
        &["ignore.txt", "libfoo/__init__.py", "libfoo/bar/baz.py"],
        &[(".gitignore", "*\n!/libfoo/**")],
    )?;
    let result = run_walk(&root, ".gitignore")?;
    let expected = get_git_expected_paths(&root)?;
    assert_eq_and_print!(result, expected);
    Ok(())
}

#[test]
fn test_very_complex_nested_gitignore_rules() -> Result<()> {
    test_logging(LevelFilter::Trace);
    let root = setup_test_directory(
        &[
            "package.json",
            "main.log",
            "app/server.js",
            "app/server.log",
            "app/client/main.js",
            "app/client/style.css",
            "app/client/bundle.js",
            "app/db/data.sql",
            "app/db/schema.log",
        ],
        &[
            (".gitignore", "*.log\n/node_modules/"),
            ("app/.gitignore", "!/app/server.log\n!/app/db/"),
            ("app/client/.gitignore", "*\n!bundle.js"),
        ],
    )?;
    let result = run_walk(&root, ".gitignore")?;
    let expected = get_git_expected_paths(&root)?;
    assert_eq_and_print!(result, expected);
    Ok(())
}

#[test]
fn some_test() -> Result<()> {
    test_logging(LevelFilter::Trace);
    let root =
        setup_test_directory(&["ignore.txt", ".git/a.txt"], &[(".gitignore", ".git")])?;
    let result = run_walk(&root, ".gitignore")?;
    let expected = get_git_expected_paths(&root)?;
    assert_eq_and_print!(result, expected);
    Ok(())
}

#[test]
fn test_double_star_in_middle() -> Result<()> {
    test_logging(LevelFilter::Trace);
    let root = setup_test_directory(
        &["a/b/c/z.txt", "a/z.txt", "x/y.txt"],
        &[(".gitignore", "a/**/z.txt")],
    )?;
    let result = run_walk(&root, ".gitignore")?;
    let expected = get_git_expected_paths(&root)?;
    assert_eq_and_print!(result, expected);
    Ok(())
}

#[test]
fn test_trailing_spaces_in_pattern() -> Result<()> {
    test_logging(LevelFilter::Trace);
    let root = setup_test_directory(&["foo", "bar"], &[(".gitignore", "foo  ")])?;
    let result = run_walk(&root, ".gitignore")?;
    let expected = get_git_expected_paths(&root)?;
    assert_eq_and_print!(result, expected);
    Ok(())
}

#[test]
fn test_trailing_escaped_spaces_in_pattern() -> Result<()> {
    test_logging(LevelFilter::Trace);
    // A pattern "foo\ " in .gitignore will be treated as "foo " (with one space).
    // This won't match the file "foo". So "foo" should NOT be ignored.
    let root = setup_test_directory(&["foo", "bar"], &[(".gitignore", "foo\\ ")])?;
    let result = run_walk(&root, ".gitignore")?;
    let expected = get_git_expected_paths(&root)?;
    assert_eq_and_print!(result, expected);
    Ok(())
}

#[test]
fn test_reinclude_file_in_ignored_dir_tree() -> Result<()> {
    test_logging(LevelFilter::Trace);
    let root = setup_test_directory(
        &["build/app/app.js", "build/lib/lib.js", "build/test.txt"],
        &[(
            ".gitignore",
            "/build/*\n!/build/app\n/build/app/*\n!/build/app/app.js",
        )],
    )?;
    let result = run_walk(&root, ".gitignore")?;
    let expected = get_git_expected_paths(&root)?;
    assert_eq_and_print!(result, expected);
    Ok(())
}

#[test]
fn test_question_mark_wildcard() -> Result<()> {
    test_logging(LevelFilter::Trace);
    let root = setup_test_directory(
        &["a.txt", "ab.txt", "abc.txt", "b.txt"],
        &[(".gitignore", "a?.txt")],
    )?;
    let result = run_walk(&root, ".gitignore")?;
    let expected = get_git_expected_paths(&root)?;
    assert_eq_and_print!(result, expected);
    Ok(())
}

#[test]
fn test_complex_nested_whitelisting() -> Result<()> {
    test_logging(LevelFilter::Trace);
    let root = setup_test_directory(
        &[
            "a/b/c.js",
            "a/b/c.txt",
            "a/d.js",
            "a/d.txt",
            "e.js",
            "e.txt",
        ],
        &[
            (".gitignore", "*.js\n!/a/b/c.js"),
            ("a/.gitignore", "*.txt\n!d.txt"),
        ],
    )?;
    let result = run_walk(&root, ".gitignore")?;
    let expected = get_git_expected_paths(&root)?;
    assert_eq_and_print!(result, expected);
    Ok(())
}

#[test]
fn test_dir_names_with_glob_chars() -> Result<()> {
    test_logging(LevelFilter::Trace);
    let root = setup_test_directory(
        &["a[b]/c.txt", "a?b/d.txt", "a*b/e.txt"],
        &[(".gitignore", "a*b/*")],
    )?;
    let result = run_walk(&root, ".gitignore")?;
    let expected = get_git_expected_paths(&root)?;
    assert_eq_and_print!(result, expected);
    Ok(())
}

#[test]
fn test_unignoring_gitignore_itself() -> Result<()> {
    test_logging(LevelFilter::Trace);
    let root = setup_test_directory(&["a.txt", "b.txt"], &[(".gitignore", "*\n!.gitignore")])?;
    let result = run_walk(&root, ".gitignore")?;
    let expected = get_git_expected_paths(&root)?;
    assert_eq_and_print!(result, expected);
    Ok(())
}