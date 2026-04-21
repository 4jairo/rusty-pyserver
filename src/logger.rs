use std::{borrow::Cow, collections::{BTreeMap, HashMap, VecDeque}, fs::File, io::Write, net::SocketAddr, sync::{mpsc, OnceLock}, time::{Duration, Instant}};
use crossterm::{cursor::{MoveDown, MoveToColumn, MoveUp}, execute, style::{Color, Print, ResetColor, SetForegroundColor}, terminal::{self, Clear, ClearType}};
use hyper::Method;
use crate::{html::format_file_size, LOG_FILE, SPA_FILE};

macro_rules! ___log_msg {
    ($tracker:expr ; $($args:expr),+ $(,)?) => {{
        $tracker.clear_rendered_stats();

        let result = crossterm::execute!(
            std::io::stdout(),
            crossterm::cursor::MoveToColumn(0),
            $( $args, )*
            crossterm::style::Print("\n"),
        );

        if result.is_ok() {
            $tracker.print_rendered_stats();
        }

        result
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


#[derive(Debug)]
pub struct Stats {
    pub who: SocketAddr,
    pub bandwith: HashMap<String, BandwithTracker>,
}
impl Stats {
    pub fn with_who(who: SocketAddr) -> Self {
        Self {
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
    requests: HashMap<u16, HashMap<usize, Stats>>,
    show_detailed_stats: bool,
    rendered_stats_lines: u16,
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
        if !listener.contains_key(&req_info.request_id) {
            listener.insert(req_info.request_id, Stats::with_who(who));
        }
    }

    pub fn request_ended(&mut self, req_info: RequestInfo) {
        if let Some(listener) = self.requests.get_mut(&req_info.listener) {
            listener.remove(&req_info.request_id);
        }
    }

    pub fn active_requests(&self) -> u32 {
        let mut total = 0;

        for inner in self.requests.values() {
            for stats in inner.values() {
                total += stats.bandwith.len() as u32;
            }
        }
        total
    }

    pub fn connected_clients(&self) -> u32 {
        let mut total = 0;

        for inner in self.requests.values() {
            total += inner.len() as u32;
        }
        total
    }

    fn get_requester_lines(&mut self) -> Vec<String> {
        let mut by_requester: BTreeMap<SocketAddr, (u32, u32, u64)> = BTreeMap::new();

        for requests in self.requests.values_mut() {
            for stats in requests.values_mut() {
                let requester = by_requester.entry(stats.who).or_insert((0, 0, 0));
                requester.0 += 1;

                for tracker in stats.bandwith.values_mut() {
                    requester.1 += 1;
                    requester.2 += tracker.get_bandwith();
                }
            }
        }

        by_requester
            .into_iter()
            .map(|(requester, (connections, active_files, bandwidth))| {
                format!(
                    "Requester {} | connections: {} | active files: {} | bytes/s: {}/s",
                    requester,
                    connections,
                    active_files,
                    format_file_size(bandwidth),
                )
            })
            .collect()
    }

    pub fn clear_rendered_stats(&mut self) {
        let mut stdout = std::io::stdout();
        let lines_to_clear = self.rendered_stats_lines.max(1);

        let _ = execute!(
            stdout,
            MoveUp(lines_to_clear),
        );

        for i in 0..lines_to_clear {
            let _ = execute!(
                stdout,
                MoveToColumn(0),
                Clear(ClearType::CurrentLine),
            );

            if i + 1 < lines_to_clear {
                let _ = execute!(stdout, MoveDown(1));
            }
        }

        if lines_to_clear > 1 {
            let _ = execute!(stdout, MoveUp(lines_to_clear - 1));
        }
    }

    pub fn print_rendered_stats(&mut self) {
        let mut stdout = std::io::stdout();
        let terminal_width = terminal::size().map(|(w, _)| w as usize).unwrap_or(120);
        let summary = fit_to_terminal_width(&self.get_print_stats(), terminal_width);

        let _ = execute!(
            stdout,
            SetForegroundColor(Color::DarkGrey),
            Print(summary),
            Print("\n"),
            ResetColor,
        );

        let mut rendered_lines = 1;

        if self.show_detailed_stats {
            for line in self.get_requester_lines() {
                let line = fit_to_terminal_width(&line, terminal_width);
                let _ = execute!(
                    stdout,
                    SetForegroundColor(Color::Blue),
                    Print(line),
                    Print("\n"),
                    ResetColor,
                );
                rendered_lines += 1;
            }
        }

        self.rendered_stats_lines = rendered_lines;
    }

    pub fn get_print_stats(&mut self) -> String {
        let bw = format_file_size(self.get_bandwith());
        let active = self.active_requests();
        let connected = self.connected_clients();
        let details = if self.show_detailed_stats { "on" } else { "off" };

        format!(
            "Requests: (total: {} | active: {} | connected: {} | Bytes/s: {}/s | details: {}). Press 'enter' to toggle detailed stats",
            self.total_requests, active, connected, bw, details
        )
    } 

    pub fn toggle_detailed_stats(&mut self) {
        self.show_detailed_stats = !self.show_detailed_stats;
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

pub enum RequestKind {
    Spa,
    Html,
    DirToZip,
    Upload(String),
    Default,
    NotFound
}

pub enum LogMsg {
    Error(String, bool, i32),
    Info(String),
    Request(String, RequestKind),
    Stats(StatsMsg),
}

pub static LOGGER: OnceLock<mpsc::Sender<LogMsg>> = OnceLock::new();


pub fn update_stats(msg: StatsMsg) {
    if let Some(tx) = LOGGER.get() {
        let _ = tx.send(LogMsg::Stats(msg));
    }
}

pub fn print_request(kind: RequestKind, who: SocketAddr, method: Method, path: &Cow<'_, str>, listener: u16) {
    if let Some(tx) = LOGGER.get() {
        let now = chrono::Local::now().format("%d-%m-%Y %H:%M:%S");
        let msg = format!("[{now}] {who} -> :{listener} -> {method} {path}");

        let _ = tx.send(LogMsg::Request(msg, kind));
    }
}

fn print_stats(tracker: &mut RequestsTracker) {
    tracker.clear_rendered_stats();
    tracker.print_rendered_stats();
}

fn fit_to_terminal_width(line: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }

    let line_len = line.chars().count();
    if line_len <= width {
        return line.to_string();
    }

    if width <= 3 {
        return ".".repeat(width);
    }

    let keep = width - 3;
    let mut out = String::with_capacity(width);
    for ch in line.chars().take(keep) {
        out.push(ch);
    }
    out.push_str("...");
    out
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
                LogMsg::Request(msg, kind) => {
                    log_request(&mut logs_file, &msg);

                    match kind {
                        RequestKind::Default => {
                            let _ = ___log_msg!(
                                tracker;
                                crossterm::style::Print(msg),
                            );
                        },
                        RequestKind::NotFound => {
                            let _ = ___log_msg!(
                                tracker;
                                crossterm::style::Print(msg),
                                crossterm::style::SetForegroundColor(crossterm::style::Color::Red),
                                crossterm::style::Print(" (not found)")
                            );
                        },
                        RequestKind::DirToZip => {
                            let _ = ___log_msg!(
                                tracker;
                                crossterm::style::Print(msg),
                                crossterm::style::SetForegroundColor(crossterm::style::Color::Green),
                                crossterm::style::Print(" (zip)"),
                            );
                        },
                        RequestKind::Html => {
                            let _ = ___log_msg!(
                                tracker;
                                crossterm::style::Print(msg),
                                crossterm::style::SetForegroundColor(crossterm::style::Color::Green),
                                crossterm::style::Print(" (index.html)"),
                            );
                        },
                        RequestKind::Spa => {
                            let spa = unsafe { SPA_FILE.as_ref().unwrap_unchecked() };
                            let spa_file_name = spa.file_name().and_then(|n| n.to_str()).unwrap_or("unknown");

                            let _ = ___log_msg!(
                                tracker;
                                crossterm::style::Print(msg),
                                crossterm::style::SetForegroundColor(crossterm::style::Color::Green),
                                crossterm::style::Print(format_args!(" ({})", spa_file_name)),
                            );
                        },
                        RequestKind::Upload(details) => {
                            let _ = ___log_msg!(
                                tracker;
                                crossterm::style::Print(msg),
                                crossterm::style::SetForegroundColor(crossterm::style::Color::Green),
                                crossterm::style::Print(format_args!(" (upload: {})", details)),
                            );
                        },
                    }
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
                        tracker.toggle_detailed_stats();
                        print_stats(&mut tracker);
                    }
                }
            }
        }
    });
}