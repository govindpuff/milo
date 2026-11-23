const FIRST_CLAUSE_WORDS: usize = 3;
const FIRST_FORCE_WORDS: usize = 12;
const MIN_SENTENCE_WORDS: usize = 4;
const CLAUSE_SPLIT_WORDS: usize = 30;
const HARD_SPLIT_WORDS: usize = 40;

const ABBREVIATIONS: &[&str] = &[
    "mr", "mrs", "ms", "dr", "prof", "sr", "jr", "st", "vs", "mt", "ft", "approx", "fig", "dept", "est", "gen", "gov",
    "sgt", "capt", "lt", "col", "rev", "hon",
];
const CLOSERS: &[char] = &['"', '\'', '”', '’', ')', ']', '»'];
const OPENERS: &[char] = &['"', '\'', '“', '‘', '(', '[', '«'];
const LEADING_STRIP: &[char] = &['.', ',', ';', ':', '!', '?', '"', '\'', '”', '’', ')', ']', '}', '»'];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Boundary {
    None,
    Clause,
    Sentence,
}

pub struct Chunker {
    buf: String,
    emitted: bool,
    line_start: bool,
    prefix: String,
    close_bracket: bool,
    in_url: bool,
}

impl Default for Chunker {
    fn default() -> Self {
        Self::new()
    }
}

impl Chunker {
    pub fn new() -> Chunker {
        Chunker {
            buf: String::new(),
            emitted: false,
            line_start: true,
            prefix: String::new(),
            close_bracket: false,
            in_url: false,
        }
    }

    pub fn push(&mut self, delta: &str) -> Vec<String> {
        let mut out = Vec::new();
        for c in delta.chars() {
            if self.line_start {
                self.line_char(c, &mut out);
            } else {
                self.inline(c, &mut out);
            }
        }
        out
    }

    pub fn finish(&mut self) -> Option<String> {
        let mut out = Vec::new();
        self.flush_prefix(&mut out);
        if let Some(rest) = clean_chunk(&std::mem::take(&mut self.buf)) {
            out.push(rest);
        }
        *self = Chunker::new();
        if out.is_empty() { None } else { Some(out.join(" ")) }
    }

    fn line_char(&mut self, c: char, out: &mut Vec<String>) {
        if self.prefix.is_empty() {
            match c {
                ' ' | '\t' | '>' => {}
                '#' | '-' | '+' | '*' | '•' => self.prefix.push(c),
                c if c.is_ascii_digit() => self.prefix.push(c),
                '\n' => self.inline(c, out),
                _ => {
                    self.line_start = false;
                    self.inline(c, out);
                }
            }
            return;
        }
        if extends_marker(&self.prefix, c) {
            self.prefix.push(c);
            return;
        }
        if is_marker(&self.prefix) && matches!(c, ' ' | '\t' | '\n') {
            self.prefix.clear();
            if c == '\n' {
                self.inline(c, out);
            }
            return;
        }
        self.flush_prefix(out);
        self.inline(c, out);
    }

    fn flush_prefix(&mut self, out: &mut Vec<String>) {
        self.line_start = false;
        let held = std::mem::take(&mut self.prefix);
        for h in held.chars() {
            self.inline(h, out);
        }
    }

    fn inline(&mut self, c: char, out: &mut Vec<String>) {
        if self.in_url {
            if c == ')' {
                self.in_url = false;
                return;
            }
            if !c.is_whitespace() {
                return;
            }
            self.in_url = false;
        }
        if self.close_bracket {
            self.close_bracket = false;
            if c == '(' {
                self.in_url = true;
                return;
            }
        }
        match c {
            '\n' => self.newline(out),
            '[' | '*' | '`' | '~' => {}
            ']' => self.close_bracket = true,
            '_' | '|' | '•' => self.space(out),
            '—' => {
                self.punct(',', out);
                self.space(out);
            }
            '…' => {
                for _ in 0..3 {
                    self.punct('.', out);
                }
            }
            c if c.is_whitespace() => self.space(out),
            c if is_emoji(c) => {}
            '.' | ',' | ';' | ':' | '!' | '?' | '。' | '！' | '？' | '，' | '；' => self.punct(c, out),
            c => self.buf.push(c),
        }
    }

    fn newline(&mut self, out: &mut Vec<String>) {
        let ends_open =
            self.buf.trim_end().trim_end_matches(CLOSERS).chars().last().is_some_and(|c| c.is_alphanumeric());
        if ends_open {
            while self.buf.ends_with(' ') {
                self.buf.pop();
            }
            self.buf.push('.');
        }
        self.space(out);
        self.line_start = true;
    }

    fn space(&mut self, out: &mut Vec<String>) {
        if self.buf.is_empty() || self.buf.ends_with(' ') {
            return;
        }
        let token = last_token(&self.buf);
        if !token.is_empty() && token.chars().all(|c| matches!(c, '-' | '–' | '=' | '_')) {
            let keep = self.buf.len() - token.len();
            self.buf.truncate(keep);
            while self.buf.ends_with(' ') {
                self.buf.pop();
            }
            if self.buf.chars().last().is_some_and(|c| c.is_alphanumeric()) {
                self.buf.push(',');
            }
            if self.buf.is_empty() {
                return;
            }
        }
        let kind = classify(last_token(&self.buf));
        self.maybe_emit(kind, out);
        if !self.buf.is_empty() {
            self.buf.push(' ');
        }
    }

    fn punct(&mut self, c: char, out: &mut Vec<String>) {
        if self.buf.ends_with(' ') {
            self.buf.pop();
        }
        self.buf.push(c);
        let kind = immediate(&self.buf, c);
        if kind != Boundary::None {
            self.maybe_emit(kind, out);
        }
    }

    fn maybe_emit(&mut self, kind: Boundary, out: &mut Vec<String>) {
        let words = count_words(&self.buf);
        let emit = if self.emitted {
            (kind == Boundary::Sentence && words >= MIN_SENTENCE_WORDS)
                || (kind == Boundary::Clause && words >= CLAUSE_SPLIT_WORDS)
                || words >= HARD_SPLIT_WORDS
        } else {
            kind == Boundary::Sentence
                || (kind == Boundary::Clause && words >= FIRST_CLAUSE_WORDS)
                || words >= FIRST_FORCE_WORDS
        };
        if emit {
            let text = std::mem::take(&mut self.buf);
            if let Some(chunk) = clean_chunk(&text) {
                out.push(chunk);
                self.emitted = true;
            }
        }
    }
}

fn extends_marker(prefix: &str, c: char) -> bool {
    let all_hash = prefix.chars().all(|x| x == '#');
    let all_digit = prefix.chars().all(|x| x.is_ascii_digit());
    (all_hash && c == '#' && prefix.len() < 6)
        || (all_digit && c.is_ascii_digit() && prefix.len() < 3)
        || (all_digit && matches!(c, '.' | ')'))
}

fn is_marker(prefix: &str) -> bool {
    if prefix.chars().all(|x| x == '#') || matches!(prefix, "-" | "+" | "*" | "•") {
        return true;
    }
    match prefix.strip_suffix(['.', ')']) {
        Some(num) => !num.is_empty() && num.chars().all(|x| x.is_ascii_digit()),
        None => false,
    }
}

fn last_token(buf: &str) -> &str {
    buf.rsplit(' ').next().unwrap_or("")
}

fn count_words(buf: &str) -> usize {
    buf.split(' ').filter(|w| w.chars().any(char::is_alphanumeric)).count()
}

fn immediate(buf: &str, c: char) -> Boundary {
    match c {
        '!' | '?' | '。' | '！' | '？' => Boundary::Sentence,
        ',' | ';' | ':' | '，' | '；' => {
            let mut rev = buf.chars().rev();
            rev.next();
            if rev.next().is_some_and(char::is_alphabetic) { Boundary::Clause } else { Boundary::None }
        }
        '.' => {
            let token = last_token(buf);
            let core = token[..token.len() - 1].trim_start_matches(OPENERS);
            let plain_word = core.chars().count() >= 3 && core.chars().all(char::is_alphabetic);
            if plain_word && !ABBREVIATIONS.contains(&core.to_lowercase().as_str()) {
                Boundary::Sentence
            } else {
                Boundary::None
            }
        }
        _ => Boundary::None,
    }
}

fn classify(token: &str) -> Boundary {
    let t = token.trim_end_matches(CLOSERS);
    if t.ends_with("...") {
        return Boundary::Clause;
    }
    match t.chars().last() {
        Some('!' | '?' | '。' | '！' | '？') => Boundary::Sentence,
        Some(',' | ';' | ':' | '，' | '；') => Boundary::Clause,
        Some('.') if is_abbreviation(&t[..t.len() - 1]) => Boundary::None,
        Some('.') => Boundary::Sentence,
        _ => Boundary::None,
    }
}

fn is_abbreviation(core: &str) -> bool {
    let core = core.trim_start_matches(OPENERS);
    if core.is_empty() {
        return false;
    }
    if ABBREVIATIONS.contains(&core.to_lowercase().as_str()) {
        return true;
    }
    let mut chars = core.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        return c.is_alphabetic() && c != 'I';
    }
    core.contains('.')
        && core.split('.').all(|s| !s.is_empty() && s.chars().count() <= 2 && s.chars().all(char::is_alphabetic))
}

fn clean_chunk(text: &str) -> Option<String> {
    let t = text.trim_start_matches(|c: char| c.is_whitespace() || LEADING_STRIP.contains(&c)).trim_end();
    t.chars().any(char::is_alphanumeric).then(|| t.to_string())
}

fn is_emoji(c: char) -> bool {
    matches!(
        c as u32,
        0x1F000..=0x1FAFF
            | 0x2600..=0x27BF
            | 0x2300..=0x23FF
            | 0x2B00..=0x2BFF
            | 0x2190..=0x21FF
            | 0xFE00..=0xFE0F
            | 0x200D
            | 0x20E3
            | 0xE0020..=0xE007F
            | 0x3030
            | 0x303D
            | 0x3297
            | 0x3299
    )
}
