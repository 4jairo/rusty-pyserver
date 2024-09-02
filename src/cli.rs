use std::{collections::HashSet, path::{Path, PathBuf}};
use clap::{Arg, Command, ValueHint};


pub struct CliArgs {
    pub show_html: bool,
    pub only_localhost: bool,
    pub spa_file: Option<PathBuf>,
    pub listen_ports: HashSet<u16>,
    pub log_file: Option<PathBuf>,
}

impl CliArgs {
    pub fn parse() -> Self {
        let matches = Command::new("fast_pyserver")
            .version("1.0")
            .about("http server that will serve static content to GET requests")
            .arg(
                Arg::new("ports")
                    .default_value("80")
                    .index(1)
                    .allow_negative_numbers(false)
                    .num_args(0..=u16::MAX as usize)
                    .help("Assigns single or multple ports to the http server")
            )
            .arg(
                Arg::new("html")
                    .long("html")
                    .help("The browser will show the content of the requested file instead of downloading it")
                    .num_args(0)
            )
            .arg(
                Arg::new("local")
                    .long("local")
                    .help("Uses only localhost instead of both your localhost and the local network IP addresses")
                    .num_args(0)
            )
            .arg(
                Arg::new("spa")
                    .long("spa")
                    .help("Serves the HTML file [default: index.html] located in the current directory when the requested URI points to a direcotry (will set the --html flag to true)")
                    .value_hint(ValueHint::FilePath)
                    .default_missing_value("index.html")
                    .num_args(0..=1)
            )
            .arg(
                Arg::new("log-file")
                    .long("log-file")
                    .help("Logs all requests to a file")
                    .value_hint(ValueHint::FilePath)
                    .num_args(1)
                    .default_missing_value("requests.log")
            )
            .get_matches();

        let spa_file = matches
            .get_one::<String>("spa")
            .map(|p| Path::new(".").join(p));
        
        let mut show_html = matches
            .get_one::<bool>("html")
            .cloned()
            .unwrap_or_default();

        if !show_html {
            show_html = spa_file.is_some();
        }
    
        let only_localhost = matches
            .get_one::<bool>("local")
            .cloned()
            .unwrap_or_default();
    
        let mut listen_ports = matches
            .get_many::<String>("ports")
            .unwrap_or_default()
            .filter_map(|p| {
                match p.parse::<u16>() {
                    Ok(port) => Some(port),
                    Err(_) => {
                        print_error!("-> [ports] Ignoring port `{p}`, valid ports: {} - {}", u16::MIN, u16::MAX);
                        None
                    }
                }
            })
            .collect::<HashSet<_>>();
    
        if listen_ports.len() == 0 {
            listen_ports.insert(80);
        }

        let log_file = matches
            .get_one::<String>("log-file")
            .map(PathBuf::from);
    
        Self {
            listen_ports,
            only_localhost,
            spa_file,
            show_html,
            log_file
        }
    }
}
