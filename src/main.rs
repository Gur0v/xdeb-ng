use std::collections::{BTreeSet, HashMap};
use std::env;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};

const SHLIBS_URL: &str =
    "https://raw.githubusercontent.com/void-linux/void-packages/master/common/shlibs";

#[derive(Debug)]
struct Config {
    pkgroot: PathBuf,
    workdir: PathBuf,
    destdir: PathBuf,
    datadir: PathBuf,
    binpkgs: PathBuf,
    shlibs: PathBuf,
    opt_deps: bool,
    opt_quit: bool,
    opt_extract: bool,
    opt_install: bool,
    opt_register: bool,
    opt_clean_dir: bool,
    opt_lazy_soname: bool,
    opt_fix_conflict: bool,
    opt_warn_conflict: bool,
    suffix: String,
    dependencies: Vec<String>,
    not_dependencies: Vec<String>,
    conflicts: Vec<String>,
    replaces: Vec<String>,
    provides: Vec<String>,
    arch: Option<String>,
    name: Option<String>,
    version: Option<String>,
    revision: Option<String>,
    post_extract: Option<PathBuf>,
    basepkg: Option<PathBuf>,
    refused: bool,
    dry_run: bool,
    explain_deps: bool,
    dep_explanations: Vec<String>,
    json: bool,
    keep_workdir: bool,
    missing_deps_error: bool,
    conflict_mode: ConflictMode,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ConflictMode {
    Warn,
    Error,
    Ignore,
    Auto,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum HostLibc {
    Glibc,
    Musl,
    Unknown,
}

#[derive(Debug)]
struct PackageMeta {
    name: String,
    version: String,
    revision: String,
    arch: String,
    license: String,
    maintainer: String,
    short_desc: String,
    long_desc: String,
}

fn main() {
    let args: Vec<OsString> = env::args_os().collect();
    let result = match args.get(1).and_then(|a| a.to_str()) {
        Some("install") | Some("file") => install_command(&args[2..]),
        Some("convert") => {
            let mut cfg = Config::from_env();
            parse_args_from(&mut cfg, &args[2..]).and_then(|_| run_config(&mut cfg))
        }
        Some("info") => info_command(&args[2..]),
        Some("clean") => clean_command(&args[2..]),
        Some("doctor") => doctor_command(),
        _ => {
            let mut cfg = Config::from_env();
            run(&mut cfg)
        }
    };
    if let Err(err) = result {
        log_crit(&err);
        std::process::exit(1);
    }
}

fn run(cfg: &mut Config) -> Result<(), String> {
    parse_args(cfg)?;
    run_config(cfg)
}

fn run_config(cfg: &mut Config) -> Result<(), String> {
    for (cmd, pkg) in [
        ("xz", "xz"),
        ("tar", "tar"),
        ("curl", "curl"),
        ("ar", "binutils"),
        ("objdump", "binutils"),
    ] {
        check_command(cmd, pkg)?;
    }

    clean(cfg)?;
    fs::create_dir_all(&cfg.pkgroot)
        .map_err(|e| format!("Unable to create {}: {e}", cfg.pkgroot.display()))?;
    fs::create_dir_all(&cfg.binpkgs)
        .map_err(|e| format!("Unable to create {}: {e}", cfg.binpkgs.display()))?;
    fs::create_dir_all(&cfg.datadir)
        .map_err(|e| format!("Unable to create {}: {e}", cfg.datadir.display()))?;
    fs::create_dir_all(&cfg.destdir)
        .map_err(|e| format!("Unable to create {}: {e}", cfg.destdir.display()))?;
    fs::create_dir_all(&cfg.workdir)
        .map_err(|e| format!("Unable to create {}: {e}", cfg.workdir.display()))?;

    if cfg.opt_deps && !cfg.shlibs.is_file() {
        return Err("shlibs file not synced. Run xdeb with '-Sd' to fetch dependency file".into());
    }
    if cfg.opt_deps && shlibs_is_stale(&cfg.shlibs, 30) {
        log_warn("shlibs cache is older than 30 days or has no metadata; consider syncing with -S");
    }

    if cfg.opt_extract {
        extract_deb(cfg)?;
        log_info("Extracted files");
    }

    if cfg.opt_quit {
        log_info("Quitting before building");
        return Ok(());
    }

    let mut meta = parse_metadata(cfg)?;

    if cfg.opt_fix_conflict {
        fix_known_conflicts(cfg)?;
    }

    if let Some(script) = &cfg.post_extract {
        run_status(
            Command::new("sh")
                .arg("-c")
                .arg(format!(". {}", shell_quote(script)))
                .current_dir(&cfg.destdir),
            "post-extract commands failed",
        )?;
    }

    if cfg.opt_deps {
        let (deps, explanations) = gen_rdeps(cfg)?;
        cfg.dependencies.extend(deps);
        cfg.dep_explanations = explanations;
    }
    if cfg.missing_deps_error
        && cfg
            .dep_explanations
            .iter()
            .any(|e| e.ends_with("-> unresolved"))
    {
        return Err("unresolved shared library dependencies found".into());
    }
    cfg.dependencies.sort();
    cfg.dependencies.dedup();
    if !cfg.dependencies.is_empty() {
        log_info(&format!(
            "Resolved dependencies ({})",
            cfg.dependencies.join(" ")
        ));
    }
    if cfg.explain_deps {
        for explanation in &cfg.dep_explanations {
            log_info(explanation);
        }
    }

    if cfg.dry_run {
        print_package_preview(cfg, &meta, cfg.json);
        log_info("Dry run: not creating or installing package");
        return Ok(());
    }

    if cfg.opt_clean_dir || cfg.opt_warn_conflict {
        check_tree(cfg, &meta.name)?;
    }

    build_package(cfg, &mut meta)?;
    Ok(())
}

impl Config {
    fn from_env() -> Self {
        let pkgroot = env_path(
            "XDEB_PKGROOT",
            env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        );
        let workdir = env_path("XDEB_WORKDIR", pkgroot.join("workdir"));
        let destdir = env_path("XDEB_DESTDIR", pkgroot.join("destdir"));
        let datadir = env_path("XDEB_DATADIR", pkgroot.join("datadir"));
        let binpkgs = env_path("XDEB_BINPKGS", pkgroot.join("binpkgs"));
        let shlibs = env_path("XDEB_SHLIBS", pkgroot.join("shlibs"));

        Self {
            pkgroot,
            workdir,
            destdir,
            datadir,
            binpkgs,
            shlibs,
            opt_deps: env_bool("XDEB_OPT_DEPS", false),
            opt_quit: env_bool("XDEB_OPT_QUIT", false),
            opt_extract: env_bool("XDEB_OPT_EXTRACT", true),
            opt_install: env_bool("XDEB_OPT_INSTALL", false),
            opt_register: env_bool("XDEB_OPT_REGISTER", true),
            opt_clean_dir: env_bool("XDEB_OPT_CLEAN_DIR", false),
            opt_lazy_soname: env_bool("XDEB_OPT_LAZY_SONAME", false),
            opt_fix_conflict: env_bool("XDEB_OPT_FIX_CONFLICT", true),
            opt_warn_conflict: env_bool("XDEB_OPT_WARN_CONFLICT", true),
            suffix: String::new(),
            dependencies: split_env_words("XDEB_DEPENDENCIES"),
            not_dependencies: split_env_words("XDEB_NOT_DEPENDENCIES"),
            conflicts: split_env_words("XDEB_CONFLICTS"),
            replaces: split_env_words("XDEB_REPLACES"),
            provides: split_env_words("XDEB_PROVIDES"),
            arch: None,
            name: None,
            version: None,
            revision: None,
            post_extract: env::var_os("XDEB_OPT_POST_EXTRACT").map(PathBuf::from),
            basepkg: None,
            refused: false,
            dry_run: false,
            explain_deps: false,
            dep_explanations: Vec::new(),
            json: false,
            keep_workdir: false,
            missing_deps_error: false,
            conflict_mode: ConflictMode::Error,
        }
    }

    fn for_pkgroot(pkgroot: PathBuf) -> Self {
        let pkgroot = normalize_path(pkgroot);
        Self {
            workdir: pkgroot.join("workdir"),
            destdir: pkgroot.join("destdir"),
            datadir: pkgroot.join("datadir"),
            binpkgs: pkgroot.join("binpkgs"),
            shlibs: pkgroot.join("shlibs"),
            pkgroot,
            opt_deps: false,
            opt_quit: false,
            opt_extract: true,
            opt_install: false,
            opt_register: true,
            opt_clean_dir: false,
            opt_lazy_soname: false,
            opt_fix_conflict: true,
            opt_warn_conflict: true,
            suffix: String::new(),
            dependencies: Vec::new(),
            not_dependencies: Vec::new(),
            conflicts: Vec::new(),
            replaces: Vec::new(),
            provides: Vec::new(),
            arch: None,
            name: None,
            version: None,
            revision: None,
            post_extract: None,
            basepkg: None,
            refused: false,
            dry_run: false,
            explain_deps: false,
            dep_explanations: Vec::new(),
            json: false,
            keep_workdir: false,
            missing_deps_error: false,
            conflict_mode: ConflictMode::Error,
        }
    }
}

fn parse_args(cfg: &mut Config) -> Result<(), String> {
    let args: Vec<OsString> = env::args_os().skip(1).collect();
    parse_args_from(cfg, &args)
}

fn parse_args_from(cfg: &mut Config, args: &[OsString]) -> Result<(), String> {
    let mut i = 0;
    while i < args.len() {
        let arg = args[i].to_string_lossy();
        match arg.as_ref() {
            "--deps" | "--arch" | "--name" | "--version" | "--not-deps" | "--revision"
            | "--rev" => {
                return Err(format!("'{arg}' invalid. Use {arg}=... instead"));
            }
            "--help" => usage(0),
            "--dry-run" => cfg.dry_run = true,
            "--explain-deps" => cfg.explain_deps = true,
            "--json" => cfg.json = true,
            "--keep-workdir" => cfg.keep_workdir = true,
            "--" => {
                i += 1;
                break;
            }
            _ if arg.starts_with("--deps=") => cfg.dependencies.extend(split_words(&arg[7..])),
            _ if arg.starts_with("--conflicts=") => cfg.conflicts.extend(split_words(&arg[12..])),
            _ if arg.starts_with("--replaces=") => cfg.replaces.extend(split_words(&arg[11..])),
            _ if arg.starts_with("--provides=") => cfg.provides.extend(split_words(&arg[11..])),
            _ if arg.starts_with("--arch=") => cfg.arch = Some(arg[7..].to_string()),
            _ if arg.starts_with("--name=") => cfg.name = Some(arg[7..].to_string()),
            _ if arg.starts_with("--version=") => cfg.version = Some(arg[10..].to_string()),
            _ if arg.starts_with("--revision=") => cfg.revision = Some(arg[11..].to_string()),
            _ if arg.starts_with("--rev=") => cfg.revision = Some(arg[6..].to_string()),
            _ if arg.starts_with("--not-deps=") => {
                cfg.not_dependencies.extend(split_words(&arg[11..]))
            }
            _ if arg.starts_with("--missing-deps=") => match &arg[15..] {
                "warn" => cfg.missing_deps_error = false,
                "error" => cfg.missing_deps_error = true,
                other => return Err(format!("invalid --missing-deps value '{other}'")),
            },
            _ if arg.starts_with("--file-conflicts=") => {
                cfg.conflict_mode = parse_conflict_mode(&arg[17..])?;
                cfg.opt_warn_conflict = cfg.conflict_mode != ConflictMode::Ignore;
                cfg.opt_fix_conflict = cfg.conflict_mode == ConflictMode::Auto;
            }
            _ if arg.starts_with("--post-extract=") => {
                cfg.post_extract = Some(PathBuf::from(arg[15..].to_string()))
            }
            _ if arg.starts_with("--") => {
                log_crit(&format!("invalid option '{arg}'"));
                usage(1);
            }
            _ if arg.starts_with('-') && arg.len() > 1 => {
                for ch in arg[1..].chars() {
                    match ch {
                        'S' => sync_shlibs(cfg)?,
                        'd' => cfg.opt_deps = true,
                        'h' => usage(0),
                        'c' => clean(cfg)?,
                        'C' => clean_all(cfg)?,
                        'r' => clean_repodata(cfg)?,
                        'R' => cfg.opt_register = false,
                        'q' => cfg.opt_quit = true,
                        'Q' => std::process::exit(0),
                        'b' => cfg.opt_extract = false,
                        'e' => cfg.opt_clean_dir = true,
                        'm' => cfg.suffix = "-32bit".into(),
                        'i' => {
                            cfg.opt_warn_conflict = false;
                            cfg.conflict_mode = ConflictMode::Ignore;
                        }
                        'f' => log_warn(
                            "Option '-f' is now enabled by default. Use '-F' to disable it.",
                        ),
                        'F' => cfg.opt_fix_conflict = false,
                        'I' => cfg.opt_install = true,
                        'L' => cfg.opt_lazy_soname = true,
                        other => {
                            log_crit(&format!("invalid option -- '{other}'"));
                            usage(1);
                        }
                    }
                }
            }
            _ => break,
        }
        i += 1;
    }

    if i < args.len() {
        cfg.basepkg = Some(PathBuf::from(args[i].clone()));
    }
    Ok(())
}

fn usage(code: i32) -> ! {
    println!("usage: xdeb [-S] [-d] [-Sd] [--deps] ... FILE");
    println!("       xdeb convert [-S] [-d] [--dry-run] [--explain-deps] FILE");
    println!("       xdeb info FILE");
    println!("       xdeb clean [--all|--repo]");
    println!("       xdeb install [--file FILE_OR_URL] [--options XDEB_OPTS] [--not-deps PKGS] [FILE_OR_URL]");
    println!("  -d                          Automatic dependency resolution");
    println!("  -S                          Download shlibs file for automatic dependencies");
    println!("  -c                          Like -C, excluding shlibs and binpkgs");
    println!("  -r                          Remove repodata file (Use for re-building)");
    println!("  -R                          Do not register package in repository pool.");
    println!("  -q                          Extract .deb into destdir only, do not build");
    println!("  -C                          Remove all files created by this script");
    println!("  -b                          Build from destdir directly without a .deb file");
    println!("  -e                          Remove empty directories from the package");
    println!("  -m                          Add the -32bit suffix to the package name");
    println!("  -i                          Don't warn if package could break the system");
    println!("  -f                          Try to fix certain file conflicts (deprecated)");
    println!("  -F                          Don't try to fix certain file conflicts");
    println!("  -I                          Automatically install the package");
    println!("  -L                          Lazy match SONAMEs");
    println!("  --deps=...                  Packages that shall be added as dependencies");
    println!("  --not-deps=...              Packages that shall not be used as dependencies");
    println!("  --arch=...                  Package arch");
    println!("  --name=...                  Package name");
    println!("  --version=...               Package version");
    println!("  --revision=... --rev=...    Package revision");
    println!("  --post-extract=...          File with post-extract commands (i.e. /dev/stdin)");
    println!("  --dry-run                   Resolve and preview without building/installing");
    println!("  --explain-deps              Show SONAME to XBPS dependency mapping");
    println!("  --json                      Print machine-readable preview output");
    println!("  --keep-workdir              Keep temporary install workdir for debugging");
    println!("  --missing-deps=warn|error   Choose whether unresolved SONAMEs fail conversion");
    println!("  --file-conflicts=warn|error|ignore|auto");
    println!("  --help | -h                 Show help page");
    println!();
    println!("example:");
    println!("  xdeb -Cq                    Remove all files and quit");
    println!("  xdeb -Sd FILE               Sync depdendency list and create package");
    println!("  xdeb info FILE.deb          Show package metadata preview");
    println!("  xdeb install FILE.deb       Convert and install a local DEB package");
    println!("  xdeb --deps='tar>0' FILE    Add tar as manual dependency and create package");
    std::process::exit(code);
}

#[derive(Debug)]
struct InstallOptions {
    source: Option<String>,
    xdeb_options: String,
    temp: PathBuf,
    dependencies: Vec<String>,
    not_dependencies: Vec<String>,
    dry_run: bool,
    explain_deps: bool,
    json: bool,
    keep_workdir: bool,
    yes: bool,
    missing_deps_error: bool,
}

fn install_command(args: &[OsString]) -> Result<(), String> {
    let opts = parse_install_args(args)?;
    let source = opts.source.ok_or("no package provided to install")?;
    let is_url = source.starts_with("http://") || source.starts_with("https://");
    if !is_url && !source.ends_with(".deb") {
        return Err(format!("file '{source}' is not a valid DEB package"));
    }

    check_command("curl", "curl")?;
    check_command("xbps-install", "xbps")?;

    let source_name = Path::new(&source)
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or("package.deb");
    let package_name = trim_extensions(source_name, 1);
    let workdir = normalize_path(opts.temp.join(&package_name));
    let deb_path = workdir.join(format!("{package_name}.deb"));

    remove_path(&workdir)?;
    fs::create_dir_all(&workdir)
        .map_err(|e| format!("Unable to create {}: {e}", workdir.display()))?;

    if is_url {
        log_info(&format!("Installing {package_name} from {source}"));
        run_status(
            Command::new("curl")
                .arg("-L")
                .arg("-f")
                .arg("-o")
                .arg(&deb_path)
                .arg(&source),
            "could not download DEB package",
        )?;
    } else {
        let source_path = normalize_path(PathBuf::from(&source));
        if !source_path.is_file() {
            return Err(format!("file '{}' does not exist", source_path.display()));
        }
        log_info(&format!(
            "Installing {package_name} from {}",
            source_path.display()
        ));
        fs::copy(&source_path, &deb_path).map_err(|e| {
            format!(
                "Unable to copy {} to {}: {e}",
                source_path.display(),
                deb_path.display()
            )
        })?;
    }

    let mut cfg = Config::for_pkgroot(workdir.clone());
    cfg.basepkg = Some(deb_path);
    cfg.dependencies = opts.dependencies;
    cfg.not_dependencies = opts.not_dependencies;
    cfg.dry_run = opts.dry_run;
    cfg.explain_deps = opts.explain_deps;
    cfg.json = opts.json;
    cfg.keep_workdir = opts.keep_workdir;
    cfg.missing_deps_error = opts.missing_deps_error;
    apply_xdeb_options(&mut cfg, &opts.xdeb_options)?;
    cfg.opt_install = false;
    run_config(&mut cfg)?;

    if opts.dry_run {
        return Ok(());
    }

    install_built_package(&workdir, opts.yes)?;
    if opts.keep_workdir {
        log_info(&format!("Keeping workdir {}", workdir.display()));
    } else {
        remove_path(&workdir)?;
    }
    Ok(())
}

fn parse_install_args(args: &[OsString]) -> Result<InstallOptions, String> {
    let mut opts = InstallOptions {
        source: None,
        xdeb_options: "-Sde".into(),
        temp: env::temp_dir().join("xdeb"),
        dependencies: Vec::new(),
        not_dependencies: Vec::new(),
        dry_run: false,
        explain_deps: false,
        json: false,
        keep_workdir: false,
        yes: false,
        missing_deps_error: false,
    };
    let mut i = 0;
    while i < args.len() {
        let arg = args[i].to_string_lossy();
        match arg.as_ref() {
            "--help" | "-h" => install_usage(0),
            "--dry-run" => opts.dry_run = true,
            "--explain-deps" => opts.explain_deps = true,
            "--json" => opts.json = true,
            "--keep-workdir" => opts.keep_workdir = true,
            "--yes" | "-y" => opts.yes = true,
            "--file" | "-f" => {
                i += 1;
                opts.source = args.get(i).map(|s| s.to_string_lossy().to_string());
                if opts.source.is_none() {
                    return Err(format!("{arg} requires a value"));
                }
            }
            "--options" | "-o" => {
                i += 1;
                opts.xdeb_options = args
                    .get(i)
                    .map(|s| s.to_string_lossy().to_string())
                    .ok_or_else(|| format!("{arg} requires a value"))?;
            }
            "--temp" | "-t" => {
                i += 1;
                opts.temp = args
                    .get(i)
                    .map(PathBuf::from)
                    .ok_or_else(|| format!("{arg} requires a value"))?;
            }
            "--deps" => {
                i += 1;
                let value = args
                    .get(i)
                    .map(|s| s.to_string_lossy().to_string())
                    .ok_or_else(|| format!("{arg} requires a value"))?;
                opts.dependencies.extend(split_words(&value));
            }
            "--not-deps" => {
                i += 1;
                let value = args
                    .get(i)
                    .map(|s| s.to_string_lossy().to_string())
                    .ok_or_else(|| format!("{arg} requires a value"))?;
                opts.not_dependencies.extend(split_words(&value));
            }
            _ if arg.starts_with("--file=") => opts.source = Some(arg[7..].to_string()),
            _ if arg.starts_with("--options=") => opts.xdeb_options = arg[10..].to_string(),
            _ if arg.starts_with("--temp=") => opts.temp = PathBuf::from(arg[7..].to_string()),
            _ if arg.starts_with("--deps=") => opts.dependencies.extend(split_words(&arg[7..])),
            _ if arg.starts_with("--not-deps=") => {
                opts.not_dependencies.extend(split_words(&arg[11..]))
            }
            _ if arg.starts_with("--missing-deps=") => match &arg[15..] {
                "warn" => opts.missing_deps_error = false,
                "error" => opts.missing_deps_error = true,
                other => return Err(format!("invalid --missing-deps value '{other}'")),
            },
            _ if arg.starts_with('-') => return Err(format!("invalid install option '{arg}'")),
            _ => opts.source = Some(arg.to_string()),
        }
        i += 1;
    }
    Ok(opts)
}

fn install_usage(code: i32) -> ! {
    println!(
        "usage: xdeb install [--file FILE_OR_URL] [--options XDEB_OPTS] [--not-deps PKGS] [FILE_OR_URL]"
    );
    println!("  --file, -f       install a package from a local DEB file or remote URL");
    println!("  --options, -o    xdeb conversion options (default: -Sde)");
    println!("  --temp, -t       temporary xdeb context root path (default: /tmp/xdeb)");
    println!("  --deps           manual dependencies to add");
    println!("  --not-deps       dependencies to exclude from auto resolution");
    println!("  --dry-run        convert and preview without installing");
    println!("  --explain-deps   show SONAME to XBPS dependency mapping");
    println!("  --json           print machine-readable preview output");
    println!("  --keep-workdir   keep temporary workdir after install");
    println!("  --yes, -y        skip install confirmation");
    println!("  --missing-deps=warn|error");
    println!("  --help, -h       show this help page");
    std::process::exit(code);
}

fn apply_xdeb_options(cfg: &mut Config, options: &str) -> Result<(), String> {
    let flags = options.trim_start_matches('-');
    for ch in flags.chars() {
        match ch {
            'S' => sync_shlibs(cfg)?,
            'd' => cfg.opt_deps = true,
            'e' => cfg.opt_clean_dir = true,
            'm' => cfg.suffix = "-32bit".into(),
            'i' => {}
            'f' => {}
            'F' => cfg.opt_fix_conflict = false,
            'L' => cfg.opt_lazy_soname = true,
            'R' => cfg.opt_register = false,
            'q' | 'Q' | 'b' | 'c' | 'C' | 'r' | 'I' | 'h' => {
                return Err(format!("install mode does not support xdeb option '-{ch}'"));
            }
            '-' => {}
            other => return Err(format!("invalid xdeb option '-{other}'")),
        }
    }
    Ok(())
}

fn install_built_package(workdir: &Path, yes: bool) -> Result<(), String> {
    let binpkgs = workdir.join("binpkgs");
    let mut packages = fs::read_dir(&binpkgs)
        .map_err(|e| {
            format!(
                "could not read XBPS packages within '{}': {e}",
                binpkgs.display()
            )
        })?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(OsStr::to_str) == Some("xbps"))
        .collect::<Vec<_>>();
    packages.sort();
    let package = packages.first().ok_or_else(|| {
        format!(
            "could not find any XBPS packages to install within '{}'",
            binpkgs.display()
        )
    })?;
    let package_name = package
        .file_name()
        .and_then(OsStr::to_str)
        .map(|s| trim_extensions(s, 2))
        .ok_or("invalid XBPS package filename")?;

    if !yes
        && !confirm(&format!(
            "Install {package_name} from {}? [y/N] ",
            binpkgs.display()
        ))?
    {
        return Err("installation cancelled".into());
    }

    let mut cmd = if current_uid() == 0 {
        Command::new("xbps-install")
    } else {
        let helper = privilege_helper()?;
        let mut cmd = Command::new(helper);
        cmd.arg("xbps-install");
        cmd
    };
    run_status(
        cmd.current_dir(workdir)
            .arg("-R")
            .arg("binpkgs")
            .arg(package_name),
        "xbps-install failed",
    )
}

fn info_command(args: &[OsString]) -> Result<(), String> {
    let mut show_deps = false;
    let mut explain_deps = false;
    let mut json = false;
    let mut source: Option<PathBuf> = None;
    let mut i = 0;
    while i < args.len() {
        let arg = args[i].to_string_lossy();
        match arg.as_ref() {
            "--help" | "-h" => info_usage(0),
            "--deps" => show_deps = true,
            "--json" => json = true,
            "--explain-deps" => {
                show_deps = true;
                explain_deps = true;
            }
            _ if arg.starts_with('-') => return Err(format!("invalid info option '{arg}'")),
            _ => source = Some(PathBuf::from(arg.to_string())),
        }
        i += 1;
    }

    let source = source.ok_or("no package provided to inspect")?;
    let root = env::temp_dir().join(format!("xdeb-info-{}", std::process::id()));
    remove_path(&root)?;
    let mut cfg = Config::for_pkgroot(root.clone());
    cfg.basepkg = Some(source);
    cfg.opt_deps = show_deps;
    cfg.explain_deps = explain_deps;
    fs::create_dir_all(&cfg.datadir)
        .map_err(|e| format!("Unable to create {}: {e}", cfg.datadir.display()))?;
    fs::create_dir_all(&cfg.destdir)
        .map_err(|e| format!("Unable to create {}: {e}", cfg.destdir.display()))?;
    fs::create_dir_all(&cfg.workdir)
        .map_err(|e| format!("Unable to create {}: {e}", cfg.workdir.display()))?;
    extract_deb(&cfg)?;
    let meta = parse_metadata(&cfg)?;
    if show_deps {
        if !cfg.shlibs.is_file() {
            sync_shlibs(&cfg)?;
        }
        let (deps, explanations) = gen_rdeps(&cfg)?;
        cfg.dependencies = deps;
        cfg.dep_explanations = explanations;
    }
    print_package_preview(&cfg, &meta, json);
    if explain_deps {
        for explanation in &cfg.dep_explanations {
            println!("dep: {explanation}");
        }
    }
    remove_path(&root)?;
    Ok(())
}

fn clean_command(args: &[OsString]) -> Result<(), String> {
    let cfg = Config::from_env();
    let mut all = false;
    let mut repo = false;
    for arg in args {
        match arg.to_string_lossy().as_ref() {
            "--help" | "-h" => clean_usage(0),
            "--all" | "-a" => all = true,
            "--repo" | "--repodata" | "-r" => repo = true,
            other => return Err(format!("invalid clean option '{other}'")),
        }
    }
    if all {
        clean_all(&cfg)
    } else if repo {
        clean_repodata(&cfg)
    } else {
        clean(&cfg)
    }
}

fn doctor_command() -> Result<(), String> {
    let checks = [
        ("xz", "xz"),
        ("tar", "tar"),
        ("curl", "curl"),
        ("ar", "binutils"),
        ("objdump", "binutils"),
        ("xbps-create", "xbps"),
        ("xbps-rindex", "xbps"),
        ("xbps-install", "xbps"),
    ];
    let mut ok = true;
    for (cmd, pkg) in checks {
        if command_exists(cmd) {
            println!("ok: {cmd}");
        } else {
            println!("missing: {cmd} from package {pkg}");
            ok = false;
        }
    }
    println!("host-libc: {:?}", detect_host_libc());
    let cfg = Config::from_env();
    if cfg.shlibs.exists() {
        println!("shlibs: {}", cfg.shlibs.display());
        if shlibs_is_stale(&cfg.shlibs, 30) {
            println!("warning: shlibs cache is older than 30 days or has no metadata");
        }
    } else {
        println!("missing: shlibs cache ({})", cfg.shlibs.display());
    }
    if ok {
        Ok(())
    } else {
        Err("doctor found missing required tools".into())
    }
}

fn info_usage(code: i32) -> ! {
    println!("usage: xdeb info [--deps] [--explain-deps] [--json] FILE.deb");
    println!("  --deps          resolve and display automatic dependencies");
    println!("  --explain-deps  show SONAME to XBPS dependency mapping");
    println!("  --json          print machine-readable output");
    std::process::exit(code);
}

fn clean_usage(code: i32) -> ! {
    println!("usage: xdeb clean [--all|--repo]");
    println!("  --all, -a   remove workdir, destdir, datadir, binpkgs, shlibs");
    println!("  --repo, -r  remove repository metadata only");
    std::process::exit(code);
}

fn print_package_preview(cfg: &Config, meta: &PackageMeta, json: bool) {
    if json {
        println!("{{");
        println!("  \"name\": \"{}\",", json_escape(&meta.name));
        println!("  \"version\": \"{}\",", json_escape(&meta.version));
        println!("  \"revision\": \"{}\",", json_escape(&meta.revision));
        println!("  \"arch\": \"{}\",", json_escape(&meta.arch));
        println!("  \"maintainer\": \"{}\",", json_escape(&meta.maintainer));
        println!("  \"license\": \"{}\",", json_escape(&meta.license));
        println!("  \"short_desc\": \"{}\",", json_escape(&meta.short_desc));
        println!("  \"long_desc\": \"{}\",", json_escape(&meta.long_desc));
        println!(
            "  \"dependencies\": [{}],",
            cfg.dependencies
                .iter()
                .map(|d| format!("\"{}\"", json_escape(d)))
                .collect::<Vec<_>>()
                .join(", ")
        );
        println!(
            "  \"output\": \"{}-{}_{}.{}.xbps\"",
            json_escape(&meta.name),
            json_escape(&meta.version),
            json_escape(&meta.revision),
            json_escape(&meta.arch)
        );
        println!("}}");
        return;
    }
    println!("name: {}", meta.name);
    println!("version: {}", meta.version);
    println!("revision: {}", meta.revision);
    println!("arch: {}", meta.arch);
    println!("maintainer: {}", meta.maintainer);
    println!("license: {}", meta.license);
    println!("short_desc: {}", meta.short_desc);
    println!("long_desc: {}", meta.long_desc.replace('\n', "\\n"));
    println!("dependencies: {}", cfg.dependencies.join(" "));
    println!(
        "output: {}-{}_{}.{}.xbps",
        meta.name, meta.version, meta.revision, meta.arch
    );
}

fn sync_shlibs(cfg: &Config) -> Result<(), String> {
    if let Some(parent) = cfg.shlibs.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Unable to create {}: {e}", parent.display()))?;
    }
    run_status(
        Command::new("curl")
            .arg("-s")
            .arg(SHLIBS_URL)
            .arg("-o")
            .arg(&cfg.shlibs)
            .arg("-f"),
        "Unable to sync shlibs.",
    )?;
    let meta = format!("url={SHLIBS_URL}\nfetched_unix={}\n", unix_time());
    fs::write(cfg.shlibs.with_extension("meta"), meta)
        .map_err(|e| format!("Unable to write shlibs metadata: {e}"))?;
    log_info("Synced shlibs");
    Ok(())
}

fn clean(cfg: &Config) -> Result<(), String> {
    remove_path(&cfg.workdir)?;
    remove_path(&cfg.datadir)?;
    remove_path(&cfg.destdir)?;
    Ok(())
}

fn clean_repodata(cfg: &Config) -> Result<(), String> {
    if let Ok(entries) = fs::read_dir(&cfg.binpkgs) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path
                .file_name()
                .and_then(OsStr::to_str)
                .is_some_and(|n| n.ends_with("-repodata"))
            {
                remove_path(&path)?;
            }
        }
    }
    Ok(())
}

fn clean_all(cfg: &Config) -> Result<(), String> {
    clean(cfg)?;
    clean_repodata(cfg)?;
    remove_path(&cfg.binpkgs)?;
    remove_path(&cfg.shlibs)?;
    Ok(())
}

fn extract_deb(cfg: &Config) -> Result<(), String> {
    let basepkg = cfg
        .basepkg
        .as_ref()
        .ok_or("Last argument is not a .deb file or does not exist")?;
    if !basepkg.is_file() {
        return Err("Last argument is not a .deb file or does not exist".into());
    }
    run_status(
        Command::new("ar")
            .arg("-xf")
            .arg(basepkg)
            .arg("--output")
            .arg(&cfg.workdir),
        "Not a valid deb file, ar extraction failed",
    )?;
    let control = first_match(&cfg.workdir, "control.")?;
    let data = first_match(&cfg.workdir, "data.")?;
    validate_tar_archive(&control)?;
    validate_tar_archive(&data)?;
    run_status(
        Command::new("tar")
            .arg("-xf")
            .arg(control)
            .arg("-C")
            .arg(&cfg.datadir),
        "Not a valid deb file, control extraction failed",
    )?;
    run_status(
        Command::new("tar")
            .arg("-xf")
            .arg(data)
            .arg("-C")
            .arg(&cfg.destdir),
        "Not a valid deb file, data extraction failed",
    )?;
    warn_maintainer_scripts(&cfg.datadir)?;
    validate_symlinks(&cfg.destdir)?;
    Ok(())
}

fn validate_tar_archive(path: &Path) -> Result<(), String> {
    let output = command_output(Command::new("tar").arg("-tf").arg(path))?;
    for entry in output.lines() {
        let p = Path::new(entry);
        if p.is_absolute() || p.components().any(|c| matches!(c, Component::ParentDir)) {
            return Err(format!(
                "archive '{}' contains unsafe path '{}'",
                path.display(),
                entry
            ));
        }
    }
    Ok(())
}

fn warn_maintainer_scripts(datadir: &Path) -> Result<(), String> {
    for script in ["preinst", "postinst", "prerm", "postrm", "config"] {
        let path = datadir.join(script);
        if path.exists() {
            log_warn(&format!(
                "Debian maintainer script '{}' is present but will not run automatically in XBPS",
                script
            ));
        }
    }
    Ok(())
}

fn validate_symlinks(root: &Path) -> Result<(), String> {
    for path in collect_paths(root)? {
        let Ok(meta) = fs::symlink_metadata(&path) else {
            continue;
        };
        if !meta.file_type().is_symlink() {
            continue;
        }
        let target = fs::read_link(&path)
            .map_err(|e| format!("Unable to read symlink {}: {e}", path.display()))?;
        if target.is_absolute()
            || target
                .components()
                .any(|c| matches!(c, Component::ParentDir))
        {
            return Err(format!(
                "unsafe symlink '{}' -> '{}' escapes package root",
                path.display(),
                target.display()
            ));
        }
    }
    Ok(())
}

fn parse_metadata(cfg: &Config) -> Result<PackageMeta, String> {
    let fields = parse_control(&cfg.datadir.join("control"))?;
    let mut name = cfg
        .name
        .clone()
        .or_else(|| fields.get("Package").cloned())
        .unwrap_or_default();
    name.push_str(&cfg.suffix);
    let mut version = cfg
        .version
        .clone()
        .or_else(|| fields.get("Version").cloned())
        .unwrap_or_default();
    let license = fields.get("License").cloned().unwrap_or_default();
    let maintainer = fields.get("Maintainer").cloned().unwrap_or_default();
    let mut short_desc = fields
        .get("Description")
        .map(|s| s.lines().next().unwrap_or("").trim().to_string())
        .unwrap_or_default();
    let mut long_desc = fields.get("Description").cloned().unwrap_or_default();
    let raw_arch = cfg
        .arch
        .clone()
        .or_else(|| fields.get("Architecture").cloned())
        .unwrap_or_default();
    let arch = match raw_arch.as_str() {
        "amd64" => "x86_64",
        "arm64" => "aarch64",
        "armhf" => "armv7l",
        "i386" => "i686",
        "all" => "noarch",
        _ => {
            return Err(format!(
                "Invalid arch: {raw_arch}. Use '--arch=...' to set arch manually."
            ))
        }
    }
    .to_string();

    validate_pkgname(&name)?;

    if short_desc.is_empty() && long_desc.is_empty() {
        return Err("Neither long_desc nor short_desc provided by package".into());
    } else if short_desc.is_empty() {
        short_desc = long_desc.clone();
    } else if long_desc.is_empty() {
        long_desc = short_desc.clone();
    }

    let mut revision = cfg.revision.clone().unwrap_or_else(|| "1".into());
    version = normalize_version(&version);
    validate_version(&version)?;
    revision = revision.chars().filter(|c| c.is_ascii_digit()).collect();
    if revision.is_empty() {
        revision = "1".into();
    }

    Ok(PackageMeta {
        name,
        version,
        revision,
        arch,
        license,
        maintainer,
        short_desc,
        long_desc,
    })
}

fn fix_known_conflicts(cfg: &Config) -> Result<(), String> {
    let pyver = command_output(Command::new("python3").arg("--version")).unwrap_or_default();
    let pyver = parse_python_minor(&pyver).unwrap_or_else(|| "3".into());
    for (src, dst, name) in [
        ("bin", "usr", None),
        ("lib", "usr", None),
        ("lib32", "usr", None),
        ("lib64", "usr", None),
        ("sbin", "usr", Some("bin")),
        ("usr/sbin", "usr", Some("bin")),
        ("usr/lib64", "usr", Some("lib")),
    ] {
        fix_conflict(cfg, src, dst, name)?;
    }
    fix_conflict(
        cfg,
        "usr/lib/python3/dist-packages",
        &format!("usr/lib/python{pyver}"),
        Some("site-packages"),
    )?;
    Ok(())
}

fn fix_conflict(
    cfg: &Config,
    src: &str,
    dst_dir: &str,
    dst_name: Option<&str>,
) -> Result<(), String> {
    let src_path = cfg.destdir.join(src);
    if !src_path.exists() || !src_path.is_dir() {
        return Ok(());
    }
    let dst_path = cfg.destdir.join(dst_dir).join(dst_name.unwrap_or(src));
    fs::create_dir_all(dst_path.parent().unwrap()).map_err(|e| {
        format!(
            "Unable to create {}: {e}",
            dst_path.parent().unwrap().display()
        )
    })?;
    copy_recursive(&src_path, &dst_path)?;
    remove_path(&src_path)?;
    log_info(&format!(
        "Moved conflict '{src}' -> '{}/{}'",
        dst_dir,
        dst_name.unwrap_or(src)
    ));
    Ok(())
}

fn gen_rdeps(cfg: &Config) -> Result<(Vec<String>, Vec<String>), String> {
    let shlibs = fs::read_to_string(&cfg.shlibs)
        .map_err(|e| format!("Unable to read {}: {e}", cfg.shlibs.display()))?;
    let mut needed = BTreeSet::new();
    let files = collect_paths(&cfg.destdir)?;
    for file in files.iter().filter(|p| p.is_file()) {
        if !is_elf(file)? {
            continue;
        }
        let out = command_output(Command::new("objdump").arg("-p").arg(file))?;
        for line in out.lines() {
            let mut parts = line.split_whitespace();
            if parts.next() == Some("NEEDED") {
                if let Some(lib) = parts.next() {
                    needed.insert(lib.to_string());
                }
            }
        }
    }

    let mut deps = BTreeSet::new();
    let mut explanations = Vec::new();
    let host_libc = detect_host_libc();
    for lib in needed {
        if provided_by_destdir(&cfg.destdir, &lib)? {
            explanations.push(format!("{lib} -> provided by current package"));
            continue;
        }
        let mut rdep = find_shlib_dep(&shlibs, &lib);
        if rdep.is_none() && cfg.opt_lazy_soname {
            log_warn(&format!("Unable to find dependency for {lib}, trying lazy"));
            let lazy = lib
                .split(".so")
                .next()
                .map(|p| format!("{p}.so"))
                .unwrap_or_else(|| lib.clone());
            rdep = find_shlib_dep(&shlibs, &lazy);
        }
        let Some(rdep) = rdep else {
            log_warn(&format!("Unable to find dependency for {lib}"));
            explanations.push(format!("{lib} -> unresolved"));
            continue;
        };
        let (pkg, ver) = split_pkg_version(&rdep);
        let pkg = format!("{pkg}{}", cfg.suffix);
        if should_skip_for_host_libc(host_libc, &pkg) {
            explanations.push(format!("{lib} -> {pkg}>={ver} skipped for host libc"));
            continue;
        }
        if cfg.not_dependencies.iter().any(|d| d == &pkg) {
            explanations.push(format!("{lib} -> {pkg}>={ver} skipped by --not-deps"));
            continue;
        }
        let dep = format!("{pkg}>={ver}");
        explanations.push(format!("{lib} -> {dep}"));
        deps.insert(dep);
    }
    Ok((deps.into_iter().collect(), explanations))
}

fn check_tree(cfg: &mut Config, pkgname: &str) -> Result<(), String> {
    let ignored = if cfg.opt_warn_conflict && command_exists("xbps-query") {
        command_output(Command::new("xbps-query").arg("-f").arg(pkgname))
            .unwrap_or_default()
            .lines()
            .map(|l| l.split(" -> ").next().unwrap_or(l).to_string())
            .collect::<BTreeSet<_>>()
    } else {
        BTreeSet::new()
    };

    let mut paths = collect_paths(&cfg.destdir)?;
    paths.sort_by_key(|p| std::cmp::Reverse(p.components().count()));
    let mut conflict = false;
    for path in paths.into_iter().filter(|p| p != &cfg.destdir) {
        if cfg.opt_clean_dir
            && path.is_dir()
            && fs::read_dir(&path)
                .map(|mut d| d.next().is_none())
                .unwrap_or(false)
        {
            remove_path(&path)?;
            log_info(&format!("Removed empty directory {}", path.display()));
            continue;
        }
        if !cfg.opt_warn_conflict {
            continue;
        }
        let rel = path.strip_prefix(&cfg.destdir).unwrap_or(&path);
        let system_path = PathBuf::from("/").join(rel);
        if !system_path.exists() || ignored.contains(system_path.to_string_lossy().as_ref()) {
            continue;
        }
        if system_path.is_dir() && path.is_dir() && !system_path.is_symlink() && !path.is_symlink()
        {
            continue;
        }
        let shown = path
            .strip_prefix(env::current_dir().unwrap_or_default())
            .unwrap_or(&path);
        let message = if path.is_dir()
            && fs::read_dir(&path)
                .map(|mut d| d.next().is_none())
                .unwrap_or(false)
        {
            format!(
                "Conflict: '{}'. Use '-e' to remove automatically.",
                shown.display()
            )
        } else {
            format!("Conflict: '{}'", shown.display())
        };
        match cfg.conflict_mode {
            ConflictMode::Warn => log_warn(&message),
            ConflictMode::Error | ConflictMode::Auto => log_crit(&message),
            ConflictMode::Ignore => {}
        }
        conflict = true;
    }
    if conflict && matches!(cfg.conflict_mode, ConflictMode::Error | ConflictMode::Auto) {
        cfg.refused = true;
        log_crit(&format!(
            "Consider (re)moving file(s) from '{}' and run `xdeb -rb`",
            cfg.destdir.display()
        ));
    }
    Ok(())
}

fn build_package(cfg: &Config, meta: &mut PackageMeta) -> Result<(), String> {
    check_command("xbps-create", "xbps")?;
    if cfg.opt_register {
        check_command("xbps-rindex", "xbps")?;
    }
    if cfg.opt_install {
        check_command("xbps-install", "xbps")?;
    }

    let out = format!("{}-{}_{}", meta.name, meta.version, meta.revision);
    run_status(
        Command::new("xbps-create")
            .current_dir(&cfg.binpkgs)
            .arg("-q")
            .arg("-t")
            .arg("xdeb")
            .arg("-A")
            .arg(&meta.arch)
            .arg("-n")
            .arg(&out)
            .arg("-m")
            .arg(&meta.maintainer)
            .arg("-s")
            .arg(&meta.short_desc)
            .arg("-S")
            .arg(&meta.long_desc)
            .arg("-l")
            .arg(&meta.license)
            .arg("-D")
            .arg(cfg.dependencies.join(" "))
            .arg("-C")
            .arg(cfg.conflicts.join(" "))
            .arg("-R")
            .arg(cfg.replaces.join(" "))
            .arg("-P")
            .arg(cfg.provides.join(" "))
            .arg("--build-options")
            .arg(env::var("XDEB_BUILD_OPTIONS").unwrap_or_default())
            .arg(&cfg.destdir),
        "xbps-create failed",
    )?;

    let package = format!("{out}.{}.xbps", meta.arch);
    if cfg.opt_register {
        run_status(
            Command::new("xbps-rindex")
                .current_dir(&cfg.binpkgs)
                .arg("-a")
                .arg(&package),
            "xbps-rindex failed",
        )?;
    }
    if !stdout_is_terminal() {
        println!("{}", cfg.binpkgs.join(&package).display());
    }
    if cfg.refused {
        return Err("Errors occurred. Do not install the package on this system!".into());
    } else if cfg.opt_install {
        let helper = command_output(
            Command::new("sh")
                .arg("-c")
                .arg("command -v sudo || command -v doas"),
        )?;
        let helper = helper.lines().next().ok_or("Neither sudo nor doas found")?;
        run_status(
            Command::new(helper)
                .arg("xbps-install")
                .arg("-R")
                .arg(&cfg.binpkgs)
                .arg(&out),
            "xbps-install failed",
        )?;
    } else {
        log_info(&format!(
            "Install using `xbps-install -R {} {out}`",
            cfg.binpkgs.display()
        ));
    }
    Ok(())
}

fn parse_control(path: &Path) -> Result<HashMap<String, String>, String> {
    let text =
        fs::read_to_string(path).map_err(|e| format!("Unable to read {}: {e}", path.display()))?;
    let mut fields: HashMap<String, String> = HashMap::new();
    let mut current: Option<String> = None;
    for line in text.lines() {
        if line.starts_with(' ') || line.starts_with('\t') {
            if let Some(key) = &current {
                fields.entry(key.clone()).and_modify(|v| {
                    v.push('\n');
                    v.push_str(line.trim());
                });
            }
            continue;
        }
        if let Some((key, value)) = line.split_once(':') {
            let key = key.trim().to_string();
            fields.insert(key.clone(), value.trim().to_string());
            current = Some(key);
        }
    }
    Ok(fields)
}

fn find_shlib_dep(shlibs: &str, lib: &str) -> Option<String> {
    let lib_lower = lib.to_ascii_lowercase();
    shlibs.lines().find_map(|line| {
        let mut parts = line.split_whitespace();
        let soname = parts.next()?;
        let dep = parts.next()?;
        let soname_lower = soname.to_ascii_lowercase();
        if soname_lower == lib_lower || soname_lower.starts_with(&(lib_lower.clone() + ".")) {
            Some(dep.to_string())
        } else {
            None
        }
    })
}

fn split_pkg_version(rdep: &str) -> (&str, &str) {
    rdep.rsplit_once('-').unwrap_or((rdep, "0"))
}

fn normalize_version(version: &str) -> String {
    let mut v: String = version
        .chars()
        .map(|c| if matches!(c, '-' | '_' | '/') { '.' } else { c })
        .collect();
    if v.is_empty() {
        v = "0".into();
    } else if !v.chars().any(|c| c.is_ascii_digit()) {
        v.push('0');
    }
    v
}

fn parse_python_minor(version: &str) -> Option<String> {
    let word = version
        .split_whitespace()
        .find(|s| s.chars().next().is_some_and(|c| c.is_ascii_digit()))?;
    let mut parts = word.split('.');
    Some(format!("{}.{}", parts.next()?, parts.next()?))
}

fn first_match(dir: &Path, prefix: &str) -> Result<PathBuf, String> {
    fs::read_dir(dir)
        .map_err(|e| format!("Unable to read {}: {e}", dir.display()))?
        .flatten()
        .map(|e| e.path())
        .find(|p| {
            p.file_name()
                .and_then(OsStr::to_str)
                .is_some_and(|n| n.starts_with(prefix))
        })
        .ok_or_else(|| format!("Missing {prefix} archive"))
}

fn collect_paths(root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut out = Vec::new();
    if !root.exists() {
        return Ok(out);
    }
    let mut stack = vec![root.to_path_buf()];
    while let Some(path) = stack.pop() {
        out.push(path.clone());
        if path.is_dir() && !path.is_symlink() {
            for entry in fs::read_dir(&path)
                .map_err(|e| format!("Unable to read {}: {e}", path.display()))?
            {
                stack.push(entry.map_err(|e| e.to_string())?.path());
            }
        }
    }
    Ok(out)
}

fn copy_recursive(src: &Path, dst: &Path) -> Result<(), String> {
    let meta =
        fs::symlink_metadata(src).map_err(|e| format!("Unable to stat {}: {e}", src.display()))?;
    if meta.is_dir() {
        fs::create_dir_all(dst).map_err(|e| format!("Unable to create {}: {e}", dst.display()))?;
        for entry in
            fs::read_dir(src).map_err(|e| format!("Unable to read {}: {e}", src.display()))?
        {
            let entry = entry.map_err(|e| e.to_string())?;
            copy_recursive(&entry.path(), &dst.join(entry.file_name()))?;
        }
    } else if meta.file_type().is_symlink() {
        let target = fs::read_link(src)
            .map_err(|e| format!("Unable to read link {}: {e}", src.display()))?;
        #[cfg(unix)]
        std::os::unix::fs::symlink(target, dst)
            .map_err(|e| format!("Unable to symlink {}: {e}", dst.display()))?;
    } else {
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Unable to create {}: {e}", parent.display()))?;
        }
        fs::copy(src, dst)
            .map_err(|e| format!("Unable to copy {} to {}: {e}", src.display(), dst.display()))?;
        fs::set_permissions(dst, meta.permissions()).map_err(|e| {
            format!(
                "Unable to preserve permissions on {} from {}: {e}",
                dst.display(),
                src.display()
            )
        })?;
    }
    Ok(())
}

fn provided_by_destdir(destdir: &Path, lib: &str) -> Result<bool, String> {
    Ok(collect_paths(destdir)?
        .iter()
        .any(|p| p.file_name().and_then(OsStr::to_str) == Some(lib)))
}

fn is_elf(path: &Path) -> Result<bool, String> {
    let mut f =
        fs::File::open(path).map_err(|e| format!("Unable to read {}: {e}", path.display()))?;
    let mut magic = [0; 4];
    Ok(f.read_exact(&mut magic).is_ok() && magic == [0x7f, b'E', b'L', b'F'])
}

fn env_path(key: &str, default: PathBuf) -> PathBuf {
    normalize_path(env::var_os(key).map(PathBuf::from).unwrap_or(default))
}

fn env_bool(key: &str, default: bool) -> bool {
    env::var(key).map(|v| v == "true").unwrap_or(default)
}

fn split_env_words(key: &str) -> Vec<String> {
    env::var(key).map(|v| split_words(&v)).unwrap_or_default()
}

fn split_words(s: &str) -> Vec<String> {
    s.split_whitespace()
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

fn trim_extensions(path: &str, count: usize) -> String {
    let mut out = path.to_string();
    for _ in 0..count {
        let Some((prefix, _)) = out.rsplit_once('.') else {
            break;
        };
        out = prefix.to_string();
    }
    out
}

fn parse_conflict_mode(value: &str) -> Result<ConflictMode, String> {
    match value {
        "warn" => Ok(ConflictMode::Warn),
        "error" => Ok(ConflictMode::Error),
        "ignore" => Ok(ConflictMode::Ignore),
        "auto" => Ok(ConflictMode::Auto),
        other => Err(format!("invalid file conflict mode '{other}'")),
    }
}

fn validate_pkgname(name: &str) -> Result<(), String> {
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '_' | '.'))
    {
        return Err(format!("Invalid package name '{name}'"));
    }
    Ok(())
}

fn validate_version(version: &str) -> Result<(), String> {
    if version.is_empty() || version.chars().any(char::is_whitespace) {
        return Err(format!("Invalid package version '{version}'"));
    }
    Ok(())
}

fn detect_host_libc() -> HostLibc {
    let ldd = command_output(Command::new("ldd").arg("--version")).unwrap_or_default();
    let lower = ldd.to_ascii_lowercase();
    if lower.contains("musl") {
        HostLibc::Musl
    } else if lower.contains("glibc")
        || lower.contains("gnu libc")
        || lower.contains("gnu c library")
    {
        HostLibc::Glibc
    } else if Path::new("/lib/ld-musl-x86_64.so.1").exists() {
        HostLibc::Musl
    } else {
        HostLibc::Unknown
    }
}

fn should_skip_for_host_libc(host: HostLibc, pkg: &str) -> bool {
    match host {
        HostLibc::Glibc => pkg.starts_with("musl"),
        HostLibc::Musl => pkg == "glibc" || pkg.starts_with("glibc-") || pkg.starts_with("glibc>"),
        HostLibc::Unknown => false,
    }
}

fn shlibs_is_stale(shlibs: &Path, max_age_days: u64) -> bool {
    let meta = shlibs.with_extension("meta");
    let Ok(text) = fs::read_to_string(meta) else {
        return true;
    };
    let fetched = text.lines().find_map(|line| {
        line.strip_prefix("fetched_unix=")
            .and_then(|v| v.parse::<u64>().ok())
    });
    fetched
        .map(|ts| unix_time().saturating_sub(ts) > max_age_days * 24 * 60 * 60)
        .unwrap_or(true)
}

fn confirm(prompt: &str) -> Result<bool, String> {
    if !io::IsTerminal::is_terminal(&io::stdin()) {
        return Ok(false);
    }
    eprint!("{prompt}");
    let mut input = String::new();
    io::stdin()
        .read_line(&mut input)
        .map_err(|e| format!("Unable to read confirmation: {e}"))?;
    Ok(matches!(input.trim(), "y" | "Y" | "yes" | "YES"))
}

fn json_escape(s: &str) -> String {
    s.chars()
        .flat_map(|c| match c {
            '\\' => "\\\\".chars().collect::<Vec<_>>(),
            '"' => "\\\"".chars().collect::<Vec<_>>(),
            '\n' => "\\n".chars().collect::<Vec<_>>(),
            '\r' => "\\r".chars().collect::<Vec<_>>(),
            '\t' => "\\t".chars().collect::<Vec<_>>(),
            c => vec![c],
        })
        .collect()
}

fn current_uid() -> u32 {
    command_output(Command::new("id").arg("-u"))
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(u32::MAX)
}

fn unix_time() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn privilege_helper() -> Result<String, String> {
    for helper in ["sudo", "doas"] {
        if command_exists(helper) {
            return Ok(helper.into());
        }
    }
    Err("Neither sudo nor doas found".into())
}

fn normalize_path(path: PathBuf) -> PathBuf {
    let path = if path.is_absolute() {
        path
    } else {
        env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    };
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

fn remove_path(path: &Path) -> Result<(), String> {
    if !path.exists() && fs::symlink_metadata(path).is_err() {
        return Ok(());
    }
    let meta = fs::symlink_metadata(path)
        .map_err(|e| format!("Unable to stat {}: {e}", path.display()))?;
    if meta.is_dir() && !meta.file_type().is_symlink() {
        fs::remove_dir_all(path).map_err(|e| format!("Unable to remove {}: {e}", path.display()))
    } else {
        fs::remove_file(path).map_err(|e| format!("Unable to remove {}: {e}", path.display()))
    }
}

fn check_command(cmd: &str, pkg: &str) -> Result<(), String> {
    if command_exists(cmd) {
        Ok(())
    } else {
        Err(format!(
            "Executable '{cmd}' from package '{pkg}' not found."
        ))
    }
}

fn command_exists(cmd: &str) -> bool {
    Command::new("sh")
        .arg("-c")
        .arg(format!(
            "command -v {} >/dev/null 2>&1",
            shell_quote_str(cmd)
        ))
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn run_status(cmd: &mut Command, err: &str) -> Result<(), String> {
    let status = cmd.status().map_err(|e| format!("{err}: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(err.into())
    }
}

fn command_output(cmd: &mut Command) -> Result<String, String> {
    let output = cmd
        .stderr(Stdio::inherit())
        .output()
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(format!("command failed with status {}", output.status));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn stdout_is_terminal() -> bool {
    io::IsTerminal::is_terminal(&io::stdout())
}

fn shell_quote(path: &Path) -> String {
    shell_quote_str(&path.to_string_lossy())
}

fn shell_quote_str(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

fn log_info(msg: &str) {
    if color_enabled() {
        eprintln!("\x1b[1;36mI\x1b[0m {msg}\x1b[0m");
    } else {
        eprintln!("I {msg}");
    }
}

fn log_warn(msg: &str) {
    if color_enabled() {
        eprintln!("\x1b[1;33mW\x1b[0m {msg}\x1b[0m");
    } else {
        eprintln!("W {msg}");
    }
}

fn log_crit(msg: &str) {
    if color_enabled() {
        eprintln!("\x1b[1;31mE\x1b[0;1m {msg}\x1b[0m");
    } else {
        eprintln!("E {msg}");
    }
}

fn color_enabled() -> bool {
    env::var_os("NO_COLOR").is_none()
        && env::var("XDEB_COLOR").map(|v| v != "never").unwrap_or(true)
}
