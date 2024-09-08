use std::{collections::VecDeque, fs::File, io::Write, sync::{mpsc, OnceLock}, time::{Duration, Instant}};
use crossterm::{cursor, execute, style::{Color, Print, ResetColor, SetForegroundColor}, terminal::{Clear, ClearType}};
use crate::{html::format_file_size, LOG_FILE};

macro_rules! ___log_msg {
    ($stats:expr ; $($args:expr),+ $(,)?) => {{
        crossterm::execute!(
            std::io::stdout(),

            // clear line and Print stats line
            crossterm::terminal::Clear(crossterm::terminal::ClearType::CurrentLine),
            crossterm::style::SetForegroundColor(crossterm::style::Color::DarkGrey),
            crossterm::style::Print(format_args!(
                "Total requests: {} | Current requests: {} | Bytes/s: {}/s",
                $stats.total_requests, $stats.requests, format_file_size($stats.bandwith.get_bandwith())
            )),
            crossterm::style::ResetColor,

            // move 1 up and clear the log line
            crossterm::cursor::MoveToColumn(0),
            crossterm::cursor::MoveUp(1),
            crossterm::terminal::Clear(crossterm::terminal::ClearType::CurrentLine),

            // Print log and move 2 down (original position)
            $( $args, )*
            
            crossterm::style::Print("\n\n"),
            crossterm::style::ResetColor
        )
    }};
}

macro_rules! print_error {
    ($exit_num:expr ; $($arg:tt)*) => {{
        if let Some(tx) = crate::logger::LOGGER.get() {
            let _ = tx.send(crate::logger::LogMsg::Error(format_args!($($arg)*).to_string(), true, $exit_num));
        }
    }};
    ($($arg:tt)*) => {{
        if let Some(tx) = crate::logger::LOGGER.get() {
            let _ = tx.send(crate::logger::LogMsg::Error(format_args!($($arg)*).to_string(), false, 0));
        }
    }};
}

macro_rules! print_info {
    ($($arg:tt)*) => {{
        if let Some(tx) = crate::logger::LOGGER.get() {
            let _ = tx.send(crate::logger::LogMsg::Info(format_args!($($arg)*).to_string()));
        }
    }};
}

macro_rules! print_request {
    ($($arg:tt)*) => {{
        if let Some(tx) = crate::logger::LOGGER.get() {
            let _ = tx.send(crate::logger::LogMsg::Request(format_args!($($arg)*).to_string()));
        }
    }};
}



#[derive(Default)]
pub struct Stats {
    pub requests: u32,
    pub total_requests: u32,
    pub bandwith: BandwithTracker,
}

#[derive(Default)]
pub struct BandwithTracker {
    timestamps: VecDeque<(Instant, u32)>
}
impl BandwithTracker {
    pub fn add_bytes(&mut self, bytes: u32) {
        self.timestamps.push_back((Instant::now(), bytes));
    }

    pub fn get_bandwith(&mut self) -> u64 {
        let now = Instant::now();

        while let Some((time, _)) = self.timestamps.front() {
            if (now - *time).as_millis() > 1000 {
                self.timestamps.pop_front();
            } else {
                break;
            }
        } 

        self.timestamps.iter().map(|(_,b)| *b as u64).sum::<u64>()
    }
}

#[derive(Debug)]
pub enum StatsMsg {
    Refresh,
    SendedBytes(u32),
    NewRequest,
    RequestEnded
}
pub enum LogMsg {
    Error(String, bool, i32),
    Info(String),
    Request(String),
    Stats(StatsMsg),
}

pub static LOGGER: OnceLock<mpsc::Sender<LogMsg>> = OnceLock::new();


pub fn update_stats(msg: StatsMsg) {
    if let Some(tx) = LOGGER.get() {
        let _ = tx.send(LogMsg::Stats(msg));
    }
}

fn print_stats(stats: &mut Stats) {
    let _ = execute!(std::io::stdout(), 

        // Move 1 up and clear stats line
        cursor::MoveUp(1), 
        Clear(ClearType::CurrentLine),

        SetForegroundColor(Color::DarkGrey),

        // Print updated stats && move 1 down
        Print(format_args!(
            "Total requests: {} | Current requests: {} | Bytes/s: {}/s\n",
            stats.total_requests, stats.requests, format_file_size(stats.bandwith.get_bandwith())
        )),
        ResetColor,
    );
}


fn log_request(file: &mut Option<File>, request: &String) {
    unsafe {
        if let Some(file_path) = LOG_FILE.as_ref() {
            if !file_path.exists() || file.is_none() {
                let new = File::create(file_path);
                if new.is_err() {
                    print_error!("Failed to create log file: {:?}", new);
                }
                *file = new.ok();
            }

            if let Some(file) = file.as_mut() {
                let _ = writeln!(file, "{}", request);
            }
        }
    }
}


pub fn init_stats_logger() {
    let (tx, rx) = mpsc::channel();
    if let Err(_) = LOGGER.set(tx.clone()) {
        eprintln!("Failed to set the logger");
    }

    std::thread::spawn(move || {
        loop {
            tx.send(LogMsg::Stats(StatsMsg::Refresh)).unwrap();
            std::thread::sleep(Duration::from_millis(300));
        }
    });

    std::thread::spawn(move || {
        let mut stats = Stats::default();
        let mut logs_file = None;

        while let Ok(msg) = rx.recv() {
            match msg {
                LogMsg::Error(e, exit, code) => {
                    log_request(&mut logs_file, &e);

                    let r = ___log_msg!(
                        stats;
                        crossterm::style::SetForegroundColor(crossterm::style::Color::Red),
                        crossterm::style::Print("⚠️ "),  
                        crossterm::style::Print(e),
                    );
                    if exit {
                        let _ = r.and_then::<(), _>(|_| std::process::exit(code));
                    }
                }
                LogMsg::Info(i) => {
                    log_request(&mut logs_file, &i);

                    let _ = ___log_msg!(
                        stats;
                        crossterm::style::SetForegroundColor(crossterm::style::Color::Yellow),
                        crossterm::style::Print("ℹ️ "),  
                        crossterm::style::Print(i),
                    );
                }
                LogMsg::Request(r) => {
                    log_request(&mut logs_file, &r);

                    let _ = ___log_msg!(
                        stats;
                        crossterm::style::Print(r),
                    );                
                },
                LogMsg::Stats(s) => match s {
                    StatsMsg::NewRequest => {
                        stats.requests += 1;
                        stats.total_requests += 1;
                    }
                    StatsMsg::RequestEnded => {
                        if stats.requests > 0 {
                            stats.requests -= 1;
                        }
                    },
                    StatsMsg::SendedBytes(b) => {
                        stats.bandwith.add_bytes(b);
                    },
                    StatsMsg::Refresh => {
                        print_stats(&mut stats);
                    },
                }
            }
        }
    });
}