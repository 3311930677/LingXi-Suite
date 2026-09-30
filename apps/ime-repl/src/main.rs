//! `ime-repl` — an interactive demo of the pure-Rust pinyin engine.
//!
//! It reads pinyin key sequences and prints ranked Chinese candidates, so the
//! `assistant-ime` engine can be exercised end-to-end from a terminal without a
//! GUI or a real desktop. Two modes:
//!
//! - **One-shot**: pass pinyin as arguments — `ime-repl nihao` — prints the
//!   candidates and exits. Handy for scripts and quick checks.
//! - **Interactive REPL**: run with no pinyin argument and type sequences at the
//!   prompt; blank line or Ctrl-D/Ctrl-Z exits.
//!
//! Options (before any pinyin argument):
//! - `--dict <FILE>` load a rime-ice-style dictionary file (repeatable); without
//!   any, the built-in dictionary is used.
//! - `--limit <N>` cap the number of candidates shown (default 9).
//! - `--context <TEXT>` preceding committed text, used by the reranker.
//! - `--no-fuzzy` disable fuzzy-pinyin matching.
//! - `-h`, `--help` show usage.

use std::io::{self, Write};
use std::process::ExitCode;

use assistant_ime::{
    Dictionary, InputContext, InputEngine, PinyinInputEngine, PrefixContextReranker,
};

struct Options {
    dict_files: Vec<String>,
    limit: usize,
    context: String,
    fuzzy: bool,
    /// Pinyin given directly on the command line (one-shot mode) if any.
    inline_pinyin: Vec<String>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            dict_files: Vec::new(),
            limit: 9,
            context: String::new(),
            fuzzy: true,
            inline_pinyin: Vec::new(),
        }
    }
}

const USAGE: &str = "\
ime-repl — pinyin candidate demo for assistant-ime

USAGE:
    ime-repl [OPTIONS] [PINYIN]...

OPTIONS:
    --dict <FILE>      Load a rime-ice-style dictionary file (repeatable).
    --limit <N>        Max candidates to show (default 9).
    --context <TEXT>   Preceding committed text (used by the reranker).
    --no-fuzzy         Disable fuzzy-pinyin matching.
    -h, --help         Show this help.

EXAMPLES:
    ime-repl nihao
    ime-repl --limit 5 --context 你好 shijie
    ime-repl --dict rime-ice.dict.yaml        # then type at the prompt
";

fn parse_args(args: impl Iterator<Item = String>) -> Result<Options, String> {
    let mut opts = Options::default();
    let mut args = args.peekable();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => return Err(String::new()), // signals "print usage, exit 0"
            "--no-fuzzy" => opts.fuzzy = false,
            "--dict" => {
                let value = args
                    .next()
                    .ok_or_else(|| "--dict requires a file path".to_string())?;
                opts.dict_files.push(value);
            }
            "--limit" => {
                let value = args
                    .next()
                    .ok_or_else(|| "--limit requires a number".to_string())?;
                opts.limit = value
                    .parse()
                    .map_err(|_| format!("--limit: not a number: {value}"))?;
            }
            "--context" => {
                opts.context = args
                    .next()
                    .ok_or_else(|| "--context requires text".to_string())?;
            }
            other if other.starts_with('-') && other.len() > 1 => {
                return Err(format!("unknown option: {other}"));
            }
            // First non-option token and everything after it is inline pinyin.
            other => {
                opts.inline_pinyin.push(other.to_string());
                opts.inline_pinyin.extend(args.by_ref());
            }
        }
    }
    Ok(opts)
}

fn build_engine(opts: &Options) -> io::Result<PinyinInputEngine> {
    let dictionary = if opts.dict_files.is_empty() {
        Dictionary::builtin().with_fuzzy(opts.fuzzy)
    } else {
        Dictionary::from_files(&opts.dict_files)?.with_fuzzy(opts.fuzzy)
    };
    Ok(PinyinInputEngine::new(
        dictionary,
        Box::new(PrefixContextReranker::default()),
    ))
}

fn print_candidates(engine: &PinyinInputEngine, pinyin: &str, opts: &Options) {
    let context = InputContext {
        preceding_text: opts.context.clone(),
        max_candidates: opts.limit,
    };
    let candidates = engine.candidates(pinyin, &context);
    if candidates.is_empty() {
        println!("  (no candidates for \"{pinyin}\")");
        return;
    }
    for (index, candidate) in candidates.iter().enumerate() {
        println!(
            "  {}. {}  [{}]  score={:.3}",
            index + 1,
            candidate.text,
            candidate.syllables.join(" "),
            candidate.score
        );
    }
}

fn run_repl(engine: &PinyinInputEngine, opts: &Options) -> io::Result<()> {
    println!("assistant-ime REPL — type pinyin, blank line or Ctrl-D/Ctrl-Z to quit.");
    let stdin = io::stdin();
    let mut line = String::new();
    loop {
        print!("pinyin> ");
        io::stdout().flush()?;
        line.clear();
        let read = stdin.read_line(&mut line)?;
        if read == 0 {
            println!();
            break; // EOF
        }
        let pinyin = line.trim();
        if pinyin.is_empty() {
            break;
        }
        print_candidates(engine, pinyin, opts);
    }
    Ok(())
}

fn main() -> ExitCode {
    let opts = match parse_args(std::env::args().skip(1)) {
        Ok(opts) => opts,
        Err(message) => {
            if message.is_empty() {
                // --help
                print!("{USAGE}");
                return ExitCode::SUCCESS;
            }
            eprintln!("error: {message}\n\n{USAGE}");
            return ExitCode::FAILURE;
        }
    };

    let engine = match build_engine(&opts) {
        Ok(engine) => engine,
        Err(error) => {
            eprintln!("error: failed to load dictionary: {error}");
            return ExitCode::FAILURE;
        }
    };

    if opts.inline_pinyin.is_empty() {
        if let Err(error) = run_repl(&engine, &opts) {
            eprintln!("error: {error}");
            return ExitCode::FAILURE;
        }
    } else {
        for pinyin in &opts.inline_pinyin {
            println!("{pinyin}:");
            print_candidates(&engine, pinyin, &opts);
        }
    }
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Options {
        parse_args(list.iter().map(|s| s.to_string())).expect("parse")
    }

    #[test]
    fn parses_options_and_inline_pinyin() {
        let opts = args(&["--limit", "5", "--context", "你好", "nihao", "shijie"]);
        assert_eq!(opts.limit, 5);
        assert_eq!(opts.context, "你好");
        assert_eq!(opts.inline_pinyin, vec!["nihao", "shijie"]);
        assert!(opts.fuzzy);
    }

    #[test]
    fn no_fuzzy_flag_and_repeated_dict() {
        let opts = args(&["--no-fuzzy", "--dict", "a.txt", "--dict", "b.txt"]);
        assert!(!opts.fuzzy);
        assert_eq!(opts.dict_files, vec!["a.txt", "b.txt"]);
        assert!(opts.inline_pinyin.is_empty());
    }

    #[test]
    fn unknown_option_is_an_error() {
        let result = parse_args(["--bogus".to_string()].into_iter());
        assert!(result.is_err());
    }

    #[test]
    fn missing_option_value_is_an_error() {
        assert!(parse_args(["--limit".to_string()].into_iter()).is_err());
        assert!(parse_args(["--dict".to_string()].into_iter()).is_err());
    }

    #[test]
    fn help_signals_empty_error() {
        // --help is modeled as an empty-string Err so main prints usage/exit 0.
        let result = parse_args(["--help".to_string()].into_iter());
        assert_eq!(result.err(), Some(String::new()));
    }

    #[test]
    fn everything_after_first_pinyin_is_treated_as_pinyin() {
        // A dash-looking token after pinyin starts is still captured, not parsed.
        let opts = args(&["nihao", "--limit"]);
        assert_eq!(opts.inline_pinyin, vec!["nihao", "--limit"]);
    }

    #[test]
    fn builtin_engine_produces_expected_top_candidate() {
        let opts = Options::default();
        let engine = build_engine(&opts).expect("engine");
        let cs = engine.candidates("nihao", &InputContext::with_limit(3));
        assert_eq!(cs[0].text, "你好");
    }
}
