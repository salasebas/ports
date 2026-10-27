//! What a server is: its project folder, git branch and framework.

use std::collections::HashSet;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

const PROJECT_MARKERS: [&str; 7] = [
    "package.json",
    "Cargo.toml",
    "go.mod",
    "pyproject.toml",
    "Gemfile",
    "composer.json",
    ".git",
];

/// Manifests whose dependency names identify a framework.
const MANIFESTS: [&str; 7] = [
    "package.json",
    "Cargo.toml",
    "go.mod",
    "pyproject.toml",
    "requirements.txt",
    "Gemfile",
    "composer.json",
];

/// Executables that are a framework on their own. Checked in order.
const COMMANDS: [(&str, &str); 22] = [
    ("postgres", "PostgreSQL"),
    ("postmaster", "PostgreSQL"),
    ("mysqld", "MySQL"),
    ("mariadbd", "MariaDB"),
    ("mongod", "MongoDB"),
    ("redis-server", "Redis"),
    ("valkey-server", "Valkey"),
    ("memcached", "Memcached"),
    ("nginx", "nginx"),
    ("caddy", "Caddy"),
    ("next", "Next.js"),
    ("next-server", "Next.js"),
    ("nuxt", "Nuxt"),
    ("nuxi", "Nuxt"),
    ("astro", "Astro"),
    ("remix", "Remix"),
    ("ng", "Angular"),
    ("rails", "Rails"),
    ("manage.py", "Django"),
    ("flask", "Flask"),
    ("artisan", "Laravel"),
    ("hugo", "Hugo"),
];

/// Official images named differently from their server's executable.
const IMAGES: [(&str, &str); 6] = [
    ("postgres", "PostgreSQL"),
    ("mysql", "MySQL"),
    ("mariadb", "MariaDB"),
    ("mongo", "MongoDB"),
    ("redis", "Redis"),
    ("valkey", "Valkey"),
];

/// Generic runners whose framework is better named by the project's dependencies.
const RUNNERS: [(&str, &str, &[&str]); 6] = [
    (
        "vite",
        "Vite",
        &["SvelteKit", "Astro", "Remix", "React Router", "Nuxt"],
    ),
    ("uvicorn", "Uvicorn", &["FastAPI", "Starlette", "Django"]),
    (
        "hypercorn",
        "Hypercorn",
        &["FastAPI", "Starlette", "Django"],
    ),
    ("gunicorn", "Gunicorn", &["Django", "Flask", "FastAPI"]),
    ("puma", "Puma", &["Rails", "Sinatra"]),
    ("webpack", "webpack", &["Angular"]),
];

/// Dependency names, most specific first: a Next.js app also depends on React.
const DEPENDENCIES: [(&str, &str); 28] = [
    ("next", "Next.js"),
    ("nuxt", "Nuxt"),
    ("@sveltejs/kit", "SvelteKit"),
    ("astro", "Astro"),
    ("@remix-run/dev", "Remix"),
    ("@react-router/dev", "React Router"),
    ("gatsby", "Gatsby"),
    ("@angular/core", "Angular"),
    ("@nestjs/core", "NestJS"),
    ("express", "Express"),
    ("fastify", "Fastify"),
    ("hono", "Hono"),
    ("koa", "Koa"),
    ("vite", "Vite"),
    ("fastapi", "FastAPI"),
    ("django", "Django"),
    ("flask", "Flask"),
    ("starlette", "Starlette"),
    ("rails", "Rails"),
    ("sinatra", "Sinatra"),
    ("laravel/framework", "Laravel"),
    ("axum", "Axum"),
    ("actix-web", "Actix Web"),
    ("rocket", "Rocket"),
    ("github.com/gin-gonic/gin", "Gin"),
    ("github.com/labstack/echo/v4", "Echo"),
    ("github.com/gofiber/fiber/v2", "Fiber"),
    ("tauri", "Tauri"),
];

const DATABASES: [&str; 7] = [
    "PostgreSQL",
    "MySQL",
    "MariaDB",
    "MongoDB",
    "Redis",
    "Valkey",
    "Memcached",
];

/// The framework of a container image, by its name without registry or tag.
pub fn image_framework(image: &str) -> Option<&'static str> {
    IMAGES
        .iter()
        .chain(&COMMANDS)
        .find(|(name, _)| *name == image)
        .map(|(_, framework)| *framework)
}

/// A program inside a macOS app bundle (or a Linux one shipped the same way,
/// like Zed's) is a desktop app, whatever folder it was opened from. Python
/// runs from a `Python.app` inside its framework, which doesn't count.
pub fn is_app_bundle(exe: &Path) -> bool {
    exe.components()
        .map(|part| part.as_os_str().to_string_lossy())
        .take_while(|part| !part.ends_with(".framework"))
        .any(|part| part.ends_with(".app"))
}

/// What usually listens on a port, for sockets whose process can't be seen.
pub fn well_known(port: u16) -> Option<(&'static str, Option<&'static str>)> {
    let found = match port {
        22 => ("ssh", None),
        53 => ("dns", None),
        80 => ("http", None),
        443 => ("https", None),
        631 => ("cups", None),
        3306 => ("mysql", Some("MySQL")),
        5432 => ("postgres", Some("PostgreSQL")),
        6379 => ("redis", Some("Redis")),
        11211 => ("memcached", Some("Memcached")),
        27017 => ("mongodb", Some("MongoDB")),
        _ => return None,
    };
    Some(found)
}

pub fn is_database(framework: &str) -> bool {
    DATABASES.contains(&framework)
}

/// The nearest folder at or above `cwd` with a project marker. Only folders
/// inside the home folder count, so services running from `/` or from
/// Homebrew's prefix (itself a git checkout) have no project.
pub fn project_folder(cwd: &Path, home: &Path) -> Option<PathBuf> {
    cwd.ancestors()
        .take_while(|dir| dir.starts_with(home) && *dir != home)
        .find(|dir| PROJECT_MARKERS.iter().any(|m| dir.join(m).exists()))
        .map(Path::to_path_buf)
}

/// The checked-out branch, or the short commit when HEAD is detached.
pub fn git_branch(folder: &Path, home: &Path) -> Option<String> {
    let dot_git = folder
        .ancestors()
        .take_while(|dir| *dir != home)
        .map(|dir| dir.join(".git"))
        .find(|path| path.exists())?;
    // Worktrees and submodules have a `.git` file pointing at the real git dir.
    let git_dir = if dot_git.is_file() {
        let pointer = fs::read_to_string(&dot_git).ok()?;
        let target = pointer.trim().strip_prefix("gitdir:")?.trim();
        dot_git.parent()?.join(target)
    } else {
        dot_git
    };
    let head = fs::read_to_string(git_dir.join("HEAD")).ok()?;
    let head = head.trim();
    match head.strip_prefix("ref: ") {
        Some(reference) => Some(
            reference
                .strip_prefix("refs/heads/")
                .unwrap_or(reference)
                .to_string(),
        ),
        None => head.get(..7).map(str::to_string),
    }
}

/// Every word of the command lines, with paths reduced to file names, so
/// `node ./node_modules/.bin/vite` and `next-server (v15.1.0)` both match.
pub fn command_words<'a>(commands: impl IntoIterator<Item = &'a [OsString]>) -> HashSet<String> {
    commands
        .into_iter()
        .flatten()
        .flat_map(|arg| {
            arg.to_string_lossy()
                .split_whitespace()
                .map(|word| file_name(word).to_string())
                .collect::<Vec<_>>()
        })
        .collect()
}

pub fn framework(words: &HashSet<String>, folder: Option<&Path>) -> Option<&'static str> {
    if let Some((_, name)) = COMMANDS.iter().find(|(word, _)| words.contains(*word)) {
        return Some(name);
    }
    let from_dependencies = folder.and_then(dependency_framework);
    if let Some((_, runner, refinements)) = RUNNERS.iter().find(|(word, ..)| words.contains(*word))
    {
        return Some(
            from_dependencies
                .filter(|name| refinements.contains(name))
                .unwrap_or(runner),
        );
    }
    from_dependencies
}

fn dependency_framework(folder: &Path) -> Option<&'static str> {
    let names = dependency_names(folder);
    DEPENDENCIES
        .iter()
        .find(|(dependency, _)| names.contains(*dependency))
        .map(|(_, name)| *name)
}

/// Dependency names from every manifest in `folder`. `package.json` is read
/// properly; the others are split into words, which finds dependency names in
/// TOML, `go.mod`, `requirements.txt` and Gemfiles alike.
fn dependency_names(folder: &Path) -> HashSet<String> {
    let mut names = HashSet::new();
    for manifest in MANIFESTS {
        let Ok(text) = fs::read_to_string(folder.join(manifest)) else {
            continue;
        };
        if manifest == "package.json" {
            let json: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
            for key in ["dependencies", "devDependencies"] {
                if let Some(deps) = json.get(key).and_then(|d| d.as_object()) {
                    names.extend(deps.keys().cloned());
                }
            }
        } else {
            names.extend(
                text.split(|c: char| !(c.is_alphanumeric() || "-_./@".contains(c)))
                    .filter(|word| !word.is_empty())
                    .map(str::to_lowercase),
            );
        }
    }
    names
}

fn file_name(word: &str) -> &str {
    word.rsplit('/').next().unwrap_or(word)
}

/// `~/projects/web` for folders inside home, the full path otherwise.
pub fn display_path(path: &Path, home: &Path) -> String {
    match path.strip_prefix(home) {
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}

/// A readable command line: `npm run dev` rather than
/// `/opt/homebrew/bin/node /opt/homebrew/bin/npm run dev`.
pub fn display_command(cmd: &[OsString]) -> String {
    const INTERPRETERS: [&str; 7] = ["node", "bun", "deno", "python", "python3", "ruby", "php"];
    let args: Vec<String> = cmd
        .iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    let Some(program) = args.first() else {
        return String::new();
    };
    let program = file_name(program);
    let script = args.get(1).filter(|arg| arg.contains('/'));
    match script {
        Some(script) if INTERPRETERS.contains(&program) => {
            let script = file_name(script);
            let script = [".js", ".mjs", ".cjs"]
                .iter()
                .find_map(|ext| script.strip_suffix(ext))
                .unwrap_or(script);
            std::iter::once(script.to_string())
                .chain(args[2..].iter().cloned())
                .collect::<Vec<_>>()
                .join(" ")
        }
        _ => std::iter::once(program.to_string())
            .chain(args[1..].iter().cloned())
            .collect::<Vec<_>>()
            .join(" "),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn words(command: &str) -> HashSet<String> {
        let args: Vec<OsString> = command.split(' ').map(OsString::from).collect();
        command_words([args.as_slice()])
    }

    fn project(files: &[(&str, &str)]) -> TempDir {
        let dir = TempDir::new().unwrap();
        for (name, contents) in files {
            let path = dir.path().join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, contents).unwrap();
        }
        dir
    }

    #[test]
    fn finds_the_nearest_project_above_the_working_directory() {
        let home = project(&[("projects/web/package.json", "{}")]);
        fs::create_dir_all(home.path().join("projects/web/src/routes")).unwrap();
        assert_eq!(
            project_folder(&home.path().join("projects/web/src/routes"), home.path()),
            Some(home.path().join("projects/web"))
        );
    }

    #[test]
    fn knows_a_desktop_app_by_its_bundle() {
        // Zed or OrbStack opened from a terminal inside a project keep that folder as cwd.
        assert!(is_app_bundle(Path::new(
            "/Applications/Zed.app/Contents/MacOS/zed"
        )));
        assert!(is_app_bundle(Path::new(
            "/home/me/.local/zed.app/libexec/zed-editor"
        )));
        assert!(!is_app_bundle(Path::new("/Users/me/.bun/bin/bun")));
        assert!(!is_app_bundle(Path::new(
            "/opt/homebrew/Cellar/python@3.14/3.14.0/Frameworks/Python.framework/Versions/3.14/Resources/Python.app/Contents/MacOS/Python"
        )));
    }

    #[test]
    fn has_no_project_outside_home_or_at_home_itself() {
        let home = project(&[(".git/HEAD", "ref: refs/heads/dotfiles\n")]);
        let outside = project(&[("package.json", "{}")]);
        assert_eq!(project_folder(home.path(), home.path()), None);
        assert_eq!(project_folder(outside.path(), home.path()), None);
        assert_eq!(project_folder(Path::new("/"), home.path()), None);
    }

    #[test]
    fn reads_the_branch_of_a_monorepo_package() {
        let home = project(&[
            ("mono/.git/HEAD", "ref: refs/heads/feat/checkout\n"),
            ("mono/apps/web/package.json", "{}"),
        ]);
        assert_eq!(
            git_branch(&home.path().join("mono/apps/web"), home.path()),
            Some("feat/checkout".to_string())
        );
    }

    #[test]
    fn reads_the_branch_of_a_worktree() {
        let home = project(&[
            (
                "repo/.git/worktrees/hotfix/HEAD",
                "ref: refs/heads/hotfix\n",
            ),
            ("hotfix/.git", "gitdir: ../repo/.git/worktrees/hotfix\n"),
        ]);
        assert_eq!(
            git_branch(&home.path().join("hotfix"), home.path()),
            Some("hotfix".to_string())
        );
    }

    #[test]
    fn shows_the_short_commit_of_a_detached_head() {
        let home = project(&[("repo/.git/HEAD", "4a1e04d7c0ffee\n")]);
        assert_eq!(
            git_branch(&home.path().join("repo"), home.path()),
            Some("4a1e04d".to_string())
        );
    }

    #[test]
    fn names_the_framework_from_the_command_and_the_dependencies() {
        let sveltekit = project(&[(
            "package.json",
            r#"{"devDependencies":{"@sveltejs/kit":"^2","vite":"^7"}}"#,
        )]);
        let next = project(&[(
            "package.json",
            r#"{"dependencies":{"next":"15","react":"19"}}"#,
        )]);
        let express = project(&[(
            "package.json",
            r#"{"dependencies":{"express":"5"},"devDependencies":{"vite":"7"}}"#,
        )]);
        let fastapi = project(&[(
            "pyproject.toml",
            "[project]\ndependencies = [\"fastapi[standard]>=0.115\"]\n",
        )]);
        let rails = project(&[("Gemfile", "gem \"rails\", \"~> 8.0\"\ngem \"puma\"\n")]);

        let cases = [
            (
                "node /p/node_modules/.bin/vite dev",
                Some(&sveltekit),
                "SvelteKit",
            ),
            ("node /p/node_modules/.bin/vite", Some(&express), "Vite"),
            ("next-server (v15.1.0)", Some(&next), "Next.js"),
            ("node server.js", Some(&express), "Express"),
            (
                "python -m uvicorn app.main:app --reload",
                Some(&fastapi),
                "FastAPI",
            ),
            (
                "puma 6.4.0 (tcp://localhost:3000) [blog]",
                Some(&rails),
                "Rails",
            ),
            (
                "/opt/homebrew/opt/postgresql@16/bin/postgres -D /opt/homebrew/var/postgresql@16",
                None,
                "PostgreSQL",
            ),
        ];
        for (command, folder, expected) in cases {
            assert_eq!(
                framework(&words(command), folder.map(TempDir::path)),
                Some(expected),
                "{command}"
            );
        }
    }

    #[test]
    fn shortens_interpreter_command_lines() {
        let cmd = |s: &str| s.split(' ').map(OsString::from).collect::<Vec<_>>();
        assert_eq!(
            display_command(&cmd("/opt/homebrew/bin/node /opt/homebrew/bin/npm run dev")),
            "npm run dev"
        );
        assert_eq!(
            display_command(&cmd("node /p/node_modules/astro/astro.mjs dev")),
            "astro dev"
        );
        assert_eq!(
            display_command(&cmd("/usr/bin/redis-server 127.0.0.1:6379")),
            "redis-server 127.0.0.1:6379"
        );
    }
}
