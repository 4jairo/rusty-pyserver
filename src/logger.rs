use std::{collections::{HashMap, VecDeque}, fs::File, io::Write, net::SocketAddr, sync::{mpsc, OnceLock}, time::{Duration, Instant}};
use crossterm::{cursor::{self, MoveToColumn, MoveUp}, execute, style::{Color, Print, ResetColor, SetForegroundColor}, terminal::{Clear, ClearType}};
use crate::{html::format_file_size, LOG_FILE};

macro_rules! ___log_msg {
    ($tracker:expr ; $($args:expr),+ $(,)?) => {{
        crossterm::execute!(
            std::io::stdout(),

            // move 1 up, clear line, print log_msg
            crossterm::cursor::MoveToColumn(0),
            crossterm::cursor::MoveUp(1),
            crossterm::terminal::Clear(crossterm::terminal::ClearType::CurrentLine),
            $( $args, )*
            crossterm::style::Print("\n"),
            
            // print stats line, move 1 down (\n)
            crossterm::terminal::Clear(crossterm::terminal::ClearType::CurrentLine),
            crossterm::style::SetForegroundColor(crossterm::style::Color::DarkGrey),
            crossterm::style::Print($tracker.get_print_stats()),
            crossterm::style::ResetColor,
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



#[derive(Debug)]
pub struct Stats {
    pub requests: u32,
    pub who: SocketAddr,
    pub bandwith: HashMap<String, BandwithTracker>,
}
impl Stats {
    pub fn with_who(who: SocketAddr) -> Self {
        Self {
            requests: 0,
            bandwith: HashMap::default(),
            who
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct RequestInfo {
    pub request_id: usize,
    pub listener: u16
}
impl RequestInfo {
    pub fn new(request_id: usize, listener: u16) -> Self {
        Self {
            request_id, listener
        }
    }
}

#[derive(Default, Debug)]
pub struct RequestsTracker {
    total_requests: u32,
    requests: HashMap<u16, HashMap<usize, Stats>>
}
impl RequestsTracker {
    pub fn sended_bytes(&mut self, req_info: RequestInfo, b: u32, file: String) {
        let listener = self.requests.entry(req_info.listener).or_default();
        if let Some(stats) = listener.get_mut(&req_info.request_id) {

            match stats.bandwith.get_mut(&file) {
                None => {
                    let mut bt = BandwithTracker::default();
                    bt.add_bytes(b);
                    stats.bandwith.insert(file, bt);
                    self.total_requests += 1;
                },
                Some(bt) => bt.add_bytes(b)
            }
        }
    }

    pub fn ended_file(&mut self, req_info: RequestInfo, file: String) {
        if let Some(listener) = self.requests.get_mut(&req_info.listener) {
            if let Some(stats) = listener.get_mut(&req_info.request_id) {
                stats.bandwith.remove(&file);
            }
        }
    }
    
    pub fn new_request(&mut self, req_info: RequestInfo, who: SocketAddr) {
        let listener = self.requests.entry(req_info.listener).or_default();
        let stats = listener.entry(req_info.request_id).or_insert(Stats::with_who(who));
        
        stats.requests += 1;
        self.total_requests += 1;
    }

    pub fn request_ended(&mut self, req_info: RequestInfo) {
        if let Some(listener) = self.requests.get_mut(&req_info.listener) {
            listener.remove(&req_info.request_id);
        }
    }

    pub fn current_requests(&self) -> u32 {
        let mut total = 0;

        for inner in self.requests.values() {
            for stats in inner.values() {
                total += stats.requests
            }
        }
        total
    }

    pub fn print_details(&mut self) {
        let mut stdout = std::io::stdout();
        let tab = "    ";

        let _ = execute!(
            stdout,
            MoveToColumn(0),
            MoveUp(1),
            Clear(ClearType::CurrentLine),
        );

        for (listener, requests) in self.requests.iter_mut() {
            let _ = execute!(
                stdout,
                Print(format_args!("Listener {}:\n", listener)),
            );

            for (request_id, stats) in requests {
                let _ = execute!(
                    stdout,
                    SetForegroundColor(Color::Blue),
                    Print(format_args!("{tab}Request Id: {}, From: {}, Total Requests: {}\n", request_id, stats.who, stats.requests)),
                    ResetColor
                );

                for (file, tracker) in stats.bandwith.iter_mut() {
                    let _ = execute!(
                        stdout,
                        SetForegroundColor(Color::Green),
                        Print(format_args!("{tab}{tab}File: {}, Bandwidth: {}/s\n", file, format_file_size(tracker.get_bandwith()))),
                        ResetColor
                    );
                }
            }
        }

        let _ = execute!(
            stdout,
            SetForegroundColor(Color::DarkGrey),
            Print(self.get_print_stats()),
            ResetColor,
        );
    }

    pub fn get_print_stats(&mut self) -> String {
        let bw = format_file_size(self.get_bandwith());
        let current_requests = self.current_requests();

        format!(
            "Total requests: {} | Current requests: {} | Bytes/s: {}/s (press 'enter' for detailed stats)\n",
            self.total_requests, current_requests, bw
        )
    } 

    pub fn get_bandwith(&mut self) -> u64 {
        let mut total = 0;

        for inner in self.requests.values_mut() {
            for stats in inner.values_mut() {
                for bt in stats.bandwith.values_mut() {
                    total += bt.get_bandwith();   
                }
            }
        }
        total
    }
}

#[derive(Default, Debug)]
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
    ShowDetails,
    SendedBytes(RequestInfo, u32, String),
    EndedFile(RequestInfo, String),
    NewRequest(RequestInfo, SocketAddr),
    RequestEnded(RequestInfo)
} 

// #[derive(Debug)]
// pub struct SendedBytesMsg {
//     request_info: RequestInfo,
//     bytes: u32,

// }

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

fn print_stats(tracker: &mut RequestsTracker) {
    let _ = execute!(std::io::stdout(), 

        // Move 1 up and clear stats line
        cursor::MoveUp(1), 
        Clear(ClearType::CurrentLine),

        SetForegroundColor(Color::DarkGrey),

        // Print updated stats && move 1 down
        Print(tracker.get_print_stats()),
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
        // let mut stats = Stats::default();
        let mut tracker = RequestsTracker::default();
        let mut logs_file = None;

        while let Ok(msg) = rx.recv() {
            match msg {
                LogMsg::Error(e, exit, code) => {
                    log_request(&mut logs_file, &e);

                    let r = ___log_msg!(
                        tracker;
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
                        tracker;
                        crossterm::style::SetForegroundColor(crossterm::style::Color::Yellow),
                        crossterm::style::Print("ℹ️ "),  
                        crossterm::style::Print(i),
                    );
                }
                LogMsg::Request(r) => {
                    log_request(&mut logs_file, &r);

                    let _ = ___log_msg!(
                        tracker;
                        crossterm::style::Print(r),
                    );                
                },
                LogMsg::Stats(s) => match s {
                    StatsMsg::NewRequest(req_info, who) => {
                        tracker.new_request(req_info, who);
                    }
                    StatsMsg::RequestEnded(req_info) => {
                        tracker.request_ended(req_info);
                    },
                    StatsMsg::SendedBytes(req_info, b, file) => {
                        tracker.sended_bytes(req_info, b, file);
                    },
                    StatsMsg::EndedFile(req_info, file) => {
                        tracker.ended_file(req_info, file);
                    },
                    StatsMsg::Refresh => {
                        print_stats(&mut tracker);
                    },
                    StatsMsg::ShowDetails => {
                        tracker.print_details();
                    }
                }
            }
        }
    });
}