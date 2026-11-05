use std::{
    collections::BTreeMap,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
};
#[derive(Clone)]
pub struct Response {
    pub status: u16,
    pub body: Vec<u8>,
    pub headers: Vec<(String, String)>,
}

pub struct Server {
    base: String,
    routes: Arc<Mutex<BTreeMap<String, Response>>>,
    requests: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}
impl Server {
    pub fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let routes = Arc::new(Mutex::new(BTreeMap::<String, Response>::new()));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (worker_routes, worker_requests, worker_stop) =
            (routes.clone(), requests.clone(), stop.clone());
        let handle = thread::spawn(move || {
            for incoming in listener.incoming() {
                let Ok(mut socket) = incoming else { break };
                if worker_stop.load(Ordering::Acquire) {
                    break;
                }
                let mut raw = [0; 8192];
                let Ok(size) = socket.read(&mut raw) else {
                    continue;
                };
                let path = String::from_utf8_lossy(&raw[..size])
                    .split_whitespace()
                    .nth(1)
                    .unwrap_or("/")
                    .to_owned();
                worker_requests.lock().unwrap().push(path.clone());
                let response =
                    worker_routes
                        .lock()
                        .unwrap()
                        .get(&path)
                        .cloned()
                        .unwrap_or(Response {
                            status: 404,
                            body: b"missing".to_vec(),
                            headers: Vec::new(),
                        });
                let reason = if response.status == 200 {
                    "OK"
                } else {
                    "ERROR"
                };
                write!(
                    socket,
                    "HTTP/1.1 {} {reason}\r\nContent-Length: {}\r\nConnection: close\r\n",
                    response.status,
                    response.body.len()
                )
                .unwrap();
                for (name, value) in response.headers {
                    write!(socket, "{name}: {value}\r\n").unwrap();
                }
                socket.write_all(b"\r\n").unwrap();
                socket.write_all(&response.body).unwrap();
            }
        });
        Self {
            base: format!("http://{address}"),
            routes,
            requests,
            stop,
            handle: Some(handle),
        }
    }
    pub fn base(&self) -> &str {
        &self.base
    }
    pub fn route(&self, path: impl Into<String>, status: u16, body: impl Into<Vec<u8>>) {
        self.routes.lock().unwrap().insert(
            path.into(),
            Response {
                status,
                body: body.into(),
                headers: Vec::new(),
            },
        );
    }
    pub fn requests(&self) -> Vec<String> {
        self.requests.lock().unwrap().clone()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        let address = self.base.trim_start_matches("http://");
        let _ = TcpStream::connect(address);
        if let Some(handle) = self.handle.take() {
            handle.join().unwrap();
        }
    }
}
