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

use std::path::Path;
use std::path::PathBuf;

use indoc::formatdoc;
use test_case::test_case;
use testutils::git;

use crate::common::to_toml_value;
use crate::common::CommandOutput;
use crate::common::TestEnvironment;

fn init_git_repo(git_repo_path: &Path, bare: bool) -> gix::Repository {
    let git_repo = if bare {
        git::init_bare(git_repo_path)
    } else {
        git::init(git_repo_path)
    };

    let git::CommitResult { commit_id, .. } = git::add_commit(
        &git_repo,
        "refs/heads/my-bookmark",
        "some-file",
        b"some content",
        "My commit message",
        &[],
    );
    git::set_head_to_id(&git_repo, commit_id);
    git_repo
}

#[must_use]
fn get_bookmark_output(test_env: &TestEnvironment, repo_path: &Path) -> CommandOutput {
    test_env.run_jj_in(repo_path, ["bookmark", "list", "--all-remotes"])
}

#[must_use]
fn get_log_output(test_env: &TestEnvironment, workspace_root: &Path) -> CommandOutput {
    let template = r#"
    separate(" ",
      commit_id.short(),
      bookmarks,
      if(git_head, "git_head()"),
      description,
    )"#;
    test_env.run_jj_in(workspace_root, ["log", "-T", template, "-r=all()"])
}

fn read_git_target(workspace_root: &Path) -> String {
    let mut path = workspace_root.to_path_buf();
    path.extend([".jj", "repo", "store", "git_target"]);
    std::fs::read_to_string(path).unwrap()
}

#[test]
fn test_git_init_internal() {
    let test_env = TestEnvironment::default();
    let output = test_env.run_jj_in(".", ["git", "init", "repo"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Initialized repo in "repo"
    [EOF]
    "#);

    let workspace_root = test_env.env_root().join("repo");
    let jj_path = workspace_root.join(".jj");
    let repo_path = jj_path.join("repo");
    let store_path = repo_path.join("store");
    assert!(workspace_root.is_dir());
    assert!(jj_path.is_dir());
    assert!(jj_path.join("working_copy").is_dir());
    assert!(repo_path.is_dir());
    assert!(store_path.is_dir());
    assert!(store_path.join("git").is_dir());
    assert_eq!(read_git_target(&workspace_root), "git");
}

#[test]
fn test_git_init_internal_ignore_working_copy() {
    let test_env = TestEnvironment::default();
    let workspace_root = test_env.env_root().join("repo");
    std::fs::create_dir(&workspace_root).unwrap();
    std::fs::write(workspace_root.join("file1"), "").unwrap();

    let output = test_env.run_jj_in(&workspace_root, ["git", "init", "--ignore-working-copy"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: --ignore-working-copy is not respected
    [EOF]
    [exit status: 2]
    ");
}

#[test]
fn test_git_init_internal_at_operation() {
    let test_env = TestEnvironment::default();
    let workspace_root = test_env.env_root().join("repo");
    std::fs::create_dir(&workspace_root).unwrap();

    let output = test_env.run_jj_in(&workspace_root, ["git", "init", "--at-op=@-"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: --at-op is not respected
    [EOF]
    [exit status: 2]
    ");
}

#[test_case(false; "full")]
#[test_case(true; "bare")]
fn test_git_init_external(bare: bool) {
    let test_env = TestEnvironment::default();
    let git_repo_path = test_env.env_root().join("git-repo");
    init_git_repo(&git_repo_path, bare);

    let output = test_env.run_jj_in(
        ".",
        [
            "git",
            "init",
            "repo",
            "--git-repo",
            git_repo_path.to_str().unwrap(),
        ],
    );
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Done importing changes from the underlying Git repo.
    Working copy now at: sqpuoqvx 0bd37cef (empty) (no description set)
    Parent commit      : nntyzxmz e80a42cc my-bookmark | My commit message
    Added 1 files, modified 0 files, removed 0 files
    Initialized repo in "repo"
    [EOF]
    "#);
    }

    let workspace_root = test_env.env_root().join("repo");
    let jj_path = workspace_root.join(".jj");
    let repo_path = jj_path.join("repo");
    let store_path = repo_path.join("store");
    assert!(workspace_root.is_dir());
    assert!(jj_path.is_dir());
    assert!(jj_path.join("working_copy").is_dir());
    assert!(repo_path.is_dir());
    assert!(store_path.is_dir());
    let unix_git_target_file_contents = read_git_target(&workspace_root).replace('\\', "/");
    if bare {
        assert!(unix_git_target_file_contents.ends_with("/git-repo"));
    } else {
        assert!(unix_git_target_file_contents.ends_with("/git-repo/.git"));
    }

    // Check that the Git repo's HEAD got checked out
    insta::allow_duplicates! {
        insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r"
        @  0bd37cef2051
        ○  e80a42cccd06 my-bookmark git_head() My commit message
        ◆  000000000000
        [EOF]
        ");
    }
}

#[test_case(false; "full")]
#[test_case(true; "bare")]
fn test_git_init_external_import_trunk(bare: bool) {
    let test_env = TestEnvironment::default();
    let git_repo_path = test_env.env_root().join("git-repo");
    let git_repo = init_git_repo(&git_repo_path, bare);

    // Add remote bookmark "trunk" for remote "origin", and set it as "origin/HEAD"
    let oid = git_repo
        .find_reference("refs/heads/my-bookmark")
        .unwrap()
        .id();

    git_repo
        .reference(
            "refs/remotes/origin/trunk",
            oid.detach(),
            gix::refs::transaction::PreviousValue::MustNotExist,
            "create remote ref",
        )
        .unwrap();

    git::set_symbolic_reference(
        &git_repo,
        "refs/remotes/origin/HEAD",
        "refs/remotes/origin/trunk",
    );

    let output = test_env.run_jj_in(
        ".",
        [
            "git",
            "init",
            "repo",
            "--git-repo",
            git_repo_path.to_str().unwrap(),
        ],
    );
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Done importing changes from the underlying Git repo.
    Setting the revset alias `trunk()` to `trunk@origin`
    Working copy now at: sqpuoqvx 0bd37cef (empty) (no description set)
    Parent commit      : nntyzxmz e80a42cc my-bookmark trunk@origin | My commit message
    Added 1 files, modified 0 files, removed 0 files
    Initialized repo in "repo"
    [EOF]
    "#);
    }

    // "trunk()" alias should be set to remote "origin"'s default bookmark "trunk"
    let output = test_env.run_jj_in(
        &test_env.env_root().join("repo"),
        ["config", "list", "--repo", "revset-aliases.\"trunk()\""],
    );
    insta::allow_duplicates! {
        insta::assert_snapshot!(output, @r#"
        revset-aliases."trunk()" = "trunk@origin"
        [EOF]
        "#);
    }
}

#[test]
fn test_git_init_external_ignore_working_copy() {
    let test_env = TestEnvironment::default();
    let git_repo_path = test_env.env_root().join("git-repo");
    init_git_repo(&git_repo_path, false);
    let workspace_root = test_env.env_root().join("repo");
    std::fs::create_dir(&workspace_root).unwrap();
    std::fs::write(workspace_root.join("file1"), "").unwrap();

    // No snapshot should be taken
    let output = test_env.run_jj_in(
        &workspace_root,
        [
            "git",
            "init",
            "--ignore-working-copy",
            "--git-repo",
            git_repo_path.to_str().unwrap(),
        ],
    );
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: --ignore-working-copy is not respected
    [EOF]
    [exit status: 2]
    ");
}

#[test]
fn test_git_init_external_at_operation() {
    let test_env = TestEnvironment::default();
    let git_repo_path = test_env.env_root().join("git-repo");
    init_git_repo(&git_repo_path, false);
    let workspace_root = test_env.env_root().join("repo");
    std::fs::create_dir(&workspace_root).unwrap();

    let output = test_env.run_jj_in(
        &workspace_root,
        [
            "git",
            "init",
            "--at-op=@-",
            "--git-repo",
            git_repo_path.to_str().unwrap(),
        ],
    );
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: --at-op is not respected
    [EOF]
    [exit status: 2]
    ");
}

#[test]
fn test_git_init_external_non_existent_directory() {
    let test_env = TestEnvironment::default();
    let output = test_env.run_jj_in(".", ["git", "init", "repo", "--git-repo", "non-existent"]);
    insta::assert_snapshot!(output.strip_stderr_last_line(), @r"
    ------- stderr -------
    Error: Failed to access the repository
    Caused by:
    1: Cannot access $TEST_ENV/non-existent
    [EOF]
    [exit status: 1]
    ");
}

#[test]
fn test_git_init_external_non_existent_git_directory() {
    let test_env = TestEnvironment::default();
    let workspace_root = test_env.env_root().join("repo");
    let output = test_env.run_jj_in(".", ["git", "init", "repo", "--git-repo", "repo"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Error: Failed to access the repository
    Caused by:
    1: Failed to open git repository
    2: "$TEST_ENV/repo" does not appear to be a git repository
    3: Missing HEAD at '.git/HEAD'
    [EOF]
    [exit status: 1]
    "#);
    let jj_path = workspace_root.join(".jj");
    assert!(!jj_path.exists());
}

#[test]
fn test_git_init_colocated_via_git_repo_path() {
    let test_env = TestEnvironment::default();
    let workspace_root = test_env.env_root().join("repo");
    init_git_repo(&workspace_root, false);
    let output = test_env.run_jj_in(&workspace_root, ["git", "init", "--git-repo", "."]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Done importing changes from the underlying Git repo.
    Initialized repo in "."
    [EOF]
    "#);

    let jj_path = workspace_root.join(".jj");
    let repo_path = jj_path.join("repo");
    let store_path = repo_path.join("store");
    assert!(workspace_root.is_dir());
    assert!(jj_path.is_dir());
    assert!(jj_path.join("working_copy").is_dir());
    assert!(repo_path.is_dir());
    assert!(store_path.is_dir());
    assert!(read_git_target(&workspace_root)
        .replace('\\', "/")
        .ends_with("../../../.git"));

    // Check that the Git repo's HEAD got checked out
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r"
    @  5f169ecc57b8
    ○  e80a42cccd06 my-bookmark git_head() My commit message
    ◆  000000000000
    [EOF]
    ");

    // Check that the Git repo's HEAD moves
    test_env.run_jj_in(&workspace_root, ["new"]).success();
    insta::assert_snapshot!(get_log_output(&test_env, &workspace_root), @r"
    @  62eda98b5eb4
    ○  5f169ecc57b8 git_head()
    ○  e80a42cccd06 my-bookmark My commit message
    ◆  000000000000
    [EOF]
    ");
}

#[test]
fn test_git_init_colocated_via_git_repo_path_gitlink() {
    let test_env = TestEnvironment::default();
    // <workspace_root>/.git -> <git_repo_path>
    let git_repo_path = test_env.env_root().join("git-repo");
    let workspace_root = test_env.env_root().join("repo");
    let git_repo = init_git_repo(&git_repo_path, false);
    std::fs::create_dir(&workspace_root).unwrap();
    git::create_gitlink(&workspace_root, git_repo.path());

    assert!(workspace_root.join(".git").is_file());
    let output = test_env.run_jj_in(&workspace_root, ["git", "init", "--git-repo", "."]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Done importing changes from the underlying Git repo.
    Initialized repo in "."
    [EOF]
    "#);
    insta::assert_snapshot!(read_git_target(&workspace_root), @"../../../.git");

    // Check that the Git repo's HEAD got checked out
    insta::assert_snapshot!(get_log_output(&test_env, &workspace_root), @r"
    @  5f169ecc57b8
    ○  e80a42cccd06 my-bookmark git_head() My commit message
    ◆  000000000000
    [EOF]
    ");

    // Check that the Git repo's HEAD moves
    test_env.run_jj_in(&workspace_root, ["new"]).success();
    insta::assert_snapshot!(get_log_output(&test_env, &workspace_root), @r"
    @  62eda98b5eb4
    ○  5f169ecc57b8 git_head()
    ○  e80a42cccd06 my-bookmark My commit message
    ◆  000000000000
    [EOF]
    ");
}

#[cfg(unix)]
#[test]
fn test_git_init_colocated_via_git_repo_path_symlink_directory() {
    let test_env = TestEnvironment::default();
    // <workspace_root>/.git -> <git_repo_path>
    let git_repo_path = test_env.env_root().join("git-repo");
    let workspace_root = test_env.env_root().join("repo");
    init_git_repo(&git_repo_path, false);
    std::fs::create_dir(&workspace_root).unwrap();
    std::os::unix::fs::symlink(git_repo_path.join(".git"), workspace_root.join(".git")).unwrap();
    let output = test_env.run_jj_in(&workspace_root, ["git", "init", "--git-repo", "."]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Done importing changes from the underlying Git repo.
    Initialized repo in "."
    [EOF]
    "#);
    insta::assert_snapshot!(read_git_target(&workspace_root), @"../../../.git");

    // Check that the Git repo's HEAD got checked out
    insta::assert_snapshot!(get_log_output(&test_env, &workspace_root), @r"
    @  5f169ecc57b8
    ○  e80a42cccd06 my-bookmark git_head() My commit message
    ◆  000000000000
    [EOF]
    ");

    // Check that the Git repo's HEAD moves
    test_env.run_jj_in(&workspace_root, ["new"]).success();
    insta::assert_snapshot!(get_log_output(&test_env, &workspace_root), @r"
    @  62eda98b5eb4
    ○  5f169ecc57b8 git_head()
    ○  e80a42cccd06 my-bookmark My commit message
    ◆  000000000000
    [EOF]
    ");
}

#[cfg(unix)]
#[test]
fn test_git_init_colocated_via_git_repo_path_symlink_directory_without_bare_config() {
    let test_env = TestEnvironment::default();
    // <workspace_root>/.git -> <git_repo_path>
    let git_repo_path = test_env.env_root().join("git-repo.git");
    let workspace_root = test_env.env_root().join("repo");
    // Set up git repo without core.bare set (as the "repo" tool would do.)
    // The core.bare config is deduced from the directory name.
    let git_repo = init_git_repo(&workspace_root, false);
    git::remove_config_value(git_repo, "config", "bare");

    std::fs::rename(workspace_root.join(".git"), &git_repo_path).unwrap();
    std::os::unix::fs::symlink(&git_repo_path, workspace_root.join(".git")).unwrap();
    let output = test_env.run_jj_in(&workspace_root, ["git", "init", "--git-repo", "."]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Done importing changes from the underlying Git repo.
    Initialized repo in "."
    [EOF]
    "#);
    insta::assert_snapshot!(read_git_target(&workspace_root), @"../../../.git");

    // Check that the Git repo's HEAD got checked out
    insta::assert_snapshot!(get_log_output(&test_env, &workspace_root), @r"
    @  5f169ecc57b8
    ○  e80a42cccd06 my-bookmark git_head() My commit message
    ◆  000000000000
    [EOF]
    ");

    // Check that the Git repo's HEAD moves
    test_env.run_jj_in(&workspace_root, ["new"]).success();
    insta::assert_snapshot!(get_log_output(&test_env, &workspace_root), @r"
    @  62eda98b5eb4
    ○  5f169ecc57b8 git_head()
    ○  e80a42cccd06 my-bookmark My commit message
    ◆  000000000000
    [EOF]
    ");
}

#[cfg(unix)]
#[test]
fn test_git_init_colocated_via_git_repo_path_symlink_gitlink() {
    let test_env = TestEnvironment::default();
    // <workspace_root>/.git -> <git_workdir_path>/.git -> <git_repo_path>
    let git_repo_path = test_env.env_root().join("git-repo");
    let git_workdir_path = test_env.env_root().join("git-workdir");
    let workspace_root = test_env.env_root().join("repo");
    let git_repo = init_git_repo(&git_repo_path, false);
    std::fs::create_dir(&git_workdir_path).unwrap();
    git::create_gitlink(&git_workdir_path, git_repo.path());
    assert!(git_workdir_path.join(".git").is_file());
    std::fs::create_dir(&workspace_root).unwrap();
    std::os::unix::fs::symlink(git_workdir_path.join(".git"), workspace_root.join(".git")).unwrap();
    let output = test_env.run_jj_in(&workspace_root, ["git", "init", "--git-repo", "."]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Done importing changes from the underlying Git repo.
    Initialized repo in "."
    [EOF]
    "#);
    insta::assert_snapshot!(read_git_target(&workspace_root), @"../../../.git");

    // Check that the Git repo's HEAD got checked out
    insta::assert_snapshot!(get_log_output(&test_env, &workspace_root), @r"
    @  5f169ecc57b8
    ○  e80a42cccd06 my-bookmark git_head() My commit message
    ◆  000000000000
    [EOF]
    ");

    // Check that the Git repo's HEAD moves
    test_env.run_jj_in(&workspace_root, ["new"]).success();
    insta::assert_snapshot!(get_log_output(&test_env, &workspace_root), @r"
    @  62eda98b5eb4
    ○  5f169ecc57b8 git_head()
    ○  e80a42cccd06 my-bookmark My commit message
    ◆  000000000000
    [EOF]
    ");
}

#[test]
fn test_git_init_colocated_via_git_repo_path_imported_refs() {
    let test_env = TestEnvironment::default();
    test_env.add_config("git.auto-local-bookmark = true");

    // Set up remote refs
    test_env.run_jj_in(".", ["git", "init", "remote"]).success();
    let remote_path = test_env.env_root().join("remote");
    test_env
        .run_jj_in(
            &remote_path,
            ["bookmark", "create", "-r@", "local-remote", "remote-only"],
        )
        .success();
    test_env.run_jj_in(&remote_path, ["new"]).success();
    test_env
        .run_jj_in(&remote_path, ["git", "export"])
        .success();

    let remote_git_path = remote_path.join(PathBuf::from_iter([".jj", "repo", "store", "git"]));
    let set_up_local_repo = |local_path: &Path| {
        let git_repo = git::clone(local_path, remote_git_path.to_str().unwrap());
        let git_ref = git_repo
            .find_reference("refs/remotes/origin/local-remote")
            .unwrap();
        git_repo
            .reference(
                "refs/heads/local-remote",
                git_ref.target().id().to_owned(),
                gix::refs::transaction::PreviousValue::MustNotExist,
                "move local-remote bookmark",
            )
            .unwrap();
    };

    // With git.auto-local-bookmark = true
    let local_path = test_env.env_root().join("local1");
    set_up_local_repo(&local_path);
    let output = test_env.run_jj_in(&local_path, ["git", "init", "--git-repo=."]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Done importing changes from the underlying Git repo.
    Initialized repo in "."
    [EOF]
    "#);
    insta::assert_snapshot!(get_bookmark_output(&test_env, &local_path), @r"
    local-remote: vvkvtnvv 230dd059 (empty) (no description set)
      @git: vvkvtnvv 230dd059 (empty) (no description set)
      @origin: vvkvtnvv 230dd059 (empty) (no description set)
    remote-only: vvkvtnvv 230dd059 (empty) (no description set)
      @git: vvkvtnvv 230dd059 (empty) (no description set)
      @origin: vvkvtnvv 230dd059 (empty) (no description set)
    [EOF]
    ");

    // With git.auto-local-bookmark = false
    test_env.add_config("git.auto-local-bookmark = false");
    let local_path = test_env.env_root().join("local2");
    set_up_local_repo(&local_path);
    let output = test_env.run_jj_in(&local_path, ["git", "init", "--git-repo=."]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Done importing changes from the underlying Git repo.
    Hint: The following remote bookmarks aren't associated with the existing local bookmarks:
      local-remote@origin
    Hint: Run `jj bookmark track local-remote@origin` to keep local bookmarks updated on future pulls.
    Initialized repo in "."
    [EOF]
    "#);
    insta::assert_snapshot!(get_bookmark_output(&test_env, &local_path), @r"
    local-remote: vvkvtnvv 230dd059 (empty) (no description set)
      @git: vvkvtnvv 230dd059 (empty) (no description set)
    local-remote@origin: vvkvtnvv 230dd059 (empty) (no description set)
    remote-only@origin: vvkvtnvv 230dd059 (empty) (no description set)
    [EOF]
    ");
}

#[test]
fn test_git_init_colocated_dirty_working_copy() {
    let test_env = TestEnvironment::default();
    let workspace_root = test_env.env_root().join("repo");
    let git_repo = init_git_repo(&workspace_root, false);

    let mut index_manager = git::IndexManager::new(&git_repo);

    index_manager.add_file("new-staged-file", b"new content");
    index_manager.add_file("some-file", b"new content");
    index_manager.sync_index();

    std::fs::write(workspace_root.join("unstaged-file"), "new content").unwrap();
    insta::assert_debug_snapshot!(git::status(&git_repo), @r#"
    [
        GitStatus {
            path: "new-staged-file",
            status: Index(
                Addition,
            ),
        },
        GitStatus {
            path: "some-file",
            status: Index(
                Modification,
            ),
        },
        GitStatus {
            path: "unstaged-file",
            status: Worktree(
                Added,
            ),
        },
    ]
    "#);

    let output = test_env.run_jj_in(&workspace_root, ["git", "init", "--git-repo", "."]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Done importing changes from the underlying Git repo.
    Initialized repo in "."
    [EOF]
    "#);

    // Working-copy changes should have been snapshotted.
    let output = test_env.run_jj_in(&workspace_root, ["log", "-s", "--ignore-working-copy"]);
    insta::assert_snapshot!(output, @r"
    @  sqpuoqvx test.user@example.com 2001-02-03 08:05:07 36dbd9a1
    │  (no description set)
    │  C {some-file => new-staged-file}
    │  M some-file
    │  C {some-file => unstaged-file}
    ○  nntyzxmz someone@example.org 1970-01-01 11:00:00 my-bookmark git_head() e80a42cc
    │  My commit message
    │  A some-file
    ◆  zzzzzzzz root() 00000000
    [EOF]
    ");

    // Git index should be consistent with the working copy parent. With the
    // current implementation, the index is unchanged. Since jj created new
    // working copy commit, it's also okay to update the index reflecting the
    // working copy commit or the working copy parent.
    insta::assert_debug_snapshot!(git::status(&git_repo), @r#"
    [
        GitStatus {
            path: ".jj/.gitignore",
            status: Worktree(
                Ignored,
            ),
        },
        GitStatus {
            path: ".jj/repo",
            status: Worktree(
                Ignored,
            ),
        },
        GitStatus {
            path: ".jj/working_copy",
            status: Worktree(
                Ignored,
            ),
        },
        GitStatus {
            path: "new-staged-file",
            status: Index(
                Addition,
            ),
        },
        GitStatus {
            path: "some-file",
            status: Index(
                Modification,
            ),
        },
        GitStatus {
            path: "unstaged-file",
            status: Worktree(
                Added,
            ),
        },
    ]
    "#);
}

#[test]
fn test_git_init_colocated_ignore_working_copy() {
    let test_env = TestEnvironment::default();
    let workspace_root = test_env.env_root().join("repo");
    init_git_repo(&workspace_root, false);
    std::fs::write(workspace_root.join("file1"), "").unwrap();

    let output = test_env.run_jj_in(
        &workspace_root,
        ["git", "init", "--ignore-working-copy", "--colocate"],
    );
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: --ignore-working-copy is not respected
    [EOF]
    [exit status: 2]
    ");
}

#[test]
fn test_git_init_colocated_at_operation() {
    let test_env = TestEnvironment::default();
    let workspace_root = test_env.env_root().join("repo");
    init_git_repo(&workspace_root, false);

    let output = test_env.run_jj_in(&workspace_root, ["git", "init", "--at-op=@-", "--colocate"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: --at-op is not respected
    [EOF]
    [exit status: 2]
    ");
}

#[test]
fn test_git_init_external_but_git_dir_exists() {
    let test_env = TestEnvironment::default();
    let git_repo_path = test_env.env_root().join("git-repo");
    let workspace_root = test_env.env_root().join("repo");
    git::init(&git_repo_path);
    init_git_repo(&workspace_root, false);
    let output = test_env.run_jj_in(
        &workspace_root,
        ["git", "init", "--git-repo", git_repo_path.to_str().unwrap()],
    );
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Initialized repo in "."
    [EOF]
    "#);

    // The local ".git" repository is unrelated, so no commits should be imported
    insta::assert_snapshot!(get_log_output(&test_env, &workspace_root), @r"
    @  230dd059e1b0
    ◆  000000000000
    [EOF]
    ");

    // Check that Git HEAD is not set because this isn't a colocated repo
    test_env.run_jj_in(&workspace_root, ["new"]).success();
    insta::assert_snapshot!(get_log_output(&test_env, &workspace_root), @r"
    @  4db490c88528
    ○  230dd059e1b0
    ◆  000000000000
    [EOF]
    ");
}

#[test]
fn test_git_init_colocated_via_flag_git_dir_exists() {
    let test_env = TestEnvironment::default();
    let workspace_root = test_env.env_root().join("repo");
    init_git_repo(&workspace_root, false);

    let output = test_env.run_jj_in(".", ["git", "init", "--colocate", "repo"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Done importing changes from the underlying Git repo.
    Initialized repo in "repo"
    [EOF]
    "#);

    // Check that the Git repo's HEAD got checked out
    insta::assert_snapshot!(get_log_output(&test_env, &workspace_root), @r"
    @  5f169ecc57b8
    ○  e80a42cccd06 my-bookmark git_head() My commit message
    ◆  000000000000
    [EOF]
    ");

    // Check that the Git repo's HEAD moves
    test_env.run_jj_in(&workspace_root, ["new"]).success();
    insta::assert_snapshot!(get_log_output(&test_env, &workspace_root), @r"
    @  62eda98b5eb4
    ○  5f169ecc57b8 git_head()
    ○  e80a42cccd06 my-bookmark My commit message
    ◆  000000000000
    [EOF]
    ");
}

#[test]
fn test_git_init_colocated_via_flag_git_dir_not_exists() {
    let test_env = TestEnvironment::default();
    let workspace_root = test_env.env_root().join("repo");
    let output = test_env.run_jj_in(".", ["git", "init", "--colocate", "repo"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Initialized repo in "repo"
    [EOF]
    "#);
    // No HEAD ref is available yet
    insta::assert_snapshot!(get_log_output(&test_env, &workspace_root), @r"
    @  230dd059e1b0
    ◆  000000000000
    [EOF]
    ");

    // Create the default bookmark (create both in case we change the default)
    test_env
        .run_jj_in(
            &workspace_root,
            ["bookmark", "create", "-r@", "main", "master"],
        )
        .success();

    // If .git/HEAD pointed to the default bookmark, new working-copy commit would
    // be created on top.
    insta::assert_snapshot!(get_log_output(&test_env, &workspace_root), @r"
    @  230dd059e1b0 main master
    ◆  000000000000
    [EOF]
    ");
}

#[test]
fn test_git_init_conditional_config() {
    let test_env = TestEnvironment::default();
    let old_workspace_root = test_env.env_root().join("old");
    let new_workspace_root = test_env.env_root().join("new");

    let run_jj_in = |current_dir: &Path, args: &[&str]| {
        test_env.run_jj_with(|cmd| {
            cmd.current_dir(current_dir)
                .args(args)
                .env_remove("JJ_EMAIL")
                .env_remove("JJ_OP_HOSTNAME")
                .env_remove("JJ_OP_USERNAME")
        })
    };
    let log_template = r#"separate(' ', author.email(), description.first_line()) ++ "\n""#;
    let op_log_template = r#"separate(' ', user, description.first_line()) ++ "\n""#;

    // Override user.email and operation.username conditionally
    test_env.add_config(formatdoc! {"
        user.email = 'base@example.org'
        operation.hostname = 'base'
        operation.username = 'base'
        [[--scope]]
        --when.repositories = [{new_workspace_root}]
        user.email = 'new-repo@example.org'
        operation.username = 'new-repo'
        ",
        new_workspace_root = to_toml_value(new_workspace_root.to_str().unwrap()),
    });

    // Override operation.hostname by repo config, which should be loaded into
    // the command settings, but shouldn't be copied to the new repo.
    run_jj_in(test_env.env_root(), &["git", "init", "old"]).success();
    run_jj_in(
        &old_workspace_root,
        &["config", "set", "--repo", "operation.hostname", "old-repo"],
    )
    .success();
    run_jj_in(&old_workspace_root, &["new"]).success();
    let output = run_jj_in(&old_workspace_root, &["op", "log", "-T", op_log_template]);
    insta::assert_snapshot!(output, @r"
    @  base@old-repo new empty commit
    ○  base@base add workspace 'default'
    ○  @
    [EOF]
    ");

    // Create new repo at the old workspace directory.
    let output = run_jj_in(&old_workspace_root, &["git", "init", "../new"]);
    insta::assert_snapshot!(output.normalize_backslash(), @r#"
    ------- stderr -------
    Initialized repo in "../new"
    [EOF]
    "#);
    run_jj_in(&new_workspace_root, &["new"]).success();
    let output = run_jj_in(&new_workspace_root, &["log", "-T", log_template]);
    insta::assert_snapshot!(output, @r"
    @  new-repo@example.org
    ○  new-repo@example.org
    ◆
    [EOF]
    ");
    let output = run_jj_in(&new_workspace_root, &["op", "log", "-T", op_log_template]);
    insta::assert_snapshot!(output, @r"
    @  new-repo@base new empty commit
    ○  new-repo@base add workspace 'default'
    ○  @
    [EOF]
    ");
}

#[test]
fn test_git_init_bad_wc_path() {
    let test_env = TestEnvironment::default();
    std::fs::write(test_env.env_root().join("existing-file"), b"").unwrap();
    let output = test_env.run_jj_in(".", ["git", "init", "existing-file"]);
    insta::assert_snapshot!(output.strip_stderr_last_line(), @r"
    ------- stderr -------
    Error: Failed to create workspace
    [EOF]
    [exit status: 1]
    ");
}
