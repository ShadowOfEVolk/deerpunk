use std::fs::File;
use std::io::{self, BufRead, BufReader};
use std::path::{Path, PathBuf};

pub struct FileSystem {
    initialized: bool,
    gamedir: PathBuf,
}

impl FileSystem {
    pub fn new() -> Self {
        Self {
            initialized: false,
            gamedir: PathBuf::new(),
        }
    }

    pub fn initialize(&mut self) {
        self.initialized = true;
    }

    pub fn set_gamedir<P: AsRef<Path>>(&mut self, gamedir: P) {
        self.gamedir = gamedir.as_ref().to_path_buf();
    }

    pub fn gamedir(&self) -> &Path {
        &self.gamedir
    }

    pub fn resolve_path<P: AsRef<Path>>(&self, path: P) -> PathBuf {
        if path.as_ref().is_absolute() {
            path.as_ref().to_path_buf()
        } else {
            self.gamedir.join(path)
        }
    }

    pub fn shutdown(&mut self) {
        self.initialized = false;
    }

    pub fn load_file<P: AsRef<Path>>(&self, path: P) -> Result<String, io::Error> {
        std::fs::read_to_string(self.resolve_path(path))
    }

    pub fn read_file<P: AsRef<Path>>(&self, path: P) -> Result<Vec<u8>, io::Error> {
        std::fs::read(self.resolve_path(path))
    }

    pub fn stream_file<P: AsRef<Path>>(&self, path: P) -> Result<FileStream, io::Error> {
        let file = File::open(self.resolve_path(path))?;
        let reader = BufReader::new(file);
        Ok(FileStream::new(reader))
    }

    pub fn file_exists<P: AsRef<Path>>(&self, path: P) -> bool {
        self.resolve_path(path).exists()
    }

    /// UI asset root: `{gamedir}/ui` or `examples/ui_hello/ui` (absolute).
    pub fn resolve_ui_dir(&self) -> PathBuf {
        let candidates = [
            self.gamedir.join("ui"),
            PathBuf::from("examples/ui_hello/ui"),
            // when cwd is target/debug
            PathBuf::from("../../examples/ui_hello/ui"),
            PathBuf::from("../../examples/deathmatch/gamedir/ui"),
        ];
        for c in candidates {
            if c.join("widgets").exists() || c.join("fonts").exists() {
                return c.canonicalize().unwrap_or(c);
            }
        }
        let fallback = self.gamedir.join("ui");
        log::error!(
            "[fs] UI dir not found (tried gamedir/ui and examples/ui_hello/ui). cwd={:?} fallback={}",
            std::env::current_dir().ok(),
            fallback.display()
        );
        fallback
    }
}

impl Default for FileSystem {
    fn default() -> Self {
        Self::new()
    }
}

pub struct FileStream {
    reader: BufReader<File>,
}

impl FileStream {
    fn new(reader: BufReader<File>) -> Self {
        Self { reader }
    }

    pub fn lines(&mut self) -> LinesIterator {
        LinesIterator {
            reader: &mut self.reader,
        }
    }

    pub fn tokens(&mut self) -> TokensIterator {
        TokensIterator {
            reader: &mut self.reader,
            current_line: None,
            line_pos: 0,
        }
    }
}

pub struct LinesIterator<'a> {
    reader: &'a mut BufReader<File>,
}

impl<'a> Iterator for LinesIterator<'a> {
    type Item = Result<String, io::Error>;

    fn next(&mut self) -> Option<Self::Item> {
        let mut line = String::new();
        loop {
            line.clear();
            match self.reader.read_line(&mut line) {
                Ok(0) => return None,
                Ok(_) => {
                    if let Some(comment_pos) = line.find("//") {
                        line.truncate(comment_pos);
                    }
                    let trimmed = line.trim().to_string();
                    if trimmed.is_empty() {
                        continue;
                    }
                    return Some(Ok(trimmed));
                }
                Err(e) => return Some(Err(e)),
            }
        }
    }
}

pub struct TokensIterator<'a> {
    reader: &'a mut BufReader<File>,
    current_line: Option<Vec<String>>,
    line_pos: usize,
}

impl<'a> TokensIterator<'a> {
    fn load_next_line(&mut self) -> Option<Result<(), io::Error>> {
        let mut line = String::new();
        loop {
            line.clear();
            match self.reader.read_line(&mut line) {
                Ok(0) => return None,
                Ok(_) => {
                    if let Some(comment_pos) = line.find("//") {
                        line.truncate(comment_pos);
                    }
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        continue;
                    }
                    self.current_line = Some(tokenize_line(trimmed));
                    self.line_pos = 0;
                    return Some(Ok(()));
                }
                Err(e) => return Some(Err(e)),
            }
        }
    }
}

impl<'a> Iterator for TokensIterator<'a> {
    type Item = Result<String, io::Error>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(ref tokens) = self.current_line {
                if self.line_pos < tokens.len() {
                    let token = tokens[self.line_pos].clone();
                    self.line_pos += 1;
                    return Some(Ok(token));
                }
            }
            match self.load_next_line() {
                Some(Ok(_)) => continue,
                Some(Err(e)) => return Some(Err(e)),
                None => return None,
            }
        }
    }
}

fn tokenize_line(line: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut chars = line.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            '"' => {
                if in_quotes {
                    if !current.is_empty() {
                        tokens.push(current.clone());
                        current.clear();
                    }
                    in_quotes = false;
                } else {
                    in_quotes = true;
                }
            }
            ' ' | '\t' if !in_quotes => {
                if !current.is_empty() {
                    tokens.push(current.clone());
                    current.clear();
                }
            }
            _ => {
                current.push(ch);
            }
        }
    }

    if !current.is_empty() || in_quotes {
        tokens.push(current);
    }

    tokens
}
