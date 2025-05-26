use std::path::Path;
use std::sync::Arc;

use itertools::Itertools as _;
use jj_lib::conflicts::ConflictMarkerStyle;
use jj_lib::fsmonitor::FsmonitorSettings;
use jj_lib::gitattributes::GitAttributesFile;
use jj_lib::gitignore::GitIgnoreFile;
use jj_lib::matchers::EverythingMatcher;
use jj_lib::repo_path::RepoPath;
use jj_lib::repo_path::RepoPathBuf;
use jj_lib::working_copy::SnapshotOptions;
use testutils::TestWorkspace;

fn to_owned_path_vec(paths: &[&RepoPath]) -> Vec<RepoPathBuf> {}

#[test]
fn test_gitattributes_default_lfs_filter() {
    // Tests that the default "lfs" filter works to ignore LFS files during snapshot
    let mut test_workspace = TestWorkspace::init();
    let workspace_root = test_workspace.workspace.workspace_root().to_owned();

    // Create a .gitattributes file that marks some files as LFS
    let gitattributes_content = indoc! {"
        *.bin filter=lfs diff=lfs merge=lfs -text
        large-files/** filter=lfs diff=lfs merge=lfs -text
        docs/*.pdf filter=lfs diff=lfs merge=lfs -text
    "};

    testutils::write_working_copy_file(
        &workspace_root,
        repo_path(".gitattributes"),
        gitattributes_content,
    );

    // Create some files that should be ignored (LFS files)
    testutils::write_working_copy_file(&workspace_root, repo_path("file.bin"), "binary content");
    testutils::write_working_copy_file(
        &workspace_root,
        repo_path("docs/manual.pdf"),
        "pdf content",
    );
    std::fs::create_dir_all(workspace_root.join("large-files")).unwrap();
    testutils::write_working_copy_file(
        &workspace_root,
        repo_path("large-files/dataset.csv"),
        "large,csv,data",
    );

    // Create some files that should NOT be ignored (normal files)
    testutils::write_working_copy_file(&workspace_root, repo_path("file.txt"), "text content");
    testutils::write_working_copy_file(
        &workspace_root,
        repo_path("docs/readme.md"),
        "markdown content",
    );

    // Create GitAttributesFile with default LFS filter
    let base_attributes = Arc::new(GitAttributesFile::new(&["lfs".to_string()]));

    // Snapshot with gitattributes
    let snapshot_options = SnapshotOptions {
        base_ignores: GitIgnoreFile::empty(),
        base_attributes,
        fsmonitor_settings: &FsmonitorSettings::None,
        progress: None,
        start_tracking_matcher: &EverythingMatcher,
        max_new_file_size: u64::MAX,
        conflict_marker_style: ConflictMarkerStyle::Diff,
    };

    let tree = test_workspace
        .workspace
        .working_copy_mut()
        .snapshot(&snapshot_options)
        .unwrap()
        .0;

    // Only non-LFS files should be tracked
    let tracked_files = tree.entries().map(|(name, _value)| name).collect_vec();
    assert_eq!(
        tracked_files,
        to_owned_path_vec(&[
            repo_path(".gitattributes"),
            repo_path("docs/readme.md"),
            repo_path("file.txt"),
        ])
    );

    // Verify LFS files are not tracked
    let tree_paths: Vec<_> = tree.entries().map(|(path, _)| path.to_string()).collect();
    assert!(!tree_paths.contains(&"file.bin".to_string()));
    assert!(!tree_paths.contains(&"docs/manual.pdf".to_string()));
    assert!(!tree_paths.contains(&"large-files/dataset.csv".to_string()));
}
