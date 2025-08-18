use clap::{Arg, Command};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::error::Error;
use std::fs::{self, File};
use std::io::Write;
use std::path::PathBuf;
use walkdir::WalkDir;

#[derive(Debug, Serialize, Deserialize)]
struct FileEntry {
    filename: String,
    hash: String,
    version: i32,
    action: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let matches = Command::new("mynk")
        .about("Synchronizes directory")
        .version("0.1.0")
        .subcommand_required(true)
        .arg_required_else_help(true)
        .subcommand(
            Command::new("init")
                .short_flag('I')
                .long_flag("init")
                .about("Initiate the mynk-db")
                .arg(
                    Arg::new("uri")
                        .long("uri")
                        .value_name("URI")
                        .help("Specify URI for initialization"),
                ),
        )
        .subcommand(
            Command::new("sync")
                .short_flag('S')
                .long_flag("sync")
                .about("Synchronize directory"),
        )
        .get_matches();

    match matches.subcommand() {
        Some(("sync", _)) => {
            println!("Syncing directory...");
            let mynk_path = find_mynk_root();
            if let Some(path) = mynk_path {
                println!("Found .mynk at: {}", path.display());
                let uri = fs::read_to_string(&path)?;
                sync_files(uri).await?;
            } else {
                println!("No .mynk file found. Please run 'mynk init --uri <URI>' first.");
            }
        }
        Some(("init", init_matches)) => {
            if let Some(uri) = init_matches.get_one::<String>("uri") {
                println!("Initiating for {uri}...");
                create_root(uri.to_string())?;
                create_root_state()?;
                sync_files(uri.to_string()).await?;
            } else {
                println!("Invalid uri.");
            }
        }
        _ => unreachable!(),
    }

    Ok(())
}

async fn sync_files(uri: String) -> Result<(), Box<dyn Error>> {
    build_state()?;

    let sync_uri = format!("{}/sync", uri);
    let summary_json: Value = build_post()?;

    let client = reqwest::Client::new();
    let resp = client.post(&sync_uri).json(&summary_json).send().await?;

    if resp.status().is_success() {
        let resp_json = resp.json::<serde_json::Value>().await?;
        handle_response(resp_json).await?;
    } else {
        let status = resp.status();
        let text = resp.text().await?;
        eprintln!("Sync failed: {} - {}", status, text);
        return Err(format!("Sync failed with status: {} - {}", status, text).into());
    }

    Ok(())
}

async fn handle_response(resp_json: Value) -> std::io::Result<()> {
    let root_dir = find_mynk_root_dir().expect("Could not find .mynk root");
    let state_file_path = find_mynk_state_root().expect("Could not find .mynk-state.json");

    Ok(())
}

fn find_mynk_root_dir() -> Option<PathBuf> {
    let current_dir = std::env::current_dir().ok()?;
    for dir in current_dir.ancestors() {
        let candidate = dir.join(".mynk");
        if candidate.exists() {
            return Some(dir.to_path_buf());
        }
    }
    None
}

fn find_mynk_root() -> Option<PathBuf> {
    let current_dir = std::env::current_dir().ok()?;
    for dir in current_dir.ancestors() {
        let candidate = dir.join(".mynk");
        if candidate.exists() {
            return Some(candidate);
        }
    }
    None
}

fn find_mynk_state_root() -> Option<PathBuf> {
    let current_dir = std::env::current_dir().ok()?;
    for dir in current_dir.ancestors() {
        let candidate = dir.join(".mynk-state.json");
        if candidate.exists() {
            return Some(candidate);
        }
    }
    None
}

fn create_root(uri: String) -> std::io::Result<()> {
    let mut file = File::create_new(".mynk")?;
    write!(file, "{}", uri)?;
    Ok(())
}

fn create_root_state() -> std::io::Result<()> {
    File::create_new(".mynk-state.json")?;
    Ok(())
}

fn build_state() -> Result<(), Box<dyn std::error::Error>> {
    let state_file_path = find_mynk_state_root().expect("Could not find .mynk-state.json");
    let root_dir = find_mynk_root_dir().expect("Could not find .mynk root");

    Ok(())
}

fn build_post() -> std::io::Result<Value> {
    let state_file_path = find_mynk_state_root().expect("Could not find .mynk-state.json");
    let root_dir = find_mynk_root_dir().expect("Could not find .mynk root");

    let new_vec: Vec<FileEntry> = {
        let file = File::open(&state_file_path)?;
        serde_json::from_reader(file)?
    };

    let files_array: Vec<Value> = new_vec
        .iter()
        .filter(|entry| entry.action != "pass")
        .map(|entry| {
            let contents = if entry.action != "delete" {
                let file_path = root_dir.join(&entry.filename);
                fs::read_to_string(&file_path).unwrap_or_default()
            } else {
                String::new()
            };

            json!({
                "filename": entry.filename,
                "contents": contents,
                "hash": entry.hash,
                "action": entry.action,
                "version": entry.version
            })
        })
        .collect();

    let summary_array: Vec<Value> = new_vec
        .iter()
        .map(|entry| {
            json!({
                "filename": entry.filename,
                "hash": entry.hash,
                "version": entry.version // Fixed to include version
            })
        })
        .collect();

    println!(
        "this is the post {}",
        json!({
            "files": files_array,
            "summary": summary_array
        })
    );

    Ok(json!({
        "files": files_array,
        "summary": summary_array
    }))
}
