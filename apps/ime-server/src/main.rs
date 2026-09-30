//! `ime-server` — TCP IPC server for the LingXi pinyin engine.
//!
//! Listens on `127.0.0.1:9527`. A librime filter plugin (inside Weasel) sends
//! one JSON line per connection and gets one JSON line back. Also usable with
//! `curl` / `nc` for testing.
//!
//! # Protocol (newline-delimited JSON, one request per connection)
//!
//! ## Rerank — filter sends librime's raw candidates for context-aware reranking:
//! ```json
//! {"type":"rerank","candidates":["你好","你","好"],"context":"","limit":9}
//! ```
//!
//! ## Query — direct pinyin lookup (standalone / testing):
//! ```json
//! {"type":"query","pinyin":"nihao","context":"","limit":9}
//! ```
//!
//! ## Response:
//! ```json
//! {"candidates":[{"text":"你好","score":19.9},{"text":"你","score":9.5}]}
//! ```

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};

use serde::{Deserialize, Serialize};

use assistant_ime::{
    Candidate, CandidateReranker, InputContext, InputEngine, PinyinInputEngine,
    PrefixContextReranker,
};

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum Request {
    Rerank {
        candidates: Vec<String>,
        #[serde(default)]
        context: String,
        #[serde(default = "default_limit")]
        limit: usize,
    },
    Query {
        pinyin: String,
        #[serde(default)]
        context: String,
        #[serde(default = "default_limit")]
        limit: usize,
    },
}

fn default_limit() -> usize {
    9
}

#[derive(Serialize)]
struct CandidateView {
    text: String,
    score: f64,
}

#[derive(Serialize)]
struct Response {
    candidates: Vec<CandidateView>,
}

fn handle_request(engine: &PinyinInputEngine, request: &Request) -> Response {
    match request {
        Request::Query {
            pinyin,
            context,
            limit,
        } => {
            let ctx = InputContext {
                preceding_text: context.clone(),
                max_candidates: *limit,
            };
            let cands = engine.candidates(pinyin, &ctx);
            Response {
                candidates: cands
                    .into_iter()
                    .map(|c| CandidateView {
                        text: c.text,
                        score: c.score,
                    })
                    .collect(),
            }
        }
        Request::Rerank {
            candidates,
            context,
            limit,
        } => {
            let mut cands: Vec<Candidate> = candidates
                .iter()
                .enumerate()
                .map(|(i, text)| Candidate {
                    text: text.clone(),
                    syllables: vec![],
                    score: (candidates.len() - i) as f64,
                })
                .collect();
            let ctx = InputContext {
                preceding_text: context.clone(),
                max_candidates: *limit,
            };
            PrefixContextReranker::default().rerank(&mut cands, &ctx);
            if *limit > 0 && cands.len() > *limit {
                cands.truncate(*limit);
            }
            Response {
                candidates: cands
                    .into_iter()
                    .map(|c| CandidateView {
                        text: c.text,
                        score: c.score,
                    })
                    .collect(),
            }
        }
    }
}

fn handle_connection(engine: &PinyinInputEngine, stream: TcpStream) {
    let peer = stream.peer_addr().ok();
    let mut reader = BufReader::new(&stream);
    let mut line = String::new();
    if reader.read_line(&mut line).is_err() {
        return;
    }
    let response = match serde_json::from_str::<Request>(line.trim()) {
        Ok(req) => handle_request(engine, &req),
        Err(e) => Response {
            candidates: vec![CandidateView {
                text: format!("error: {e}"),
                score: 0.0,
            }],
        },
    };
    let mut out = serde_json::to_string(&response).unwrap();
    out.push('\n');
    let _ = (&stream).write_all(out.as_bytes());
    let _ = (&stream).flush();
    if let Some(peer) = peer {
        eprintln!("  handled request from {peer}");
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut dict_files: Vec<String> = Vec::new();
    let mut port = 9527u16;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--dict" => {
                i += 1;
                if i < args.len() {
                    dict_files.push(args[i].clone());
                }
            }
            "--port" => {
                i += 1;
                if i < args.len() {
                    port = args[i].parse().unwrap_or(9527);
                }
            }
            "-h" | "--help" => {
                println!("Usage: ime-server [--dict <FILE>]... [--port <PORT>]");
                println!("  --dict <FILE>  Load a rime-ice-style dictionary (repeatable)");
                println!("  --port <PORT>  TCP port (default 9527)");
                return;
            }
            _ => {}
        }
        i += 1;
    }

    let engine = if dict_files.is_empty() {
        PinyinInputEngine::builtin()
    } else {
        use assistant_ime::{Dictionary, PrefixContextReranker};
        let mut dict = Dictionary::new();
        for path in &dict_files {
            match dict.load_file(path) {
                Ok(0) => {
                    eprintln!("ime-server: dictionary contains no entries: {path}");
                    std::process::exit(1);
                }
                Ok(n) => println!("  loaded {n} entries from {path}"),
                Err(error) => {
                    eprintln!("ime-server: cannot load {path}: {error}");
                    std::process::exit(1);
                }
            }
        }
        println!("  total: {} dictionary entries", dict.entry_count());
        PinyinInputEngine::new(dict, Box::new(PrefixContextReranker::default()))
    };

    let bind = format!("127.0.0.1:{port}");
    let listener = TcpListener::bind(&bind).unwrap_or_else(|e| {
        eprintln!("ime-server: cannot bind {bind}: {e}");
        std::process::exit(1);
    });
    println!("LingXi IME Server");
    println!("  Listening: {bind}");
    println!("  Protocol: one JSON line in → one JSON line out (per connection)");
    println!("  Test: echo '{{\"type\":\"query\",\"pinyin\":\"nihao\"}}' | nc 127.0.0.1 {port}");
    println!("  Press Ctrl+C to stop.\n");

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => handle_connection(&engine, stream),
            Err(e) => eprintln!("  accept error: {e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_produces_candidates() {
        let engine = PinyinInputEngine::builtin();
        let req = Request::Query {
            pinyin: "nihao".into(),
            context: String::new(),
            limit: 5,
        };
        let resp = handle_request(&engine, &req);
        assert!(!resp.candidates.is_empty());
        assert_eq!(resp.candidates[0].text, "你好");
    }

    #[test]
    fn query_supports_simplified_pinyin() {
        let engine = PinyinInputEngine::builtin();
        for (pinyin, expected) in [("nh", "你好"), ("zgr", "中国人")] {
            let response = handle_request(
                &engine,
                &Request::Query {
                    pinyin: pinyin.into(),
                    context: String::new(),
                    limit: 9,
                },
            );
            assert!(
                response
                    .candidates
                    .iter()
                    .any(|candidate| candidate.text == expected),
                "{pinyin} should contain {expected}"
            );
        }
    }

    #[test]
    fn rerank_demotes_repetition() {
        let engine = PinyinInputEngine::builtin();
        let req = Request::Rerank {
            candidates: vec!["妈".into(), "吗".into(), "马".into()],
            context: "妈".into(),
            limit: 9,
        };
        let resp = handle_request(&engine, &req);
        assert_ne!(resp.candidates[0].text, "妈");
    }

    #[test]
    fn empty_query_returns_empty() {
        let engine = PinyinInputEngine::builtin();
        let req = Request::Query {
            pinyin: "".into(),
            context: String::new(),
            limit: 5,
        };
        let resp = handle_request(&engine, &req);
        assert!(resp.candidates.is_empty());
    }

    #[test]
    fn limit_is_respected() {
        let engine = PinyinInputEngine::builtin();
        let req = Request::Rerank {
            candidates: vec![
                "一".into(),
                "二".into(),
                "三".into(),
                "四".into(),
                "五".into(),
            ],
            context: String::new(),
            limit: 2,
        };
        let resp = handle_request(&engine, &req);
        assert!(resp.candidates.len() <= 2);
    }

    #[test]
    fn json_round_trip() {
        let json = r#"{"type":"query","pinyin":"hao","limit":3}"#;
        let req: Request = serde_json::from_str(json).unwrap();
        let engine = PinyinInputEngine::builtin();
        let resp = handle_request(&engine, &req);
        let out = serde_json::to_string(&resp).unwrap();
        assert!(out.contains("好"));
    }
}
