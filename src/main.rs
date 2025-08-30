use clap::{Arg, Command};
use reqwest::multipart::{Form, Part};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::error::Error;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use tokio::fs as tokio_fs;
use walkdir::WalkDir;

#[derive(Debug, Serialize, Deserialize, Clone)]
struct FileEntry {
    filename: String,
    hash: String,
    version: i32,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let matches = Command::new("mynk")
        .about("Synchronizes directory with server")
        .version("0.1.0")
        .subcommand_required(true)
        .arg_required_else_help(true)
        .subcommand(
            Command::new("init")
                .short_flag('I')
                .long_flag("init")
                .about("Initiate mynk with server URI")
                .arg(
                    Arg::new("uri")
                        .long("uri")
                        .value_name("URI")
                        .help("Server URI for synchronization")
                        .required(true),
                ),
        )
        .subcommand(
            Command::new("sync")
                .short_flag('S')
                .long_flag("sync")
                .about("Synchronize directory with server"),
        )
        .get_matches();

    match matches.subcommand() {
        Some(("init", init_matches)) => {
            let uri = init_matches.get_one::<String>("uri").unwrap();
            println!("Initializing for URI: {}", uri);
            create_root(uri)?;
            create_root_state()?;
            sync_files(uri).await?;
        }
        Some(("sync", _)) => {
            println!("Syncing directory...");
            let mynk_path = find_mynk_root();
            if let Some(path) = mynk_path {
                let uri = fs::read_to_string(&path)?;
                sync_files(&uri).await?;
            } else {
                return Err("No .mynk file found. Run 'mynk init --uri <URI>' first.".into());
            }
        }
        _ => unreachable!(),
    }

    Ok(())
}

fn create_root(uri: &str) -> std::io::Result<()> {
    let mut file = File::create_new(".mynk")?;
    write!(file, "{}", uri)?;
    Ok(())
}

fn create_root_state() -> std::io::Result<()> {
    let state = json!({ "files": [] });
    let mut file = File::create_new(".mynk-state.json")?;
    write!(file, "{}", state)?;
    Ok(())
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

fn compute_file_hash(path: &Path) -> std::io::Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0; 4096];
    loop {
        let bytes_read = file.read(&mut buffer)?;
        if bytes_read == 0 {
            break;
        }
        hasher.update(&buffer[..bytes_read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn build_local_state(root_dir: &Path) -> std::io::Result<Vec<FileEntry>> {
    let mut files = Vec::new();
    let state_file_path = root_dir.join(".mynk-state.json");
    let mynk_file = root_dir.join(".mynk");

    for entry in WalkDir::new(root_dir).into_iter().filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.is_file() && path != state_file_path && path != mynk_file {
            let relative_path = path
                .strip_prefix(root_dir)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            let hash = compute_file_hash(path)?;
            files.push(FileEntry {
                filename: relative_path,
                hash,
                version: 0,
            });
        }
    }

    let state_file_path = root_dir.join(".mynk-state.json");
    if state_file_path.exists() {
        let state: Value = serde_json::from_reader(File::open(&state_file_path)?)?;
        if let Some(existing_files) = state.get("files").and_then(|f| f.as_array()) {
            for file in files.iter_mut() {
                if let Some(existing) = existing_files
                    .iter()
                    .find(|e| e["filename"].as_str() == Some(&file.filename))
                {
                    file.version = existing["version"].as_i64().unwrap_or(0) as i32;
                }
            }
        }
    }

    Ok(files)
}

async fn sync_files(uri: &str) -> Result<(), Box<dyn Error>> {
    let root_dir = find_mynk_root_dir().ok_or("Could not find .mynk root")?;
    let state_file_path = find_mynk_state_root().ok_or("Could not find .mynk-state.json")?;

    let local_files = build_local_state(&root_dir)?;

    let client = reqwest::Client::new();
    let server_state: Value = client
        .get(format!("{}/structure", uri))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let server_files: Vec<FileEntry> = serde_json::from_value(server_state["files"].clone())?;

    let mut local_files_map: std::collections::HashMap<String, FileEntry> = local_files
        .clone()
        .into_iter()
        .map(|f| (f.filename.clone(), f))
        .collect();
    let server_files_map: std::collections::HashMap<String, FileEntry> = server_files
        .into_iter()
        .map(|f| (f.filename.clone(), f))
        .collect();

    let mut new_local_state = Vec::new();
    let mut to_upload = Vec::new();
    let mut to_delete_server: Vec<String> = Vec::new();
    let mut staged_files = Vec::new();

    for (filename, server_file) in &server_files_map {
        if let Some(local_file) = local_files_map.remove(filename) {
            if server_file.version > local_file.version {
                println!("Server has newer version for {}. Downloading.", filename);
                download_file(uri, &root_dir, filename).await?;
                new_local_state.push(server_file.clone());
            } else if server_file.version == local_file.version
                && local_file.hash != server_file.hash
            {
                println!(
                    "Local changes detected for {}. Queuing for upload.",
                    filename
                );
                to_upload.push(local_file.clone());
                new_local_state.push(local_file.clone());
            } else {
                new_local_state.push(local_file.clone());
                if local_file.hash != server_file.hash {
                    println!("Local file {} is newer. Queuing for upload.", filename);
                    to_upload.push(local_file.clone());
                }
            }
        } else {
            println!("New file on server: {}. Downloading.", filename);
            download_file(uri, &root_dir, filename).await?;
            new_local_state.push(server_file.clone());
        }
    }

    for (filename, local_file) in local_files_map {
        if server_files_map.contains_key(&filename) {
            continue;
        }
        if local_file.version == 0 {
            println!("New local file: {}. Staging for upload.", filename);
            staged_files.push(local_file.clone());
            new_local_state.push(local_file.clone());
        } else {
            println!("Deleting local file not on server: {}", filename);
            tokio_fs::remove_file(root_dir.join(&filename)).await?;
        }
    }

    for file in to_upload.iter() {
        upload_file(uri, &root_dir, file).await?;
        new_local_state.retain(|f| f.filename != file.filename);
        let updated_file = FileEntry {
            filename: file.filename.clone(),
            hash: file.hash.clone(),
            version: file.version + 1,
        };
        new_local_state.push(updated_file);
    }

    for file in staged_files {
        upload_file(uri, &root_dir, &file).await?;
        new_local_state.retain(|f| f.filename != file.filename);
        let updated_file = FileEntry {
            filename: file.filename.clone(),
            hash: file.hash.clone(),
            version: 1,
        };
        new_local_state.push(updated_file);
    }

    for (filename, _server_file) in server_files_map {
        if !local_files.iter().any(|f| f.filename == filename)
            && !new_local_state.iter().any(|f| f.filename == filename)
        {
            println!("Deleting file on server not present locally: {}", filename);
            to_delete_server.push(filename.clone());
        }
    }
    for filename in to_delete_server {
        delete_file(uri, &filename).await?;
    }

    let state = json!({ "files": new_local_state });
    tokio_fs::write(&state_file_path, serde_json::to_string_pretty(&state)?).await?;

    Ok(())
}

async fn download_file(uri: &str, root_dir: &Path, filename: &str) -> Result<(), Box<dyn Error>> {
    let client = reqwest::Client::new();
    let response = client
        .get(format!("{}/file/{}", uri, filename))
        .send()
        .await?
        .error_for_status()?;

    let content = response.json::<Value>().await?;
    let file_content = content["contents"]
        .as_str()
        .ok_or("Invalid file content response")?;

    let file_path = root_dir.join(filename);
    tokio_fs::create_dir_all(file_path.parent().ok_or("Invalid file path")?).await?;
    tokio_fs::write(&file_path, file_content).await?;
    println!("Downloaded file: {}", filename);

    Ok(())
}

async fn upload_file(uri: &str, root_dir: &Path, file: &FileEntry) -> Result<(), Box<dyn Error>> {
    let file_path = root_dir.join(&file.filename);
    let file_content = tokio_fs::read(&file_path).await?;
    let part = Part::bytes(file_content).file_name(file.filename.clone());
    let form = Form::new().part("file", part);

    let client = reqwest::Client::new();
    let response = client
        .post(format!("{}/upload", uri))
        .multipart(form)
        .send()
        .await?
        .error_for_status()?;

    let resp_json: Value = response.json().await?;
    println!(
        "Uploaded file: {}. Server response: {}",
        file.filename, resp_json
    );
    Ok(())
}

async fn delete_file(uri: &str, filename: &str) -> Result<(), Box<dyn Error>> {
    let client = reqwest::Client::new();
    let response = client
        .delete(format!("{}/delete", uri))
        .json(&json!({ "filename": filename }))
        .send()
        .await?
        .error_for_status()?;

    println!(
        "Deleted file on server: {}. Server response: {}",
        filename,
        response.json::<Value>().await?
    );
    Ok(())
}
