use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::Serialize;

const PROJECT_MARKERS: [&str; 7] = [
    "package.json",
    "Cargo.toml",
    "go.mod",
    "pyproject.toml",
    "Gemfile",
    "composer.json",
    ".git",
];

/// Locations of the system and of package managers, never a user's project,
/// though some are git checkouts, like Homebrew's prefix.
const SYSTEM_ROOTS: [&str; 15] = [
    "/System",
    "/Library",
    "/Applications",
    "/usr",
    "/bin",
    "/sbin",
    "/etc",
    "/private/etc",
    "/private/var/db",
    "/var/lib",
    "/opt/homebrew",
    "/opt/local",
    "/home/linuxbrew",
    "/nix",
    "/snap",
];

const MANIFESTS: [&str; 7] = [
    "package.json",
    "Cargo.toml",
    "go.mod",
    "pyproject.toml",
    "requirements.txt",
    "Gemfile",
    "composer.json",
];

const COMMANDS: [(&str, &str); 34] = [
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
    ("expo", "Expo"),
    ("storybook", "Storybook"),
    ("start-storybook", "Storybook"),
    ("docusaurus", "Docusaurus"),
    ("jupyter-lab", "Jupyter"),
    ("jupyter-notebook", "Jupyter"),
    ("jupyter-server", "Jupyter"),
    ("jupyterlab", "Jupyter"),
    ("mkdocs", "MkDocs"),
    ("jekyll", "Jekyll"),
    ("http.server", "http.server"),
    ("webpack-dev-server", "webpack"),
];

const IMAGES: [(&str, &str); 6] = [
    ("postgres", "PostgreSQL"),
    ("mysql", "MySQL"),
    ("mariadb", "MariaDB"),
    ("mongo", "MongoDB"),
    ("redis", "Redis"),
    ("valkey", "Valkey"),
];

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

const DEPENDENCIES: [(&str, &str); 30] = [
    ("next", "Next.js"),
    ("nuxt", "Nuxt"),
    ("@sveltejs/kit", "SvelteKit"),
    ("astro", "Astro"),
    ("@docusaurus/core", "Docusaurus"),
    ("@remix-run/dev", "Remix"),
    ("@react-router/dev", "React Router"),
    ("gatsby", "Gatsby"),
    ("expo", "Expo"),
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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Confidence {
    Confirmed,
    Likely,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "source", rename_all = "camelCase")]
pub enum Evidence {
    Marker {
        file: &'static str,
    },
    Command {
        name: &'static str,
    },
    Dependency {
        manifest: &'static str,
        name: &'static str,
    },
    Image {
        name: String,
    },
    WellKnownPort {
        port: u16,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Attribution {
    pub confidence: Confidence,
    pub evidence: Vec<Evidence>,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Detection {
    pub framework: Option<&'static str>,
    pub attribution: Attribution,
}

impl Detection {
    fn new(framework: &'static str, confidence: Confidence, evidence: Vec<Evidence>) -> Self {
        Self {
            framework: Some(framework),
            attribution: Attribution {
                confidence,
                evidence,
            },
        }
    }
}

pub fn image_detection(image: &str) -> Detection {
    image_framework(image).map_or_else(Detection::default, |framework| {
        Detection::new(
            framework,
            Confidence::Confirmed,
            vec![Evidence::Image { name: image.into() }],
        )
    })
}

pub fn port_detection(port: u16) -> Detection {
    match well_known(port) {
        Some((_, Some(framework))) => Detection::new(
            framework,
            Confidence::Likely,
            vec![Evidence::WellKnownPort { port }],
        ),
        _ => Detection::default(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Project {
    pub root: PathBuf,
    pub marker: &'static str,
}

pub fn project_folder(cwd: &Path, home: &Path) -> Option<Project> {
    let in_home = cwd.starts_with(home);
    if !in_home && SYSTEM_ROOTS.iter().any(|root| cwd.starts_with(root)) {
        return None;
    }
    cwd.ancestors()
        .take_while(|dir| {
            if in_home {
                *dir != home
            } else {
                dir.parent().is_some()
            }
        })
        .find_map(|dir| {
            // `exists` is false for a folder Ports may not read.
            let marker = PROJECT_MARKERS.iter().find(|m| dir.join(m).exists())?;
            Some(Project {
                root: dir.to_path_buf(),
                marker,
            })
        })
}

const DELETED_MARK: &str = " (deleted)";

pub fn is_deleted(cwd: &Path) -> bool {
    if cwd.to_string_lossy().ends_with(DELETED_MARK) {
        return true;
    }
    matches!(fs::symlink_metadata(cwd), Err(error) if error.kind() == std::io::ErrorKind::NotFound)
}

pub fn without_deleted_mark(cwd: &Path) -> PathBuf {
    match cwd.to_string_lossy().strip_suffix(DELETED_MARK) {
        Some(path) => PathBuf::from(path),
        None => cwd.to_path_buf(),
    }
}

pub fn canonical(path: &Path) -> PathBuf {
    path.ancestors()
        .find_map(|dir| {
            let resolved = fs::canonicalize(dir).ok()?;
            match path.strip_prefix(dir).ok()? {
                rest if rest.as_os_str().is_empty() => Some(resolved),
                rest => Some(resolved.join(rest)),
            }
        })
        .unwrap_or_else(|| path.to_path_buf())
}

pub fn command_words<'a>(commands: impl IntoIterator<Item = &'a [OsString]>) -> HashSet<String> {
    let mut words = HashSet::new();
    for command in commands {
        let Some(first) = command.first() else {
            continue;
        };
        let first = first.to_string_lossy();
        // Retitled processes read `nginx: master process …`.
        let executable =
            file_name(first.split_whitespace().next().unwrap_or_default()).trim_end_matches(':');
        words.insert(executable.to_string());
        let args: Vec<_> = command
            .iter()
            .skip(1)
            .map(|arg| arg.to_string_lossy())
            .collect();
        // A framework build of Python on macOS runs as `Python`.
        let script = if executable.to_ascii_lowercase().starts_with("python") {
            match args.as_slice() {
                [flag, module, ..] if flag == "-m" => Some(module.as_ref()),
                [script, ..] if !script.starts_with('-') => Some(script.as_ref()),
                _ => None,
            }
        } else if ["node", "bun", "deno", "ruby", "php"].contains(&executable) {
            args.first()
                .filter(|arg| !arg.starts_with('-'))
                .map(|arg| arg.as_ref())
        } else {
            None
        };
        if let Some(script) = script {
            words.extend(script_names(script).map(str::to_string));
        }
    }
    words
}

pub fn is_http_command(words: &HashSet<String>) -> bool {
    COMMANDS
        .iter()
        .any(|(word, name)| !is_database(name) && words.contains(*word))
        || RUNNERS.iter().any(|(word, ..)| words.contains(*word))
}

pub fn framework(words: &HashSet<String>, folder: Option<&Path>) -> Detection {
    if let Some((word, name)) = COMMANDS.iter().find(|(word, _)| words.contains(*word)) {
        return Detection::new(
            name,
            Confidence::Confirmed,
            vec![Evidence::Command { name: word }],
        );
    }
    let dependencies = folder.map(declared_frameworks).unwrap_or_default();
    if let Some((word, runner, refinements)) =
        RUNNERS.iter().find(|(word, ..)| words.contains(*word))
    {
        let mut evidence = vec![Evidence::Command { name: word }];
        let refinement = dependencies
            .first()
            .filter(|(framework, _)| refinements.contains(framework));
        if let Some((_, dependency)) = refinement {
            evidence.push(dependency.clone());
        }
        let name = refinement.map_or(*runner, |(framework, _)| framework);
        return Detection::new(name, Confidence::Confirmed, evidence);
    }
    match dependencies.first() {
        Some((framework, _)) => Detection::new(
            framework,
            Confidence::Likely,
            dependencies
                .into_iter()
                .map(|(_, evidence)| evidence)
                .collect(),
        ),
        None => Detection::default(),
    }
}

fn declared_frameworks(folder: &Path) -> Vec<(&'static str, Evidence)> {
    let names = dependency_names(folder);
    DEPENDENCIES
        .iter()
        .filter_map(|(dependency, framework)| {
            let manifest = names.get(*dependency)?;
            Some((
                *framework,
                Evidence::Dependency {
                    manifest,
                    name: dependency,
                },
            ))
        })
        .collect()
}

fn dependency_names(folder: &Path) -> HashMap<String, &'static str> {
    let mut names = HashMap::new();
    for manifest in MANIFESTS {
        let Ok(text) = fs::read_to_string(folder.join(manifest)) else {
            continue;
        };
        if manifest == "package.json" {
            let json: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
            for key in ["dependencies", "devDependencies"] {
                if let Some(deps) = json.get(key).and_then(|d| d.as_object()) {
                    for name in deps.keys() {
                        names.entry(name.clone()).or_insert(manifest);
                    }
                }
            }
        } else {
            for word in text
                .split(|c: char| !(c.is_alphanumeric() || "-_./@".contains(c)))
                .filter(|word| !word.is_empty())
            {
                names.entry(word.to_lowercase()).or_insert(manifest);
            }
        }
    }
    names
}

const LOCKFILES: [&str; 6] = [
    "Cargo.lock",
    "uv.lock",
    "poetry.lock",
    "Gemfile.lock",
    "composer.lock",
    "go.mod",
];

const MAX_WORKSPACE_DEPTH: usize = 4;

pub fn framework_version(
    framework: &str,
    folder: &Path,
    lockfiles: &mut Lockfiles,
) -> Option<String> {
    if is_database(framework) {
        return None;
    }
    let packages: Vec<&str> = DEPENDENCIES
        .iter()
        .filter(|(_, name)| *name == framework)
        .map(|(package, _)| *package)
        .chain(
            RUNNERS
                .iter()
                .filter(|(_, name, _)| *name == framework)
                .map(|(word, ..)| *word),
        )
        .chain(
            COMMANDS
                .iter()
                .filter(|(_, name)| *name == framework)
                .map(|(word, _)| *word),
        )
        .collect();
    let mut ancestors = Vec::new();
    for dir in folder.ancestors().take(MAX_WORKSPACE_DEPTH + 1) {
        ancestors.push(dir);
        if dir.join(".git").exists() {
            break;
        }
    }
    let from_node_modules = ancestors.iter().find_map(|dir| {
        packages.iter().find_map(|package| {
            let text =
                fs::read_to_string(dir.join("node_modules").join(package).join("package.json"))
                    .ok()?;
            package_json_string(&text, "version")
        })
    });
    from_node_modules.or_else(|| {
        ancestors.iter().find_map(|dir| {
            LOCKFILES.iter().find_map(|lockfile| {
                let versions = lockfiles.versions(&dir.join(lockfile), lockfile)?;
                packages
                    .iter()
                    .find_map(|package| versions.get(*package).cloned())
            })
        })
    })
}

#[derive(Default)]
pub struct Lockfiles {
    parsed: HashMap<PathBuf, Parsed>,
    used: HashSet<PathBuf>,
}

struct Parsed {
    modified: Option<SystemTime>,
    len: u64,
    versions: HashMap<String, String>,
}

impl Lockfiles {
    fn versions(&mut self, path: &Path, lockfile: &str) -> Option<&HashMap<String, String>> {
        let metadata = fs::metadata(path).ok()?;
        let modified = metadata.modified().ok();
        self.used.insert(path.to_path_buf());
        let fresh = self
            .parsed
            .get(path)
            .is_some_and(|parsed| parsed.modified == modified && parsed.len == metadata.len());
        if !fresh {
            let versions = fs::read_to_string(path)
                .map(|text| locked_versions(lockfile, &text))
                .unwrap_or_default();
            self.parsed.insert(
                path.to_path_buf(),
                Parsed {
                    modified,
                    len: metadata.len(),
                    versions,
                },
            );
        }
        self.parsed.get(path).map(|parsed| &parsed.versions)
    }

    pub fn finish(&mut self) {
        let used = std::mem::take(&mut self.used);
        self.parsed.retain(|path, _| used.contains(path));
    }
}

fn package_json_string(text: &str, key: &str) -> Option<String> {
    let json: serde_json::Value = serde_json::from_str(text).ok()?;
    json.get(key)?
        .as_str()
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn locked_versions(lockfile: &str, text: &str) -> HashMap<String, String> {
    match lockfile {
        // `[[package]]` tables with `name` and `version`.
        "Cargo.lock" | "uv.lock" | "poetry.lock" => text
            .parse::<toml::Table>()
            .ok()
            .and_then(|lock| lock.get("package")?.as_array().cloned())
            .unwrap_or_default()
            .iter()
            .filter_map(|package| {
                let field = |key: &str| package.get(key)?.as_str().map(str::to_string);
                Some((field("name")?, field("version")?))
            })
            .collect(),
        // `    rails (8.0.1)` among the specs; dependencies of a spec are indented further.
        "Gemfile.lock" => text
            .lines()
            .filter_map(|line| {
                let spec = line.strip_prefix("    ")?;
                let (name, version) = spec.strip_suffix(')')?.split_once(" (")?;
                (!name.starts_with(' ')).then(|| (name.to_string(), version.to_string()))
            })
            .collect(),
        "composer.lock" => {
            let lock: serde_json::Value = serde_json::from_str(text).unwrap_or_default();
            ["packages", "packages-dev"]
                .iter()
                .filter_map(|key| lock.get(key)?.as_array())
                .flatten()
                .filter_map(|package| {
                    let field = |key: &str| package.get(key)?.as_str().map(str::to_string);
                    let version = field("version")?;
                    Some((field("name")?, version.trim_start_matches('v').to_string()))
                })
                .collect()
        }
        // `require github.com/gin-gonic/gin v1.10.0`, alone or in a block.
        "go.mod" => text
            .lines()
            .filter_map(|line| {
                let mut words = line
                    .split_whitespace()
                    .skip_while(|word| *word == "require");
                let name = words.next()?;
                let version = words.next()?.strip_prefix('v')?;
                Some((name.to_string(), version.to_string()))
            })
            .collect(),
        _ => HashMap::new(),
    }
}

pub fn manifest_name(folder: &Path) -> Option<String> {
    for manifest in ["package.json", "composer.json"] {
        if let Some(name) = fs::read_to_string(folder.join(manifest))
            .ok()
            .and_then(|text| package_json_string(&text, "name"))
        {
            return Some(name);
        }
    }
    for (manifest, tables) in [
        ("Cargo.toml", &["package"][..]),
        ("pyproject.toml", &["project", "tool.poetry"][..]),
    ] {
        let Some(toml) = fs::read_to_string(folder.join(manifest))
            .ok()
            .and_then(|text| text.parse::<toml::Table>().ok())
        else {
            continue;
        };
        let name = tables.iter().find_map(|path| {
            let table = path
                .split('.')
                .try_fold(&toml, |table, key| table.get(key)?.as_table())?;
            table.get("name")?.as_str().map(str::to_string)
        });
        if name.is_some() {
            return name;
        }
    }
    None
}

const ICON_FILES: [&str; 19] = [
    "public/favicon.svg",
    "public/favicon.ico",
    "public/favicon.png",
    "static/favicon.svg",
    "static/favicon.png",
    "static/favicon.ico",
    "src/app/icon.svg",
    "src/app/icon.png",
    "src/app/favicon.ico",
    "app/icon.svg",
    "app/icon.png",
    "app/favicon.ico",
    "src/favicon.svg",
    "src/favicon.ico",
    "favicon.svg",
    "favicon.ico",
    "favicon.png",
    "assets/icon.png",
    "icon.png",
];

pub fn icon_file(folder: &Path) -> Option<PathBuf> {
    ICON_FILES
        .iter()
        .map(|file| folder.join(file))
        .find(|path| path.is_file())
}

fn file_name(word: &str) -> &str {
    word.rsplit('/').next().unwrap_or(word)
}

/// The script's name without its extension (`astro.mjs` → `astro`) and the
/// package it belongs to, so pnpm's `node …/node_modules/vite/bin/vite.js`
/// and npm's `node_modules/.bin/vite` both read as `vite`.
fn script_names(script: &str) -> impl Iterator<Item = &str> {
    let name = file_name(script);
    let stem = [".js", ".mjs", ".cjs", ".ts"]
        .iter()
        .find_map(|extension| name.strip_suffix(extension))
        .unwrap_or(name);
    let package = script
        .rsplit_once("node_modules/")
        .and_then(|(_, rest)| rest.split('/').find(|part| !part.starts_with(['@', '.'])));
    std::iter::once(stem).chain(package)
}

pub fn display_path(path: &Path, home: &Path) -> String {
    match path.strip_prefix(home) {
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}

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

    fn branch(folder: &Path, home: &Path) -> Option<String> {
        super::super::place::checkout(folder, home).and_then(|checkout| checkout.branch)
    }

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

    /// The temporary folder as working directories report it (`/private/var` on macOS).
    fn root(dir: &TempDir) -> PathBuf {
        canonical(dir.path())
    }

    fn found(root: PathBuf, marker: &'static str) -> Option<Project> {
        Some(Project { root, marker })
    }

    #[test]
    fn finds_the_nearest_project_above_the_working_directory() {
        let home = project(&[("projects/web/package.json", "{}")]);
        fs::create_dir_all(home.path().join("projects/web/src/routes")).unwrap();
        assert_eq!(
            project_folder(&root(&home).join("projects/web/src/routes"), &root(&home)),
            found(root(&home).join("projects/web"), "package.json")
        );
    }

    #[test]
    fn knows_a_desktop_app_by_its_bundle() {
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
    fn has_no_project_at_home_itself_or_in_system_locations() {
        let home = project(&[(".git/HEAD", "ref: refs/heads/dotfiles\n")]);
        assert_eq!(project_folder(&root(&home), &root(&home)), None);
        assert_eq!(project_folder(Path::new("/"), &root(&home)), None);
        assert_eq!(
            project_folder(Path::new("/opt/homebrew/var/postgresql@16"), &root(&home)),
            None
        );
        assert_eq!(
            project_folder(Path::new("/usr/local/Homebrew"), &root(&home)),
            None
        );
    }

    #[test]
    fn finds_a_project_outside_home_with_its_branch() {
        let home = TempDir::new().unwrap();
        let workspace = project(&[
            (".git/HEAD", "ref: refs/heads/release\n"),
            ("package.json", "{}"),
            ("src/main.ts", ""),
        ]);
        let folder = project_folder(&root(&workspace).join("src"), &root(&home));
        assert_eq!(folder, found(root(&workspace), "package.json"));
        assert_eq!(
            branch(&folder.unwrap().root, &root(&home)),
            Some("release".to_string())
        );
    }

    #[test]
    fn resolves_symlinked_and_deleted_working_directories() {
        let home = project(&[("web/package.json", "{}")]);
        let web = root(&home).join("web");
        std::os::unix::fs::symlink(&web, home.path().join("alias")).unwrap();
        assert_eq!(
            project_folder(&canonical(&home.path().join("alias")), &root(&home)),
            found(web.clone(), "package.json")
        );
        let deleted = home.path().join("web/.svelte-kit/output");
        fs::create_dir_all(&deleted).unwrap();
        fs::remove_dir_all(home.path().join("web/.svelte-kit")).unwrap();
        assert_eq!(
            project_folder(&canonical(&deleted), &root(&home)),
            found(web, "package.json")
        );
    }

    #[test]
    fn skips_a_folder_it_may_not_read() {
        use std::os::unix::fs::PermissionsExt;
        let home = project(&[("web/package.json", "{}"), ("web/locked/inner/x", "")]);
        let locked = home.path().join("web/locked");
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
        let folder = project_folder(&canonical(&locked.join("inner")), &root(&home));
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(folder, found(root(&home).join("web"), "package.json"));
    }

    #[test]
    fn keeps_a_monorepo_package_as_its_own_project_on_the_repository_branch() {
        let home = project(&[
            ("mono/.git/HEAD", "ref: refs/heads/feat/checkout\n"),
            ("mono/package.json", "{}"),
            ("mono/apps/web/package.json", "{}"),
            ("mono/apps/web/src/app.ts", ""),
        ]);
        let folder = project_folder(&root(&home).join("mono/apps/web/src"), &root(&home));
        assert_eq!(
            folder,
            found(root(&home).join("mono/apps/web"), "package.json")
        );
        assert_eq!(
            branch(&folder.unwrap().root, &root(&home)),
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
            project_folder(&root(&home).join("hotfix"), &root(&home)),
            found(root(&home).join("hotfix"), ".git")
        );
        assert_eq!(
            branch(&home.path().join("hotfix"), home.path()),
            Some("hotfix".to_string())
        );
    }

    #[test]
    fn shows_the_short_commit_of_a_detached_head() {
        let home = project(&[("repo/.git/HEAD", "4a1e04d7c0ffee\n")]);
        assert_eq!(
            branch(&home.path().join("repo"), home.path()),
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
            (
                "node /p/node_modules/.pnpm/vite@7.1.0/node_modules/vite/bin/vite.js dev",
                Some(&sveltekit),
                "SvelteKit",
            ),
            (
                "node /p/node_modules/.pnpm/astro@5.0.0/node_modules/astro/bin/astro.mjs dev",
                None,
                "Astro",
            ),
            ("nginx: master process /usr/sbin/nginx", None, "nginx"),
            ("node dist/server.js", Some(&express), "Express"),
            ("next-server (v15.1.0)", Some(&next), "Next.js"),
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
                framework(&words(command), folder.map(TempDir::path)).framework,
                Some(expected),
                "{command}"
            );
        }
    }

    #[test]
    fn detects_tools_by_their_own_command_only() {
        let storybook = project(&[(
            "package.json",
            r#"{"devDependencies":{"storybook":"^9","vite":"^7"}}"#,
        )]);
        let python = "/opt/homebrew/Cellar/python@3.13/3.13.0/Frameworks/Python.framework/Versions/3.13/Resources/Python.app/Contents/MacOS/Python";
        let jupyter = format!("{python} /opt/homebrew/bin/jupyter-lab --no-browser");
        let cases = [
            ("node /p/node_modules/.bin/expo start", None, Some("Expo")),
            ("node server.js --platform expo", None, None),
            (
                "node /p/node_modules/.bin/storybook dev -p 6006",
                None,
                Some("Storybook"),
            ),
            (
                "node /p/node_modules/.bin/vite",
                Some(&storybook),
                Some("Vite"),
            ),
            (
                "node /p/node_modules/.bin/docusaurus start",
                None,
                Some("Docusaurus"),
            ),
            ("node build.js docusaurus", None, None),
            (jupyter.as_str(), None, Some("Jupyter")),
            ("python3 -m jupyterlab", None, Some("Jupyter")),
            ("python3 app.py --notebook jupyter-lab", None, None),
            ("python3 /u/.venv/bin/mkdocs serve", None, Some("MkDocs")),
            ("python3 serve.py mkdocs", None, None),
            ("ruby /u/.gem/bin/jekyll serve", None, Some("Jekyll")),
            ("ruby app.rb jekyll", None, None),
            ("python3 -m http.server 8000", None, Some("http.server")),
            ("python3 server.py http.server", None, None),
            (
                "node /p/node_modules/.bin/webpack-dev-server",
                None,
                Some("webpack"),
            ),
            ("node build.js webpack-dev-server", None, None),
        ];
        for (command, folder, expected) in cases {
            assert_eq!(
                framework(&words(command), folder.map(TempDir::path)).framework,
                expected,
                "{command}"
            );
        }
    }

    #[test]
    fn explains_why_it_names_a_framework() {
        let sveltekit = project(&[(
            "package.json",
            r#"{"devDependencies":{"@sveltejs/kit":"^2","vite":"^7"}}"#,
        )]);
        assert_eq!(
            framework(
                &words("node /p/node_modules/.bin/vite dev"),
                Some(sveltekit.path())
            ),
            Detection::new(
                "SvelteKit",
                Confidence::Confirmed,
                vec![
                    Evidence::Command { name: "vite" },
                    Evidence::Dependency {
                        manifest: "package.json",
                        name: "@sveltejs/kit",
                    },
                ],
            )
        );
        let vite = project(&[("package.json", r#"{"devDependencies":{"vite":"^7"}}"#)]);
        let tcp = words("node custom-tcp.js");
        assert_eq!(
            framework(&tcp, Some(vite.path())),
            Detection::new(
                "Vite",
                Confidence::Likely,
                vec![Evidence::Dependency {
                    manifest: "package.json",
                    name: "vite",
                }],
            )
        );
        assert!(!is_http_command(&tcp));
        let both = project(&[(
            "package.json",
            r#"{"dependencies":{"fastify":"5","express":"5"}}"#,
        )]);
        assert_eq!(
            framework(&words("node server.js"), Some(both.path()))
                .attribution
                .evidence
                .len(),
            2
        );
        assert_eq!(
            framework(&words("node server.js"), None),
            Detection::default()
        );
        assert_eq!(
            image_detection("postgres").attribution,
            Attribution {
                confidence: Confidence::Confirmed,
                evidence: vec![Evidence::Image {
                    name: "postgres".into()
                }],
            }
        );
        assert_eq!(
            port_detection(5432).attribution.confidence,
            Confidence::Likely
        );
        assert_eq!(port_detection(22), Detection::default());
    }

    #[test]
    fn does_not_classify_option_values_or_unrelated_project_tools_as_servers() {
        assert_eq!(
            framework(&words("python app.py --database postgres"), None).framework,
            None
        );
        assert!(!is_http_command(&words("node custom-tcp.js")));
        assert!(!is_http_command(&words("bazel -jar A-server.jar")));
        assert!(is_http_command(&words(
            "node /p/node_modules/.pnpm/vite@7.1.0/node_modules/vite/bin/vite.js"
        )));
    }

    #[test]
    fn reads_the_installed_version_of_the_framework() {
        let mono = project(&[
            (".git/HEAD", "ref: refs/heads/main\n"),
            ("node_modules/vite/package.json", r#"{"version":"7.1.3"}"#),
            (
                "node_modules/@sveltejs/kit/package.json",
                r#"{"version":"2.21.0"}"#,
            ),
            (
                "node_modules/postgres/package.json",
                r#"{"version":"3.4.0"}"#,
            ),
            (
                "node_modules/webpack/package.json",
                r#"{"version":"5.99.0"}"#,
            ),
            (
                "node_modules/webpack-dev-server/package.json",
                r#"{"version":"5.2.0"}"#,
            ),
            ("apps/web/package.json", "{}"),
        ]);
        let web = mono.path().join("apps/web");
        assert_eq!(
            framework_version("Vite", &web, &mut Lockfiles::default()).as_deref(),
            Some("7.1.3")
        );
        assert_eq!(
            framework_version("SvelteKit", &web, &mut Lockfiles::default()).as_deref(),
            Some("2.21.0")
        );
        assert_eq!(
            framework_version("webpack", &web, &mut Lockfiles::default()).as_deref(),
            Some("5.99.0")
        );
        assert_eq!(
            framework_version("PostgreSQL", &web, &mut Lockfiles::default()),
            None
        );
        assert_eq!(
            framework_version("Next.js", &web, &mut Lockfiles::default()),
            None
        );

        let locked = project(&[
            (
                "Cargo.lock",
                "version = 4\n\n[[package]]\nname = \"axum\"\nversion = \"0.8.4\"\n",
            ),
            (
                "uv.lock",
                "version = 1\n\n[[package]]\nname = \"fastapi\"\nversion = \"0.115.12\"\n",
            ),
            (
                "Gemfile.lock",
                "GEM\n  specs:\n    actionpack (8.0.1)\n      rails-dom (~> 2.0)\n    rails (8.0.1)\n",
            ),
            (
                "composer.lock",
                r#"{"packages":[{"name":"laravel/framework","version":"v11.9.2"}]}"#,
            ),
            (
                "go.mod",
                "module app\n\nrequire (\n\tgithub.com/gin-gonic/gin v1.10.0\n)\n",
            ),
        ]);
        for (framework, expected) in [
            ("Axum", "0.8.4"),
            ("FastAPI", "0.115.12"),
            ("Rails", "8.0.1"),
            ("Laravel", "11.9.2"),
            ("Gin", "1.10.0"),
        ] {
            assert_eq!(
                framework_version(framework, locked.path(), &mut Lockfiles::default()).as_deref(),
                Some(expected),
                "{framework}"
            );
        }

        fs::write(
            mono.path().join("node_modules/vite/package.json"),
            r#"{"version":"7.2.0"}"#,
        )
        .unwrap();
        assert_eq!(
            framework_version("Vite", &web, &mut Lockfiles::default()).as_deref(),
            Some("7.2.0")
        );
    }

    #[test]
    fn reads_the_name_the_manifest_gives_the_project() {
        let cases = [
            (
                "package.json",
                r#"{"name":"@acme/storefront"}"#,
                Some("@acme/storefront"),
            ),
            ("Cargo.toml", "[package]\nname = \"ports\"\n", Some("ports")),
            (
                "pyproject.toml",
                "[project]\nname = \"billing-api\"\n",
                Some("billing-api"),
            ),
            (
                "pyproject.toml",
                "[tool.poetry]\nname = \"legacy\"\n",
                Some("legacy"),
            ),
            ("package.json", r#"{"private":true}"#, None),
        ];
        for (manifest, contents, expected) in cases {
            let dir = project(&[(manifest, contents)]);
            assert_eq!(manifest_name(dir.path()).as_deref(), expected, "{contents}");
        }
    }

    #[test]
    fn finds_the_project_icon_where_its_framework_keeps_it() {
        let sveltekit = project(&[("static/favicon.png", "png")]);
        assert_eq!(
            icon_file(sveltekit.path()),
            Some(sveltekit.path().join("static/favicon.png"))
        );
        let vite = project(&[("public/favicon.svg", "svg"), ("favicon.ico", "ico")]);
        assert_eq!(
            icon_file(vite.path()),
            Some(vite.path().join("public/favicon.svg"))
        );
        assert_eq!(icon_file(project(&[("README.md", "")]).path()), None);
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

    #[test]
    fn tells_a_deleted_working_folder_from_one_it_cannot_read() {
        let dir = tempfile::tempdir().unwrap();
        let worktree = dir.path().join("worktree");
        fs::create_dir(&worktree).unwrap();
        assert!(!is_deleted(&worktree));
        fs::remove_dir(&worktree).unwrap();
        assert!(is_deleted(&worktree));
        assert!(is_deleted(Path::new("/proc/1/cwd/gone (deleted)")));
        assert_eq!(
            without_deleted_mark(Path::new("/srv/app (deleted)")),
            PathBuf::from("/srv/app")
        );
    }

    #[cfg(unix)]
    #[test]
    fn does_not_call_an_unreadable_folder_deleted() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let locked = dir.path().join("locked");
        let inside = locked.join("project");
        fs::create_dir_all(&inside).unwrap();
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
        let readable = fs::symlink_metadata(&inside).is_ok();
        let deleted = is_deleted(&inside);
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(readable || !deleted);
    }
}
