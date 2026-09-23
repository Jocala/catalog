//! catalog-cli: acceptance harness for catalog-core (Phase 5).
//! Exercises library, zip, smb, and settings with no GUI.
//!
//!   catalog-cli library open <path> [--user U --password P --domain D]
//!   catalog-cli library list <path> [QUERY] [--mode M] [--title T ...]
//!   catalog-cli library detail <path> <id>
//!   catalog-cli import-zip <file> --to <dir>
//!   catalog-cli smb ls <unc> [--user U ...]
//!   catalog-cli smb get <unc> --out <file> [--user U ...]
//!   catalog-cli cover <library> <bookdir> [--out <file>]
//!   catalog-cli settings get <key> | set <key> <value> | path
//!
//! <path> is a local dir or smb://host/share[/dir].
//! <unc> is //host/share/path, \\host\share\path, or smb://host/share/path.

use catalog_core::db::{
    all_authors, all_series, all_tags, book_detail, fetch_books, fetch_count, open_memory_db,
    search_books, FileSource, SearchParams,
};
use catalog_core::settings::Settings;
use catalog_core::smb;
use clap::{Args, Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "catalog-cli", version, about = "catalog-core acceptance CLI")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Args, Clone)]
struct Creds {
    #[arg(long)]
    user: Option<String>,
    #[arg(long)]
    password: Option<String>,
    #[arg(long)]
    domain: Option<String>,
}

#[derive(Subcommand)]
enum Cmd {
    Library {
        #[command(subcommand)]
        op: LibraryOp,
    },
    ImportZip {
        file: PathBuf,
        #[arg(long)]
        to: PathBuf,
    },
    Smb {
        #[command(subcommand)]
        op: SmbOp,
    },
    /// Fetch a book's cover (187x240 scaled JPEG) or print `none`.
    Cover {
        /// Library path (local dir or smb://host/share[/dir]).
        library: String,
        /// Book dir as listed by `library list` (book.path).
        bookdir: String,
        #[arg(long)]
        out: Option<PathBuf>,
        #[command(flatten)]
        creds: Creds,
    },
    Settings {
        #[command(subcommand)]
        op: SettingsOp,
    },
    Kobo {
        #[command(subcommand)]
        op: KoboOp,
    },
}

#[derive(Subcommand)]
enum LibraryOp {
    /// Validate a library and print its book count.
    Open { path: String, #[command(flatten)] creds: Creds },
    /// List books (default) or author/series/tag summaries.
    List {
        path: String,
        query: Option<String>,
        #[arg(long, default_value = "books")]
        mode: String,
        #[arg(long, default_value = "")]
        title: String,
        #[arg(long, default_value = "")]
        author: String,
        #[arg(long, default_value = "")]
        series: String,
        #[arg(long, default_value = "")]
        tag: String,
        #[command(flatten)]
        creds: Creds,
    },
    /// Full detail for one book id.
    Detail {
        path: String,
        id: i64,
        #[command(flatten)]
        creds: Creds,
    },
}

#[derive(Subcommand)]
enum SmbOp {
    /// List a share directory: smb ls //host/share[/path].
    Ls { unc: String, #[command(flatten)] creds: Creds },
    /// Copy a remote file locally: smb get //host/share/path --out file.
    Get {
        unc: String,
        #[arg(long)]
        out: PathBuf,
        #[command(flatten)]
        creds: Creds,
    },
}

#[derive(Subcommand)]
enum SettingsOp {
    Get { key: String },
    Set { key: String, value: String },
    /// Print the settings file location.
    Path,
}

#[derive(Subcommand)]
enum KoboOp {
    /// SSH shell-exec spike (russh). Password comes from KOBO_PASSWORD
    /// env (never argv, never printed); empty means the default key file.
    /// Prints the remote output, then `exit=<code>`.
    Ssh {
        ip: String,
        cmd: String,
        #[arg(long, default_value_t = 10)]
        timeout: u64,
    },
    /// Full open flow (probe → predict → find → match → open).
    Open {
        ip: String,
        title: String,
        author: String,
    },
    /// Full sync & open flow for a library book.
    Sync {
        path: String,
        id: i64,
        ip: String,
        #[command(flatten)]
        creds: Creds,
    },
}

fn source_from(
    path: &str,
    creds: &Creds,
    settings: &Settings,
) -> Result<FileSource, String> {
    FileSource::from_arg(
        path,
        creds.user.as_deref(),
        creds.password.as_deref(),
        creds.domain.as_deref(),
        settings,
    )
    .map_err(|e| e.to_string())
}

async fn open_db(source: &FileSource) -> Result<rusqlite::Connection, String> {
    let bytes = source.read_db_bytes().await.map_err(|e| e.to_string())?;
    open_memory_db(&bytes).map_err(|e| e.to_string())
}

async fn cmd_library_open(path: &str, creds: &Creds, settings: &Settings) -> Result<(), String> {
    let source = source_from(path, creds, settings)?;
    let db = open_db(&source).await?;
    let n = fetch_count(&db, "").map_err(|e| e.to_string())?;
    println!("{}: {n} books", source.display_path());
    Ok(())
}

struct ListQuery<'a> {
    query: Option<&'a str>,
    mode: &'a str,
    title: &'a str,
    author: &'a str,
    series: &'a str,
    tag: &'a str,
}

async fn cmd_library_list(
    path: &str,
    q: ListQuery<'_>,
    creds: &Creds,
    settings: &Settings,
) -> Result<(), String> {
    let source = source_from(path, creds, settings)?;
    let db = open_db(&source).await?;
    match q.mode {
        "authors" => {
            for a in all_authors(&db, &source, false).map_err(|e| e.to_string())? {
                println!("{}|{}|{} books", a.id, a.name, a.book_count);
            }
        }
        "series" => {
            for s in all_series(&db, &source, false, false).map_err(|e| e.to_string())? {
                println!("{}|{}|{} books", s.id, s.name, s.book_count);
            }
        }
        "tags" => {
            for t in all_tags(&db, false).map_err(|e| e.to_string())? {
                println!("{}|{}|{} books", t.id, t.name, t.book_count);
            }
        }
        _ => {
            let params = SearchParams {
                query: q.query.unwrap_or("").to_string(),
                title: q.title.to_string(),
                author: q.author.to_string(),
                series: q.series.to_string(),
                tag: q.tag.to_string(),
                ..Default::default()
            };
            let found =
                search_books(&db, &source, &params).map_err(|e| e.to_string())?;
            // Mirror fetch_books default ordering input: when no filters at
            // all, show the whole catalogue the same way the GUI root does.
            let found = if params.query.is_empty()
                && q.title.is_empty()
                && q.author.is_empty()
                && q.series.is_empty()
                && q.tag.is_empty()
            {
                fetch_books(&db, &source, "", false, true, false)
                    .map_err(|e| e.to_string())?
                    .into_iter()
                    .map(|b| catalog_core::models::SearchedBook {
                        id: b.id,
                        title: b.title,
                        author: b.author,
                        path: b.path,
                        series: b.series,
                        series_index: b.series_index,
                        tags: b.tags,
                        cover_hash: String::new(),
                        author_sort: String::new(),
                    })
                    .collect()
            } else {
                found
            };
            for b in &found {
                println!("{}|{}|{}", b.id, b.title, b.author);
            }
            eprintln!("{} result(s)", found.len());
        }
    }
    Ok(())
}

async fn cmd_library_detail(
    path: &str,
    id: i64,
    creds: &Creds,
    settings: &Settings,
) -> Result<(), String> {
    let source = source_from(path, creds, settings)?;
    let db = open_db(&source).await?;
    let d = book_detail(&db, id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("no book id {id}"))?;
    println!("title: {}", d.title);
    println!("author: {}", d.author);
    if let Some(s) = d.series {
        println!("series: {s} #{}", d.series_index);
    }
    if let Some(t) = d.tags {
        println!("tags: {t}");
    }
    if let Some(p) = d.publisher {
        println!("publisher: {p}");
    }
    if let Some(i) = d.isbn {
        println!("isbn: {i}");
    }
    if let Some(c) = d.comments {
        println!("comments: {c}");
    }
    Ok(())
}

fn cmd_import_zip(file: &Path, to: &Path) -> Result<(), String> {
    let placed =
        catalog_core::epub::import_zip(file, to).map_err(|e| e.to_string())?;
    if placed.is_empty() {
        println!("archive held no files");
    }
    for p in placed {
        println!("{}", p.display());
    }
    Ok(())
}

fn unc_parts(unc: &str) -> Result<(String, String, String), String> {
    smb::parse_unc(unc)
        .ok_or_else(|| "UNC must be //host/share[/path], \\\\host\\share[\\path], or smb://host/share[/path]".to_string())
}

async fn cmd_smb_ls(unc: &str, creds: &Creds, settings: &Settings) -> Result<(), String> {
    let (host, share, rest) = unc_parts(unc)?;
    let conn = smb::conn_for(
        &host,
        creds.user.as_deref(),
        creds.password.as_deref(),
        creds.domain.as_deref(),
        settings,
    );
    let entries = smb::list_dir(conn, &share, &rest).await.map_err(|e| e.to_string())?;
    for e in entries {
        println!("{}\t{}\t{}", if e.is_directory { "d" } else { "f" }, e.size, e.name);
    }
    Ok(())
}

async fn cmd_smb_get(
    unc: &str,
    out: &Path,
    creds: &Creds,
    settings: &Settings,
) -> Result<(), String> {
    let (host, share, rest) = unc_parts(unc)?;
    if rest.is_empty() {
        return Err("smb get needs a file path, not a directory".to_string());
    }
    let conn = smb::conn_for(
        &host,
        creds.user.as_deref(),
        creds.password.as_deref(),
        creds.domain.as_deref(),
        settings,
    );
    let data = smb::download_db_bytes(conn, &share, &rest)
        .await
        .map_err(|e| e.to_string())?;
    if let Some(parent) = out.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
    }
    std::fs::write(out, &data).map_err(|e| e.to_string())?;
    println!("{} bytes -> {}", data.len(), out.display());
    Ok(())
}

async fn cmd_cover(
    library: &str,
    bookdir: &str,
    out: Option<&Path>,
    creds: &Creds,
    settings: &Settings,
) -> Result<(), String> {
    let src = source_from(library, creds, settings)?;
    let path = src.book_path(bookdir);
    let data = catalog_core::covers::fetch_cover_cached(&src, &path)
        .await
        .filter(|d| !d.is_empty())
        .ok_or_else(|| format!("no cover for {bookdir}"))?;
    let scaled = catalog_core::covers::scale_cover(&data, 187, 240)
        .ok_or_else(|| format!("undecodable cover for {bookdir}"))?;
    if let Some(dest) = out {
        if let Some(parent) = dest.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
        }
        std::fs::write(dest, &scaled).map_err(|e| e.to_string())?;
        println!("{} bytes -> {}", scaled.len(), dest.display());
    } else {
        println!("{} bytes", scaled.len());
    }
    Ok(())
}

fn cmd_settings_get(key: &str, settings: &Settings) -> Result<(), String> {
    match key {
        "library_source" => println!("{}", settings.library_source),
        "local_library_dir" => println!("{}", settings.local_library_dir),
        "kobo_ip" => println!("{}", settings.kobo_ip),
        "kobo_ips" => println!("{}", settings.kobo_ips.join(",")),
        "theme_preference" => println!("{}", settings.theme_preference),
        "diagnostic_logging" => println!("{}", settings.diagnostic_logging),
        "smb_host" => println!("{}", settings.primary_server().map(|s| s.host.as_str()).unwrap_or("")),
        "smb_share" => println!(
            "{}",
            settings
                .primary_server()
                .and_then(|s| s.shares.first())
                .map(|s| s.name.as_str())
                .unwrap_or("")
        ),
        "smb_user" => println!("{}", settings.primary_server().map(|s| s.user.as_str()).unwrap_or("")),
        "smb_domain" => println!("{}", settings.primary_server().map(|s| s.domain.as_str()).unwrap_or("")),
        "smb_calibre_path" => println!(
            "{}",
            settings
                .primary_server()
                .and_then(|s| s.shares.first())
                .map(|s| s.calibre_metadata_path.as_str())
                .unwrap_or("")
        ),
        "smb_password" => println!(
            "{}",
            match settings.primary_server().and_then(|s| settings.read_password(&s.host)) {
                Some(_) => "(set)",
                None => "(empty)",
            }
        ),
        _ => return Err(format!("unknown key: {key}")),
    }
    Ok(())
}

fn cmd_settings_set(key: &str, value: &str, settings: &mut Settings) -> Result<(), String> {
    match key {
        "library_source" if value == "smb" || value == "local" => {
            settings.library_source = value.to_string();
        }
        "local_library_dir" => settings.local_library_dir = value.to_string(),
        "kobo_ip" => settings.kobo_ip = value.to_string(),
        "kobo_ips" => {
            settings.kobo_ips =
                value.split(',').map(str::trim).filter(|s| !s.is_empty()).map(str::to_string).collect();
            if !settings.kobo_ips.contains(&settings.kobo_ip) {
                settings.kobo_ip = settings.kobo_ips.first().cloned().unwrap_or_default();
            }
        }
        "theme_preference" => {
            settings.theme_preference =
                value.parse().map_err(|_| "theme_preference must be an integer".to_string())?;
        }
        "diagnostic_logging" => {
            settings.diagnostic_logging = matches!(value, "1" | "true" | "yes");
        }
        "smb_host" | "smb_share" | "smb_user" | "smb_domain" | "smb_calibre_path"
        | "smb_password" => {
            // Ensure a primary server row exists, then patch the field.
            if settings.primary_server().is_none() {
                settings.save_servers(vec![catalog_core::models::SmbServer {
                    label: String::new(),
                    host: String::new(),
                    port: 445,
                    user: String::new(),
                    domain: String::new(),
                    shares: vec![catalog_core::models::SmbShare::new("")],
                }]);
            }
            let server = &mut settings.smb_servers[0];
            match key {
                "smb_host" => server.host = value.to_string(),
                "smb_share" => {
                    if server.shares.is_empty() {
                        server.shares.push(catalog_core::models::SmbShare::new(value));
                    } else {
                        server.shares[0].name = value.to_string();
                    }
                }
                "smb_user" => server.user = value.to_string(),
                "smb_domain" => server.domain = value.to_string(),
                "smb_calibre_path" => {
                    if server.shares.is_empty() {
                        server.shares.push(catalog_core::models::SmbShare::new(""));
                    }
                    server.shares[0].calibre_metadata_path = value.to_string();
                }
                "smb_password" => {
                    if server.host.is_empty() {
                        return Err("set smb_host before smb_password".to_string());
                    }
                    let host = server.host.clone();
                    settings.save_password(&host, value);
                }
                _ => unreachable!(),
            }
        }
        _ => return Err(format!("unknown key: {key}")),
    }
    settings.save().map_err(|e| e.to_string())?;
    println!("{key} updated");
    Ok(())
}

use std::path::Path;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), String> {
    let cli = Cli::parse();
    let mut settings = Settings::load();
    match cli.cmd {
        Cmd::Library { op } => match op {
            LibraryOp::Open { path, creds } => cmd_library_open(&path, &creds, &settings).await,
            LibraryOp::List { path, query, mode, title, author, series, tag, creds } => {
                cmd_library_list(
                    &path,
                    ListQuery {
                        query: query.as_deref(),
                        mode: &mode,
                        title: &title,
                        author: &author,
                        series: &series,
                        tag: &tag,
                    },
                    &creds,
                    &settings,
                )
                .await
            }
            LibraryOp::Detail { path, id, creds } => {
                cmd_library_detail(&path, id, &creds, &settings).await
            }
        },
        Cmd::ImportZip { file, to } => cmd_import_zip(&file, &to),
        Cmd::Smb { op } => match op {
            SmbOp::Ls { unc, creds } => cmd_smb_ls(&unc, &creds, &settings).await,
            SmbOp::Get { unc, out, creds } => cmd_smb_get(&unc, &out, &creds, &settings).await,
        },
        Cmd::Cover { library, bookdir, out, creds } => {
            cmd_cover(&library, &bookdir, out.as_deref(), &creds, &settings).await
        }
        Cmd::Settings { op } => match op {
            SettingsOp::Get { key } => cmd_settings_get(&key, &settings),
            SettingsOp::Set { key, value } => cmd_settings_set(&key, &value, &mut settings),
            SettingsOp::Path => {
                println!("{}", catalog_core::settings::settings_file().display());
                Ok(())
            }
        },
        Cmd::Kobo { op } => match op {
            KoboOp::Ssh { ip, cmd, timeout } => cmd_kobo_ssh(&ip, &cmd, timeout).await,
            KoboOp::Open { ip, title, author } => cmd_kobo_open(&ip, &title, &author).await,
            KoboOp::Sync { path, id, ip, creds } => {
                cmd_kobo_sync(&path, id, &ip, &creds, &settings).await
            }
        },
    }
}

fn kobo_auth() -> catalog_core::kobo::ssh::SshAuth {
    use catalog_core::kobo::ssh::{default_key_file, SshAuth};
    let pw = std::env::var("KOBO_PASSWORD").unwrap_or_default();
    if pw.is_empty() {
        SshAuth::KeyFile(default_key_file())
    } else {
        SshAuth::Password(pw)
    }
}

fn print_outcome(v: serde_json::Value) -> Result<(), String> {
    println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default());
    if v.get("status").and_then(|s| s.as_str()) == Some("opened") {
        Ok(())
    } else {
        Err(format!(
            "kobo outcome: {}",
            v.get("status").and_then(|s| s.as_str()).unwrap_or("?")
        ))
    }
}

async fn cmd_kobo_open(ip: &str, title: &str, author: &str) -> Result<(), String> {
    let out =
        catalog_core::kobo::open::open_on_kobo(ip, title, author, kobo_auth(), None).await;
    print_outcome(out.to_json())
}

async fn cmd_kobo_sync(
    path: &str,
    id: i64,
    ip: &str,
    creds: &Creds,
    settings: &Settings,
) -> Result<(), String> {
    let src = source_from(path, creds, settings)?;
    let out = catalog_core::kobo::sync::sync_and_open(&src, id, ip, kobo_auth()).await;
    print_outcome(out.to_json())
}

async fn cmd_kobo_ssh(ip: &str, cmd: &str, timeout: u64) -> Result<(), String> {
    use catalog_core::kobo::ssh::{default_key_file, ssh_sync_russh, SshAuth};
    let pw = std::env::var("KOBO_PASSWORD").unwrap_or_default();
    let auth = if pw.is_empty() {
        SshAuth::KeyFile(default_key_file())
    } else {
        SshAuth::Password(pw)
    };
    let r = ssh_sync_russh(ip, cmd, timeout, auth).await;
    println!("{}", r.output);
    println!("exit={}", r.code);
    if r.code != 0 {
        return Err(format!("kobo ssh exit {}", r.code));
    }
    Ok(())
}
