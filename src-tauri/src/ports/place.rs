//! Which Git checkout a project folder belongs to.
//!
//! A linked worktree and the main checkout share one repository id (the common
//! git directory) and keep distinct worktree paths, so their servers are never
//! merged. A submodule or a nested repository has its own git directory and is
//! its own repository. The nearest `.git` wins.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;

use super::project;

/// A checkout's branch, worktree and repository. Paths are canonical.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checkout {
    pub branch: Option<String>,
    /// Set only when this checkout is a linked worktree.
    pub worktree: Option<String>,
    pub worktree_path: PathBuf,
    pub repository_path: PathBuf,
    pub repository_name: String,
    /// The common git directory. Shared by every worktree of one repository.
    pub repository_id: PathBuf,
}

/// What the windows show. Paths inside home use the `~/` form, matching `folder`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Place {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worktree: Option<String>,
    pub worktree_path: String,
    pub repository: String,
    pub repository_name: String,
    pub repository_id: String,
}

impl Checkout {
    pub fn place(&self, home: &Path) -> Place {
        Place {
            branch: self.branch.clone(),
            worktree: self.worktree.clone(),
            worktree_path: project::display_path(&self.worktree_path, home),
            repository: project::display_path(&self.repository_path, home),
            repository_name: self.repository_name.clone(),
            repository_id: project::display_path(&self.repository_id, home),
        }
    }
}

/// The checkout that owns `folder`, walking up to the nearest `.git`.
/// `folder` and `home` follow the same rules as `project::git_branch`.
pub fn checkout(folder: &Path, home: &Path) -> Option<Checkout> {
    let (worktree_path, git_dir) = git_paths(folder, home)?;
    let head = fs::read_to_string(git_dir.join("HEAD")).ok()?;
    let branch = branch_of(head.trim());
    let linked = linked_worktree_name(&git_dir);
    let repository_id = common_git_dir(&git_dir);
    let submodule = is_submodule(&git_dir);
    let repository_path = if submodule {
        worktree_path.clone()
    } else {
        repository_id
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| worktree_path.clone())
    };
    let repository_name = repository_path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| repository_path.display().to_string());
    Some(Checkout {
        branch,
        worktree: linked,
        worktree_path,
        repository_path,
        repository_name,
        repository_id,
    })
}

/// The checked-out branch, or the short commit when HEAD is detached.
pub fn branch_of(head: &str) -> Option<String> {
    match head.strip_prefix("ref: ") {
        Some(reference) => Some(
            reference
                .strip_prefix("refs/heads/")
                .unwrap_or(reference)
                .to_string(),
        ),
        None if head.is_empty() => None,
        None => head.get(..7).map(str::to_string),
    }
}

fn git_paths(folder: &Path, home: &Path) -> Option<(PathBuf, PathBuf)> {
    let dot_git = folder
        .ancestors()
        .take_while(|dir| *dir != home)
        .map(|dir| dir.join(".git"))
        .find(|path| path.exists())?;
    let worktree_path = project::canonical(dot_git.parent()?);
    let git_dir = if dot_git.is_file() {
        let pointer = fs::read_to_string(&dot_git).ok()?;
        let target = pointer.trim().strip_prefix("gitdir:")?.trim();
        project::canonical(&dot_git.parent()?.join(target))
    } else {
        project::canonical(&dot_git)
    };
    Some((worktree_path, git_dir))
}

/// `repo/.git/worktrees/<name>` → `<name>`.
fn linked_worktree_name(git_dir: &Path) -> Option<String> {
    let parent = git_dir.parent()?;
    if parent.file_name()? != "worktrees" {
        return None;
    }
    Some(git_dir.file_name()?.to_string_lossy().into_owned())
}

fn is_submodule(git_dir: &Path) -> bool {
    linked_worktree_name(git_dir).is_none()
        && git_dir
            .components()
            .any(|component| component.as_os_str() == "modules")
}

/// The directory every worktree of a repository shares.
fn common_git_dir(git_dir: &Path) -> PathBuf {
    let pointer = git_dir.join("commondir");
    if let Ok(raw) = fs::read_to_string(&pointer) {
        let target = raw.trim();
        if !target.is_empty() {
            return project::canonical(&git_dir.join(target));
        }
    }
    if linked_worktree_name(git_dir).is_some() {
        return project::canonical(&git_dir.join("../.."));
    }
    git_dir.to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn tree(files: &[(&str, &str)]) -> TempDir {
        let dir = TempDir::new().unwrap();
        for (name, contents) in files {
            let path = dir.path().join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, contents).unwrap();
        }
        dir
    }

    fn home_of(dir: &TempDir) -> PathBuf {
        project::canonical(dir.path())
    }

    #[test]
    fn reads_a_branch_or_a_short_detached_commit() {
        assert_eq!(
            branch_of("ref: refs/heads/feat/checkout"),
            Some("feat/checkout".to_string())
        );
        assert_eq!(branch_of("4a1e04d7c0ffee"), Some("4a1e04d".to_string()));
        assert_eq!(branch_of(""), None);
    }

    #[test]
    fn two_worktrees_share_a_repository_and_keep_their_own_paths() {
        let home = tree(&[
            ("repo/.git/HEAD", "ref: refs/heads/main\n"),
            (
                "repo/.git/worktrees/hotfix/HEAD",
                "ref: refs/heads/hotfix\n",
            ),
            ("repo/.git/worktrees/hotfix/commondir", "../..\n"),
            ("hotfix/.git", "gitdir: ../repo/.git/worktrees/hotfix\n"),
            ("repo/apps/web/package.json", "{}"),
            ("hotfix/apps/api/package.json", "{}"),
        ]);
        let root = home_of(&home);
        let main = checkout(&root.join("repo/apps/web"), &root).unwrap();
        let hotfix = checkout(&root.join("hotfix/apps/api"), &root).unwrap();

        assert_eq!(main.repository_id, hotfix.repository_id);
        assert_eq!(main.worktree, None);
        assert_eq!(hotfix.worktree.as_deref(), Some("hotfix"));
        assert_ne!(main.worktree_path, hotfix.worktree_path);
        assert_eq!(main.branch.as_deref(), Some("main"));
        assert_eq!(hotfix.branch.as_deref(), Some("hotfix"));
        assert_eq!(main.repository_name, "repo");
    }

    #[test]
    fn a_submodule_is_its_own_repository() {
        let home = tree(&[
            ("repo/.git/HEAD", "ref: refs/heads/main\n"),
            ("repo/.git/modules/pkg/HEAD", "ref: refs/heads/pkg\n"),
            ("repo/pkg/.git", "gitdir: ../.git/modules/pkg\n"),
        ]);
        let root = home_of(&home);
        let parent = checkout(&root.join("repo"), &root).unwrap();
        let child = checkout(&root.join("repo/pkg"), &root).unwrap();
        assert_ne!(parent.repository_id, child.repository_id);
        assert_eq!(child.worktree, None);
        assert_eq!(child.repository_name, "pkg");
        assert_eq!(child.branch.as_deref(), Some("pkg"));
    }

    #[test]
    fn a_nested_repository_does_not_inherit_its_parent() {
        let home = tree(&[
            ("outer/.git/HEAD", "ref: refs/heads/outer\n"),
            ("outer/inner/.git/HEAD", "ref: refs/heads/inner\n"),
        ]);
        let root = home_of(&home);
        let inner = checkout(&root.join("outer/inner"), &root).unwrap();
        let outer = checkout(&root.join("outer"), &root).unwrap();
        assert_ne!(inner.repository_id, outer.repository_id);
        assert_eq!(inner.branch.as_deref(), Some("inner"));
    }
}
