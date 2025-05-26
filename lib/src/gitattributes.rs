// Copyright 2024 The Jujutsu Authors
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
// https://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

#![allow(missing_docs)]

use gix::attrs as gix_attrs;
use gix::glob as gix_glob;
use gix::path as gix_path;
use std::borrow::Cow;
use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum GitAttributesError {
    #[error("Failed to read attributes patterns from file {path}")]
    ReadFile { path: PathBuf, source: io::Error },
}

/// Models the effective contents of multiple .gitattributes files.
#[derive(Debug)]
pub struct GitAttributesFile {
    search: gix_attrs::Search,
    collection: gix_attrs::search::MetadataCollection,
    ignore_filters: Vec<String>,
}

impl GitAttributesFile {
    pub fn new(ignore_filters: &[String]) -> Self {
        let base_attributes = Self::default();

        GitAttributesFile {
            ignore_filters: ignore_filters.to_vec(),
            ..base_attributes
        }
    }

    pub fn test_matching(&self) {
        println!("Testing gitattributes matching:");
        let test_files = vec!["test.png", "image.jpg", "doc.txt", "assets/image.png"];

        for file in test_files {
            let matches = self.matches(file);
            println!(" {} -> {}", file, matches);
        }
    }

    pub fn debug_print_all_patterns(&self) {
        dbg!(&self.search);
    }

    pub fn debug_match_for_path(&self, path: &str) {
        let (candidate, is_dir) = if let Some(p) = path.strip_suffix('/') {
            (p, true)
        } else {
            (path, false)
        };
        let case = gix_glob::pattern::Case::Sensitive;

        println!("--- matching `{}` (is_dir={}) ---", candidate, is_dir);
        dbg!(&self.search);

        let mut out = gix_attrs::search::Outcome::default();
        out.initialize_with_selection(&self.collection, ["filter"]);
        self.search
            .pattern_matching_relative_path(candidate.into(), case, Some(is_dir), &mut out);
        dbg!(&out);
    }

    pub fn chain(
        self: &Arc<GitAttributesFile>,
        prefix: PathBuf,
        input: &[u8],
    ) -> Result<Arc<GitAttributesFile>, GitAttributesError> {
        let mut source_file = prefix.clone();
        source_file.push(".gitattributes");

        let mut search = self.search.clone();
        let mut collection = self.collection.clone();
        let ignore_filters = self.ignore_filters.clone();

        let prefix_for_patterns = if prefix.as_os_str().is_empty() {
            search.add_patterns_buffer(input, source_file, None, &mut collection, true);
        } else {
            search.add_patterns_buffer(input, source_file, Some(&prefix), &mut collection, true);
        };

        Ok(Arc::new(GitAttributesFile {
            search,
            collection,
            ignore_filters,
        }))
    }

    /// Concatenates new `.gitattributes` file.
    ///
    /// The `prefix` should be a slash-separated path relative to the workspace
    /// root.
    pub fn chain_with_file(
        self: &Arc<GitAttributesFile>,
        prefix: &str,
        file: PathBuf,
    ) -> Result<Arc<GitAttributesFile>, GitAttributesError> {
        if file.is_file() {
            let buf = std::fs::read(&file).map_err(|err| GitAttributesError::ReadFile {
                path: file.clone(),
                source: err,
            })?;
            let repo_prefix = PathBuf::from(prefix);
            self.chain(repo_prefix, &buf)
        } else {
            Ok(self.clone())
        }
    }

    pub fn matches(&self, path: &str) -> bool {
        // If path ends with slash, consider it as a directory.
        let (path, is_dir) = match path.strip_suffix('/') {
            Some(path) => (path, true),
            None => (path, false),
        };

        let mut out = gix_attrs::search::Outcome::default();
        out.initialize_with_selection(&self.collection, ["filter"]);
        self.search.pattern_matching_relative_path(
            path.into(),
            gix_glob::pattern::Case::Sensitive,
            Some(is_dir),
            &mut out,
        );

        let matched = out
            .iter_selected()
            .filter_map(|attr| {
                if let gix_attrs::StateRef::Value(value_ref) = attr.assignment.state {
                    if let Some(source_path) = &attr.location.source {
                        if let Some(source_str) = source_path.to_str() {
                            if source_str.ends_with("/.gitattributes")
                                && source_str != ".gitattributes"
                            {
                                if let Some(subdir) = source_str.strip_suffix("/.gitattributes") {
                                    let required_prefix = format!("{}/", subdir);
                                    let path_matches = path.starts_with(&required_prefix);
                                    if !path_matches {
                                        return None;
                                    }
                                }
                            }
                        }
                    }
                    Some(value_ref.as_bstr())
                } else {
                    None
                }
            })
            .any(|value| self.ignore_filters.iter().any(|state| value == state));
        matched
    }
}

impl Default for GitAttributesFile {
    fn default() -> Self {
        let files = [
            gix_attrs::Source::GitInstallation,
            gix_attrs::Source::System,
            gix_attrs::Source::Git,
            gix_attrs::Source::Local,
        ]
        .iter()
        .filter_map(|source| {
            source
                .storage_location(&mut gix_path::env::var)
                .and_then(|p| p.is_file().then_some(p))
                .map(Cow::into_owned)
        });

        let mut buf = Vec::new();
        let mut collection = gix_attrs::search::MetadataCollection::default();
        let search = gix_attrs::Search::new_globals(files, &mut buf, &mut collection)
            .unwrap_or_else(|_| gix_attrs::Search::default());
        let ignore_filters = Vec::new();

        GitAttributesFile {
            search,
            collection,
            ignore_filters,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn matches(input: &[u8], path: &str) -> bool {
        let file = Arc::new(GitAttributesFile::new(&["lfs".to_string()]))
            .chain(PathBuf::new(), input)
            .unwrap();
        file.matches(path)
    }

    #[test]
    fn test_gitattributes_empty_file() {
        let file = GitAttributesFile::new(&["lfs".to_string()]);
        assert!(!file.matches("foo"));
    }

    #[test]
    fn test_gitattributes_simple_match() {
        assert!(matches(b"*.bin filter=lfs\n", "file.bin"));
        assert!(!matches(b"*.bin filter=lfs\n", "file.txt"));
        assert!(!matches(b"*.bin filter=other\n", "file.bin"));
    }

    #[test]
    fn test_gitattributes_directory_match() {
        assert!(matches(b"dir/** filter=lfs\n", "dir/file.txt"));
        assert!(matches(b"dir/* filter=lfs\n", "dir/file.txt"));
        assert!(matches(b"dir/ filter=lfs\n", "dir/"));

        assert!(!matches(b"dir/** filter=lfs\n", "other/file.txt"));
        assert!(!matches(b"dir/* filter=lfs\n", "other/file.txt"));

        assert!(!matches(b"dir/ filter=lfs\n", "dir"));
    }

    #[test]
    fn test_gitattributes_path_match() {
        assert!(matches(
            b"path/to/file.bin filter=lfs\n",
            "path/to/file.bin"
        ));
        assert!(!matches(b"path/to/file.bin filter=lfs\n", "path/file.bin"));
    }

    #[test]
    fn test_gitattributes_wildcard_match() {
        assert!(matches(b"*.bin filter=lfs\n", "file.bin"));
        assert!(matches(b"file.* filter=lfs\n", "file.bin"));
        assert!(matches(b"**/file.bin filter=lfs\n", "path/to/file.bin"));
    }

    #[test]
    fn test_gitattributes_multiple_attributes() {
        let input = b"*.bin filter=lfs diff=binary\n";
        assert!(matches(input, "file.bin"));
        assert!(!matches(b"*.bin diff=binary\n", "file.bin")); // Only testing filter=lfs
    }

    #[test]
    fn test_gitattributes_chained_files() {
        let base = Arc::new(GitAttributesFile::new(&[
            "lfs".to_string(),
            "text".to_string(),
        ]));
        let with_first = base.chain(PathBuf::new(), b"*.bin filter=lfs\n").unwrap();
        let with_second = with_first
            .chain(PathBuf::from("subdir"), b"*.txt filter=text\n")
            .unwrap();

        assert!(with_second.matches("file.bin"));
        assert!(with_second.matches("subdir/file.txt"));
        assert!(!with_second.matches("file.txt")); // Not in subdir
    }

    #[test]
    fn test_gitattributes_negated_pattern() {
        assert!(matches(b"*.bin filter=lfs\n", "file.bin"));
        assert!(matches(b"*.bin filter=lfs\n", "important.bin"));
        assert!(matches(
            b"*.bin filter=lfs\nimportant.bin -filter",
            "file.bin"
        ));
        assert!(!matches(
            b"*.bin filter=lfs\nimportant.bin -filter",
            "important.bin"
        ));
    }

    #[test]
    fn test_gitattributes_multiple_filters() {
        // Create a GitAttributesFile with both "lfs" and "git-crypt" as ignore filters
        let file = Arc::new(GitAttributesFile::new(&[
            "lfs".to_string(),
            "git-crypt".to_string(),
        ]));

        // Test with lfs filter
        let with_lfs = file.chain(PathBuf::new(), b"*.bin filter=lfs\n").unwrap();
        assert!(with_lfs.matches("file.bin"));

        // Test with git-crypt filter
        let with_git_crypt = file
            .chain(PathBuf::new(), b"*.secret filter=git-crypt\n")
            .unwrap();
        assert!(with_git_crypt.matches("credentials.secret"));

        // Test with both filters in the same file
        let with_both = file
            .chain(
                PathBuf::new(),
                b"*.bin filter=lfs\n*.secret filter=git-crypt\n",
            )
            .unwrap();
        assert!(with_both.matches("file.bin"));
        assert!(with_both.matches("credentials.secret"));
        assert!(!with_both.matches("normal.txt"));

        // Test that other filters don't match
        let with_other = file.chain(PathBuf::new(), b"*.txt filter=other\n").unwrap();
        assert!(!with_other.matches("file.txt"));
    }
}

#[cfg(test)]
mod comprehensive_tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_git_crypt_patterns() {
        let file = Arc::new(GitAttributesFile::new(&["git-crypt".to_string()]));

        let patterns =
            b"secrets/* filter=git-crypt\n*.key filter=git-crypt\ndatabase.conf filter=git-crypt\n";
        let with_crypt = file.chain(PathBuf::new(), patterns).unwrap();

        assert!(with_crypt.matches("secrets/api.key"));
        assert!(with_crypt.matches("secrets/passwords.txt"));
        assert!(with_crypt.matches("config.key"));
        assert!(with_crypt.matches("database.conf"));

        assert!(!with_crypt.matches("public/readme.txt"));
        assert!(!with_crypt.matches("src/main.rs"));
        assert!(!with_crypt.matches("config.json"));
    }

    #[test]
    fn test_multiple_filters_interaction() {
        let file = Arc::new(GitAttributesFile::new(&[
            "lfs".to_string(),
            "git-crypt".to_string(),
            "custom".to_string(),
        ]));

        let patterns = b"*.large filter=lfs\n*.secret filter=git-crypt\n*.special filter=custom\n*.txt filter=lfs\n";
        let multi_filter = file.chain(PathBuf::new(), patterns).unwrap();

        // Each filter type should work
        assert!(multi_filter.matches("file.large")); // LFS
        assert!(multi_filter.matches("file.secret")); // git-crypt
        assert!(multi_filter.matches("file.txt")); // LFS (should match because lfs is in ignore_filters)

        // Custom filter should work too
        assert!(multi_filter.matches("file.special")); // custom

        // Non-matching files
        assert!(!multi_filter.matches("file.normal"));
    }

    #[test]
    fn test_complex_directory_structures() {
        let file = Arc::new(GitAttributesFile::new(&["lfs".to_string()]));

        // Root gitattributes
        let root_patterns = b"*.bin filter=lfs\n";
        let with_root = file.chain(PathBuf::new(), root_patterns).unwrap();

        // Subdirectory gitattributes - deeper nesting
        let sub1_patterns = b"*.tmp filter=lfs\n";
        let with_sub1 = with_root
            .chain(PathBuf::from("project/assets"), sub1_patterns)
            .unwrap();

        // Even deeper subdirectory
        let sub2_patterns = b"*.cache filter=lfs\n";
        let with_sub2 = with_sub1
            .chain(PathBuf::from("project/assets/images"), sub2_patterns)
            .unwrap();

        // Test inheritance and scoping
        assert!(with_sub2.matches("file.bin")); // Root pattern - global
        assert!(with_sub2.matches("other/file.bin")); // Root pattern - global
        assert!(with_sub2.matches("project/assets/temp.tmp")); // Sub1 pattern - scoped
        assert!(with_sub2.matches("project/assets/images/thumb.cache")); // Sub2 pattern - scoped

        // Test that subdirectory patterns don't leak
        assert!(!with_sub2.matches("temp.tmp")); // Sub1 pattern outside scope
        assert!(!with_sub2.matches("project/thumb.cache")); // Sub2 pattern outside scope
        assert!(!with_sub2.matches("project/assets/thumb.cache")); // Sub2 pattern in wrong scope
    }

    #[test]
    fn test_gitattributes_with_comments_and_whitespace() {
        let file = Arc::new(GitAttributesFile::new(&["lfs".to_string()]));

        // Real-world gitattributes with comments and varied formatting
        let patterns = b"# LFS tracking\n*.psd filter=lfs\n\n# Binary files\n*.dll filter=lfs\n\n# Large text files\n*.log filter=lfs  \n";
        let with_comments = file.chain(PathBuf::new(), patterns).unwrap();

        assert!(with_comments.matches("design.psd"));
        assert!(with_comments.matches("library.dll"));
        assert!(with_comments.matches("debug.log"));
        assert!(!with_comments.matches("readme.txt"));
    }

    #[test]
    fn test_pattern_precedence_and_scoping() {
        let file = Arc::new(GitAttributesFile::new(&["lfs".to_string()]));

        // Root level patterns
        let root_patterns = b"*.txt filter=lfs\n*.md filter=lfs\n";
        let with_root = file.chain(PathBuf::new(), root_patterns).unwrap();

        // Subdirectory adds additional patterns
        let sub_patterns = b"*.log filter=lfs\n";
        let with_subdirectory = with_root
            .chain(PathBuf::from("logs"), sub_patterns)
            .unwrap();

        // Test that root patterns still work everywhere
        assert!(with_subdirectory.matches("readme.txt"));
        assert!(with_subdirectory.matches("docs/guide.txt"));
        assert!(with_subdirectory.matches("changelog.md"));

        // Test that subdirectory patterns only work in their scope
        assert!(with_subdirectory.matches("logs/debug.log")); // In subdirectory
        assert!(!with_subdirectory.matches("debug.log")); // Not in subdirectory
        assert!(!with_subdirectory.matches("src/debug.log")); // Not in subdirectory

        // Test inheritance - files in subdirectory get both root and local patterns
        assert!(with_subdirectory.matches("logs/readme.txt")); // Root pattern applies in subdirectory
        assert!(with_subdirectory.matches("logs/debug.log")); // Local pattern applies in subdirectory
    }

    #[test]
    fn test_pattern_specificity() {
        let file = Arc::new(GitAttributesFile::new(&[
            "lfs".to_string(),
            "other".to_string(),
        ]));

        // Test that more specific patterns can be added
        let general_patterns = b"*.txt filter=lfs\n";
        let with_general = file.chain(PathBuf::new(), general_patterns).unwrap();

        // Add more specific patterns in a subdirectory
        let specific_patterns = b"special.txt filter=other\n";
        let with_specific = with_general
            .chain(PathBuf::from("special"), specific_patterns)
            .unwrap();

        // General pattern still works at root
        assert!(with_specific.matches("readme.txt"));

        // General pattern works in subdirectory for non-specific files
        assert!(with_specific.matches("special/readme.txt"));

        // Specific pattern works for the specific file in subdirectory
        assert!(with_specific.matches("special/special.txt"));

        // Specific pattern doesn't affect root level
        assert!(with_specific.matches("special.txt")); // This should match the general lfs pattern, not other
    }
}
