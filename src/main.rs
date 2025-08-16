use clap::{Arg, Command};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::collections::HashSet;
use std::error::Error;
use std::fs;
use std::io;
use std::path::PathBuf;
use std::process::exit;
use std::{fs::File, io::Write};
use walkdir::WalkDir;

#[derive(Debug, Serialize, Deserialize)]
struct FileEntry {
    filename: String,
    hash: String,
    version: u32,
    action: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let matches = Command::new("mynk")
        .about("syncronizes directory")
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
                .about("Synchronize directory."),
        )
        .get_matches();

    match matches.subcommand() {
        Some(("sync", _)) => {
            println!("Syncing directory...");
            // walk the dir up till you find .mynk
            let mynk_path = find_mynk_root();
            if let Some(path) = mynk_path {
                println!("Found .mynk at: {}", path.display());
                let uri = fs::read_to_string(&path)?;
                sync_files(uri.to_string()).await?;
            } else {
                println!("No .mynk file found. Please run 'mynk init --uri <URI>' first.");
            }
        }
        Some(("init", init_matches)) => {
            if let Some(uri) = init_matches.get_one::<String>("uri") {
                println!("Initiating for {uri}...");

                create_root(uri.to_string());
                create_root_state();
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
    compare_state();

    let sync_uri = format!("{}/sync", uri);

    let summary_json: Value = build_post()?;

    let client = reqwest::Client::new();
    let resp = client.post(&sync_uri).json(&summary_json).send().await?;

    if resp.status().is_success() {
        let resp_json = resp.json::<serde_json::Value>().await?;
        handle_response(resp_json).await?;
    } else {
        eprintln!("Sync failed: {}", resp.status());
    }

    Ok(())
}

async fn handle_response(resp_json: Value) -> std::io::Result<()> {
    let root_dir = find_mynk_root_dir().expect("Could not find .mynk root");

    if let Value::Array(files) = resp_json {
        for file_obj in files {
            if let Value::Object(map) = file_obj {
                let filename = map
                    .get("filename")
                    .and_then(Value::as_str)
                    .expect("filename missing");
                let contents = map.get("contents").and_then(Value::as_str).unwrap_or("");

                let full_path = root_dir.join(filename);

                if let Some(parent) = full_path.parent() {
                    fs::create_dir_all(parent)?;
                }

                fs::write(&full_path, contents)?;
                println!("Wrote file: {}", full_path.display());
            }
        }
    } else {
        eprintln!("Expected JSON array in response");
    }
    Ok(())
}

fn find_mynk_root_dir() -> Option<PathBuf> {
    let current_dir = std::env::current_dir().ok()?;
    let mut mynk_dir: Option<PathBuf> = None;
    for dir in current_dir.ancestors() {
        let candidate = dir.join(".mynk");
        if candidate.exists() {
            mynk_dir = Some(dir.to_path_buf());
            break;
        }
    }
    mynk_dir
}

fn find_mynk_root() -> Option<PathBuf> {
    let current_dir = std::env::current_dir().ok()?;
    let mut mynk_path: Option<PathBuf> = None;
    for dir in current_dir.ancestors() {
        let candidate = dir.join(".mynk");
        if candidate.exists() {
            mynk_path = Some(candidate);
            break;
        }
    }
    mynk_path
}
fn find_mynk_state_root() -> Option<PathBuf> {
    let current_dir = std::env::current_dir().ok()?;
    let mut mynk_state_path: Option<PathBuf> = None;
    for dir in current_dir.ancestors() {
        let candidate = dir.join(".mynk-state.json");
        if candidate.exists() {
            mynk_state_path = Some(candidate);
            break;
        }
    }
    mynk_state_path
}

fn create_root(uri: String) {
    match File::create_new(".mynk") {
        Ok(mut file) => {
            write!(file, "{uri}").ok();
        }
        Err(err) => {
            eprintln!("Error creating .mynk file: {err}");
            exit(0);
        }
    }
}
fn create_root_state() {
    match File::create_new(".mynk-state.json") {
        Ok(mut file) => {}
        Err(err) => {
            eprintln!("Error creating .mynk-state file: {err}");
        }
    }
}

fn compare_state() {
    let mynk_state_path = find_mynk_state_root();

    let state_file_path = match mynk_state_path {
        Some(path) => path,
        None => {
            eprintln!("No .mynk-state.json file found. Please run 'mynk init --uri <URI>' first.");
            return;
        }
    };

    let metadata = std::fs::metadata(&state_file_path).expect("Failed to read file metadata");
    let old_vec: Vec<FileEntry> = if metadata.len() == 0 {
        Vec::new()
    } else {
        let file = File::open(&state_file_path).expect("Failed to open .mynk-state.json");
        serde_json::from_reader(file).expect("Failed to parse .mynk-state.json")
    };

    let root_dir = match find_mynk_root_dir() {
        Some(dir) => dir,
        None => {
            eprintln!("No root dir");
            return;
        }
    };

    let mut new_vec = Vec::<FileEntry>::new();
    for entry in WalkDir::new(root_dir.clone())
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter(|e| !e.path().starts_with(root_dir.join(".mynk")))
        .filter(|e| !e.path().ends_with(".mynk-state.json"))
    {
        let bytes = std::fs::read(entry.path()).unwrap();
        let hash = sha256::digest(&bytes);

        // Get relative path from root_dir
        let rel_path = entry
            .path()
            .strip_prefix(&root_dir)
            .unwrap()
            .to_string_lossy()
            .to_string();

        new_vec.push(FileEntry {
            filename: rel_path,
            hash,
            version: 1,
            action: "create".to_string(),
        });
    }
    let updated_vec: Vec<FileEntry> = if old_vec.is_empty() {
        new_vec
    } else {
        let old_map: HashMap<String, FileEntry> = old_vec
            .into_iter()
            .map(|entry| (entry.filename.clone(), entry))
            .collect();

        let new_map: HashMap<String, FileEntry> = new_vec
            .into_iter()
            .map(|entry| (entry.filename.clone(), entry))
            .collect();

        compare_state_keys(new_map, &old_map)
            .into_values()
            .collect()
    };

    let json = serde_json::to_string_pretty(&updated_vec).expect("Failed to serialize new state");
    let mut file =
        File::create(&state_file_path).expect("Failed to open .mynk-state.json for writing");
    file.write_all(json.as_bytes())
        .expect("Failed to write updated state");
}

fn compare_state_keys(
    mut new_map: HashMap<String, FileEntry>,
    old_map: &HashMap<String, FileEntry>,
) -> HashMap<String, FileEntry> {
    let old_keys: HashSet<_> = old_map.keys().cloned().collect();
    let new_keys: HashSet<_> = new_map.keys().cloned().collect();

    // Union of all filenames
    let all_keys: HashSet<_> = old_keys.union(&new_keys).cloned().collect();

    for filename in all_keys {
        match (new_map.get_mut(&filename), old_map.get(&filename)) {
            (Some(new_entry), None) => {
                new_entry.action = "create".to_string();
            }
            (None, Some(old_entry)) => {
                new_map.insert(
                    filename.clone(),
                    FileEntry {
                        filename: filename.clone(),
                        hash: old_entry.hash.clone(),
                        version: old_entry.version,
                        action: "delete".to_string(),
                    },
                );
            }
            (Some(new_entry), Some(old_entry)) => {
                if old_entry.hash != new_entry.hash {
                    new_entry.action = "edit".to_string();
                    new_entry.version = old_entry.version + 1;
                } else {
                    new_entry.action = "".to_string();
                    new_entry.version = old_entry.version;
                }
            }
            _ => {}
        }
    }

    new_map
}

fn build_post() -> io::Result<Value> {
    let mynk_state_path = find_mynk_state_root();

    let state_file_path = match mynk_state_path {
        Some(path) => path,
        None => {
            eprintln!("No .mynk-state.json file found. Please run 'mynk init --uri <URI>' first.");
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "No .mynk-state.json file found",
            ));
        }
    };

    let new_vec: Vec<FileEntry> = {
        let file = File::open(&state_file_path).expect("Failed to open .mynk-state.json");
        serde_json::from_reader(file).expect("Failed to parse .mynk-state.json")
    };

    let root_dir = state_file_path
        .parent()
        .expect("State file should have a parent directory");

    let files_array: Vec<Value> = new_vec
        .iter()
        .map(|entry| {
            let file_path = root_dir.join(&entry.filename);
            let content = std::fs::read_to_string(&file_path).unwrap_or_default();

            json!({
                "filename": entry.filename,
                "version": entry.version,
                "contents": content,
                "hash": entry.hash,
                "action": entry.action
            })
        })
        .collect();

    let summary_array: Vec<Value> = new_vec
        .iter()
        .map(|entry| {
            json!({
                "filename": entry.filename,
                "hash": entry.hash
            })
        })
        .collect();
    println!(
        "{}",
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
