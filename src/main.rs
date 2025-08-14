use clap::{Arg, Command};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::error::Error;
use std::fs;
use std::io;
use std::{fs::File, io::Write};

#[derive(Serialize, Deserialize)]
struct StateEntry {
    version: u32,
    hash: String,
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
            let uri = fs::read_to_string(".mynk")?;
            sync_files(uri.to_string()).await?;
        }
        Some(("init", init_matches)) => {
            if let Some(uri) = init_matches.get_one::<String>("uri") {
                println!("Initiating for {uri}...");

                create_root(uri.to_string());

                sync_files(uri.to_string()).await?;
            } else {
                println!("Invalid uri.");
            }
        }
        _ => unreachable!(),
    }

    Ok(())
}

fn create_root(uri: String) {
    match File::create_new(".mynk") {
        Ok(mut file) => {
            write!(file, "{uri}").ok();
        }
        Err(err) => {
            eprintln!("Error creating .mynk file: {err}");
        }
    }
}

async fn sync_files(uri: String) -> Result<(), Box<dyn Error>> {
    let sync_uri = format!("{}/sync", uri);

    let summary_json: Value = build_post(".")?;

    Ok(())
}

fn build_post(root: &str) -> io::Result<Value> {
    let files_array: String = "goober".to_string();
    let summary_array: String = "goober".to_string();
    Ok(json!({
        "files": files_array,
        "summary": summary_array
    }))
}
