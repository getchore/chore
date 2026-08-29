//! `chore spec` — the reference has to stay valid JSON, and stay honest
//! about the builtins and variables the crate actually implements.
//!
//! The JSON is checked with the recursive-descent validator below rather than
//! a parser crate: `chorefile` has no serde dependency and this document is
//! the only JSON it ever produces, so a hundred lines of validator is cheaper
//! than a dependency in the tree.

use chorefile::{builtins, spec, vars};

// ---------------------------------------------------------------------------
// a minimal JSON validator
// ---------------------------------------------------------------------------

struct Parser<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Parser<'a> {
    fn new(text: &'a str) -> Self {
        Self {
            bytes: text.as_bytes(),
            at: 0,
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.at).copied()
    }

    fn bump(&mut self) -> Option<u8> {
        let b = self.peek();
        self.at += 1;
        b
    }

    fn space(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.at += 1;
        }
    }

    fn expect(&mut self, want: u8) -> Result<(), String> {
        if self.bump() == Some(want) {
            Ok(())
        } else {
            Err(format!(
                "expected {:?} at byte {}",
                want as char,
                self.at - 1
            ))
        }
    }

    fn literal(&mut self, word: &str) -> Result<(), String> {
        if self.bytes[self.at..].starts_with(word.as_bytes()) {
            self.at += word.len();
            Ok(())
        } else {
            Err(format!("expected {word} at byte {}", self.at))
        }
    }

    fn value(&mut self) -> Result<(), String> {
        self.space();
        match self.peek() {
            Some(b'{') => self.object(),
            Some(b'[') => self.array(),
            Some(b'"') => self.string().map(|_| ()),
            Some(b't') => self.literal("true"),
            Some(b'f') => self.literal("false"),
            Some(b'n') => self.literal("null"),
            Some(b'-' | b'0'..=b'9') => self.number(),
            other => Err(format!("unexpected {other:?} at byte {}", self.at)),
        }
    }

    fn object(&mut self) -> Result<(), String> {
        self.expect(b'{')?;
        self.space();
        if self.peek() == Some(b'}') {
            self.at += 1;
            return Ok(());
        }
        loop {
            self.space();
            self.string()?;
            self.space();
            self.expect(b':')?;
            self.value()?;
            self.space();
            match self.bump() {
                Some(b',') => continue,
                Some(b'}') => return Ok(()),
                _ => return Err(format!("expected , or }} at byte {}", self.at - 1)),
            }
        }
    }

    fn array(&mut self) -> Result<(), String> {
        self.expect(b'[')?;
        self.space();
        if self.peek() == Some(b']') {
            self.at += 1;
            return Ok(());
        }
        loop {
            self.value()?;
            self.space();
            match self.bump() {
                Some(b',') => continue,
                Some(b']') => return Ok(()),
                _ => return Err(format!("expected , or ] at byte {}", self.at - 1)),
            }
        }
    }

    /// Returns the decoded string, so the escaping test can compare against
    /// the original text rather than against another hand-written escape.
    fn string(&mut self) -> Result<String, String> {
        self.expect(b'"')?;
        let mut out = String::new();
        loop {
            match self.bump() {
                None => return Err("unterminated string".into()),
                Some(b'"') => return Ok(out),
                Some(b'\\') => match self.bump() {
                    Some(b'"') => out.push('"'),
                    Some(b'\\') => out.push('\\'),
                    Some(b'/') => out.push('/'),
                    Some(b'b') => out.push('\u{8}'),
                    Some(b'f') => out.push('\u{c}'),
                    Some(b'n') => out.push('\n'),
                    Some(b'r') => out.push('\r'),
                    Some(b't') => out.push('\t'),
                    Some(b'u') => {
                        let hex = std::str::from_utf8(&self.bytes[self.at..self.at + 4])
                            .map_err(|e| e.to_string())?;
                        let code = u32::from_str_radix(hex, 16).map_err(|e| e.to_string())?;
                        out.push(char::from_u32(code).ok_or("bad \\u escape")?);
                        self.at += 4;
                    }
                    other => return Err(format!("bad escape {other:?}")),
                },
                // A raw control character in a string is exactly what the
                // escaping is supposed to prevent, so reject it.
                Some(b) if b < 0x20 => return Err(format!("raw control byte {b:#04x}")),
                Some(b) => {
                    // Multi-byte UTF-8 arrives one byte at a time; collect the
                    // continuation bytes and decode the whole scalar.
                    let extra = match b {
                        0x00..=0x7f => 0,
                        0xc0..=0xdf => 1,
                        0xe0..=0xef => 2,
                        _ => 3,
                    };
                    let start = self.at - 1;
                    self.at += extra;
                    let text = std::str::from_utf8(&self.bytes[start..self.at])
                        .map_err(|e| e.to_string())?;
                    out.push_str(text);
                }
            }
        }
    }

    fn number(&mut self) -> Result<(), String> {
        let start = self.at;
        while matches!(
            self.peek(),
            Some(b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9')
        ) {
            self.at += 1;
        }
        if self.at == start {
            Err(format!("expected a number at byte {start}"))
        } else {
            Ok(())
        }
    }
}

/// Whole-document validation: one value, then nothing but whitespace.
fn validate(text: &str) -> Result<(), String> {
    let mut p = Parser::new(text);
    p.value()?;
    p.space();
    if p.at == p.bytes.len() {
        Ok(())
    } else {
        Err(format!("trailing input at byte {}", p.at))
    }
}

// ---------------------------------------------------------------------------
// the document
// ---------------------------------------------------------------------------

#[test]
fn the_document_is_valid_json() {
    let doc = spec::json();
    validate(&doc).unwrap_or_else(|e| panic!("spec::json() is not valid JSON: {e}"));
}

#[test]
fn json_is_stable_and_pretty() {
    let doc = spec::json();
    // Two calls must agree; a HashMap anywhere in the builder would show up
    // here as a reordered document.
    assert_eq!(doc, spec::json());
    assert!(doc.starts_with("{\n  \"version\": "), "{}", &doc[..40]);
    assert!(doc.ends_with("}\n"));
    // Two-space indent, never a tab.
    assert!(!doc.contains('\t'));
    assert!(doc.contains("\n  \"builtins\": [\n    {\n      \"name\": \"download\","));
}

#[test]
fn every_builtin_appears_exactly_once_and_nothing_else_does() {
    // Forwards: every implemented builtin is documented.
    for name in builtins::NAMES {
        let found = spec::builtins().iter().filter(|b| b.name == *name).count();
        assert_eq!(found, 1, "builtin `{name}` is documented {found} times");
        // ...and reaches the JSON, where a consumer will look for it.
        let key = format!("\"name\": \"{name}\"");
        assert_eq!(
            spec::json().matches(&key).count(),
            1,
            "`{name}` should appear once as a name in the JSON"
        );
    }
    // Backwards: nothing is documented that the crate does not implement.
    for b in spec::builtins() {
        assert!(
            builtins::NAMES.contains(&b.name),
            "`{}` is documented but is not a builtin",
            b.name
        );
    }
    assert_eq!(spec::builtins().len(), builtins::NAMES.len());
}

#[test]
fn every_builtin_variable_appears() {
    for name in vars::BUILTIN_NAMES {
        assert_eq!(
            spec::variables().iter().filter(|v| v.name == *name).count(),
            1,
            "variable `${name}` is not documented exactly once"
        );
    }
    for v in spec::variables() {
        assert!(
            vars::BUILTIN_NAMES.contains(&v.name),
            "`${}` is documented but is not set",
            v.name
        );
        // The distinction a reader needs: fixed for the run, or read off the
        // running task.
        assert!(matches!(v.scope, "run" | "task"), "{} {}", v.name, v.scope);
    }
    // The two that depend on where they are read from.
    for name in ["TASK", "CWD"] {
        let v = spec::variables().iter().find(|v| v.name == name).unwrap();
        assert_eq!(v.scope, "task", "${name} is per-task, not per-run");
    }
}

#[test]
fn extract_reports_its_real_flags() {
    let b = spec::builtin("extract").expect("extract is documented");
    assert_eq!(b.name, "extract");
    assert_eq!(
        b.usage,
        "extract <archive> <dest> [--member name] [--strip n] [--flatten]"
    );
    assert!(b.effects, "extract writes files, so --dry must skip it");

    let flags: Vec<&str> = b.flags.iter().map(|f| f.name).collect();
    assert_eq!(flags, ["--member", "--strip", "--flatten"]);

    let strip = b.flags.iter().find(|f| f.name == "--strip").unwrap();
    assert_eq!(strip.argument, "n");
    // `ExtractArgs::default()` — the implementation's own default.
    assert_eq!(strip.default, "0");

    assert!(spec::builtin("no-such-builtin").is_none());
}

#[test]
fn read_only_builtins_are_marked_effect_free() {
    // These are the ones `--dry` still runs, because a condition or a capture
    // depends on their answer.
    for name in ["which", "find", "read", "sha256", "exists", "echo", "fail"] {
        assert!(
            !spec::builtin(name).unwrap().effects,
            "`{name}` is read-only and must run under --dry"
        );
    }
    for name in ["download", "extract", "archive", "copy", "remove", "write"] {
        assert!(
            spec::builtin(name).unwrap().effects,
            "`{name}` has effects and must be skipped under --dry"
        );
    }
}

#[test]
fn syntax_covers_every_statement_form() {
    let names: Vec<&str> = spec::syntax().iter().map(|f| f.name).collect();
    for want in [
        "assignment",
        "if",
        "for",
        "try",
        "exit",
        "task",
        "include",
        "require",
    ] {
        assert!(names.contains(&want), "no `{want}` form");
    }
    // Every form carries an example, which is the part an agent copies.
    for f in spec::syntax() {
        assert!(!f.example.is_empty(), "`{}` has no example", f.name);
        assert!(!f.meaning.is_empty(), "`{}` has no meaning", f.name);
    }
}

#[test]
fn quotes_and_backslashes_survive_the_round_trip() {
    // The word-splitting rule holds both a quote and a backslash, and the
    // paths rule holds a backslash, so the document escapes itself for us:
    // decoding what the writer produced must give the original text back.
    let doc = spec::json();
    assert!(doc.contains(r#"\""#), "quotes must be escaped");
    assert!(doc.contains(r"\\"), "backslashes must be escaped");

    for name in ["word splitting", "paths"] {
        let rule = spec::rules().iter().find(|r| r.name == name).unwrap();
        assert!(
            rule.rule.contains('"') || rule.rule.contains('\\'),
            "`{name}` is only a fixture if it contains a quote or a backslash"
        );

        // Find this rule's `"rule": "..."` in the document and decode it.
        let key = format!("\"name\": \"{name}\"");
        let at = doc.find(&key).expect("the rule is in the document");
        let value = at + doc[at..].find("\"rule\": ").expect("a rule field") + "\"rule\": ".len();
        let mut p = Parser::new(&doc[value..]);
        assert_eq!(p.string().unwrap(), rule.rule);
    }
}

#[test]
fn the_validator_rejects_broken_json() {
    assert!(validate("{\"a\": }").is_err());
    assert!(validate("[1, 2").is_err());
    assert!(validate("{} {}").is_err());
    assert!(validate("\"unterminated").is_err());
    // A raw control character inside a string is the escaping bug this whole
    // test file exists to catch.
    assert!(validate("\"a\nb\"").is_err());
}

#[test]
fn version_is_the_crate_version() {
    assert_eq!(spec::version(), env!("CARGO_PKG_VERSION"));
    assert!(spec::json().contains(&format!("\"version\": \"{}\"", spec::version())));
}

#[test]
fn reserved_names_and_operators_are_documented() {
    let doc = spec::json();
    for name in chorefile::RESERVED_TASKS {
        assert!(doc.contains(&format!("\"{name}\"")), "`{name}` is missing");
    }
    assert!(doc.contains(chorefile::NAMESPACE_SEP));

    let symbols: Vec<&str> = spec::chaining().iter().map(|o| o.symbol).collect();
    assert_eq!(symbols, ["&&", "||", "|", ">", ">>", "2>", "2>&1"]);
}

/// The one subcommand a chorefile may take back. An agent reading the spec has
/// to be told, or it will keep renaming a perfectly legal `task check`.
#[test]
fn check_is_not_reserved_and_the_rule_says_so() {
    assert!(!chorefile::RESERVED_TASKS.contains(&"check"));
    let rule = spec::rules()
        .iter()
        .find(|r| r.name == "reserved tasks")
        .expect("a rule about reserved tasks");
    assert!(rule.rule.contains("`chore --check`"), "{}", rule.rule);
}

/// An empty unquoted argument collapsing is sh's rule, and the place someone
/// meets it is `$1` — not the paragraph about word splitting.
#[test]
fn interpolation_warns_about_an_empty_unquoted_argument() {
    let form = spec::syntax()
        .iter()
        .find(|f| f.name == "interpolation")
        .expect("the interpolation form");
    assert!(form.meaning.contains("--dry"), "{}", form.meaning);
    assert!(form.meaning.contains("empty"), "{}", form.meaning);
}
