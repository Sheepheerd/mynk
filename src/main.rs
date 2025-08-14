use clap::{Arg, Command};

fn main() {
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
        }
        Some(("init", init_matches)) => {
            if let Some(uri) = init_matches.get_one::<String>("uri") {
                println!("Initiating for {uri}...");
            } else {
                println!("Invalid uri.");
            }
        }
        _ => unreachable!(),
    }
}
