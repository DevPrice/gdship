//! A reader for Godot's ConfigFile syntax, as used by `project.godot` and
//! `export_presets.cfg`. It keeps each value's source text and decodes only the kinds
//! gdship reads: strings and `PackedStringArray`s.

use anyhow::{Result, bail};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Section {
    pub(crate) name: String,
    /// Keys and raw value text, in file order.
    pub(crate) entries: Vec<(String, String)>,
}

impl Section {
    pub(crate) fn get(&self, key: &str) -> Option<&str> {
        self.entries
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    /// The value of `key` decoded as a string, or `None` if the key is missing.
    pub(crate) fn string(&self, key: &str) -> Result<Option<String>> {
        match self.get(key) {
            Some(raw) => match parse_string(raw) {
                Some(value) => Ok(Some(value)),
                None => bail!("`{key}` in [{}] is not a string: {raw}", self.name),
            },
            None => Ok(None),
        }
    }
}

/// Splits `text` into sections. Entries before the first header go in a section named "".
pub(crate) fn parse(text: &str) -> Result<Vec<Section>> {
    let mut sections = vec![Section {
        name: String::new(),
        entries: Vec::new(),
    }];
    let mut scanner = Scanner {
        text,
        pos: 0,
        line: 1,
    };
    loop {
        scanner.skip_blank_and_comments();
        let Some(c) = scanner.peek() else { break };
        if c == '[' {
            let line = scanner.line;
            let header = scanner.take_line();
            let Some(name) = header
                .trim_end()
                .strip_prefix('[')
                .and_then(|h| h.strip_suffix(']'))
            else {
                bail!(
                    "line {line}: malformed section header `{}`",
                    header.trim_end()
                );
            };
            sections.push(Section {
                name: name.to_owned(),
                entries: Vec::new(),
            });
            continue;
        }
        let line = scanner.line;
        let key = scanner.key()?;
        if scanner.peek() != Some('=') {
            bail!("line {line}: expected `=` after `{key}`");
        }
        scanner.bump();
        let value = scanner.value(line)?;
        sections
            .last_mut()
            .expect("there is always a section")
            .entries
            .push((key, value.trim().to_owned()));
    }
    Ok(sections)
}

struct Scanner<'a> {
    text: &'a str,
    pos: usize,
    line: usize,
}

impl Scanner<'_> {
    fn peek(&self) -> Option<char> {
        self.text[self.pos..].chars().next()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.pos += c.len_utf8();
        if c == '\n' {
            self.line += 1;
        }
        Some(c)
    }

    fn take_line(&mut self) -> &str {
        let start = self.pos;
        while let Some(c) = self.bump() {
            if c == '\n' {
                return &self.text[start..self.pos - 1];
            }
        }
        &self.text[start..]
    }

    fn skip_blank_and_comments(&mut self) {
        while let Some(c) = self.peek() {
            if c.is_whitespace() {
                self.bump();
            } else if c == ';' || c == '#' {
                self.take_line();
            } else {
                break;
            }
        }
    }

    /// A bare key runs to `=`; a quoted one, like `"Windows Desktop"` in
    /// `[runnable_presets]`, is decoded like a string value.
    fn key(&mut self) -> Result<String> {
        let line = self.line;
        if self.peek() == Some('"') {
            let start = self.pos;
            self.skip_string(line)?;
            let key = parse_string(&self.text[start..self.pos]).expect("scanned a string");
            self.skip_inline_space();
            return Ok(key);
        }
        let start = self.pos;
        while let Some(c) = self.peek() {
            if c == '=' || c == '\n' {
                break;
            }
            self.bump();
        }
        let key = self.text[start..self.pos].trim();
        if key.is_empty() {
            bail!("line {line}: expected a key");
        }
        Ok(key.to_owned())
    }

    fn skip_inline_space(&mut self) {
        while self.peek().is_some_and(|c| c == ' ' || c == '\t') {
            self.bump();
        }
    }

    /// Consumes one value: up to the end of the line, except that strings, and brackets
    /// of any kind, continue across lines.
    fn value(&mut self, line: usize) -> Result<&str> {
        let start = self.pos;
        let mut closers = Vec::new();
        while let Some(c) = self.peek() {
            match c {
                '"' => {
                    self.skip_string(line)?;
                    continue;
                }
                '(' => closers.push(')'),
                '[' => closers.push(']'),
                '{' => closers.push('}'),
                ')' | ']' | '}' => {
                    if closers.pop() != Some(c) {
                        bail!(
                            "line {}: unbalanced `{c}` in the value on line {line}",
                            self.line
                        );
                    }
                }
                '\n' if closers.is_empty() => break,
                _ => {}
            }
            self.bump();
        }
        if !closers.is_empty() {
            bail!("line {line}: the value never closes its brackets");
        }
        Ok(&self.text[start..self.pos])
    }

    fn skip_string(&mut self, line: usize) -> Result<()> {
        self.bump();
        while let Some(c) = self.bump() {
            match c {
                '\\' => {
                    self.bump();
                }
                '"' => return Ok(()),
                _ => {}
            }
        }
        bail!("line {line}: unterminated string")
    }
}

/// Decodes a string literal, including its escapes. `None` if `raw` is not exactly one
/// string.
pub(crate) fn parse_string(raw: &str) -> Option<String> {
    let (value, rest) = take_string(raw.trim())?;
    rest.trim().is_empty().then_some(value)
}

/// Decodes the elements of `PackedStringArray(...)`, or `None` if `raw` is not one.
pub(crate) fn parse_packed_strings(raw: &str) -> Option<Vec<String>> {
    let mut rest = raw.trim().strip_prefix("PackedStringArray(")?.trim_start();
    let mut items = Vec::new();
    loop {
        if let Some(after) = rest.strip_prefix(')') {
            return after.trim().is_empty().then_some(items);
        }
        let (item, after) = take_string(rest)?;
        items.push(item);
        rest = after.trim_start();
        if let Some(after) = rest.strip_prefix(',') {
            rest = after.trim_start();
        } else if !rest.starts_with(')') {
            return None;
        }
    }
}

/// Decodes the string literal at the start of `text` and returns what follows it.
fn take_string(text: &str) -> Option<(String, &str)> {
    let mut chars = text.strip_prefix('"')?.char_indices();
    let mut value = String::new();
    while let Some((i, c)) = chars.next() {
        match c {
            '"' => return Some((value, &text[i + 2..])),
            '\\' => {
                let (_, escaped) = chars.next()?;
                match escaped {
                    'n' => value.push('\n'),
                    't' => value.push('\t'),
                    'r' => value.push('\r'),
                    'b' => value.push('\u{8}'),
                    'f' => value.push('\u{c}'),
                    'u' | 'U' => {
                        let digits = if escaped == 'u' { 4 } else { 6 };
                        let hex: String = (0..digits)
                            .map_while(|_| chars.next())
                            .map(|(_, c)| c)
                            .collect();
                        value.push(char::from_u32(u32::from_str_radix(&hex, 16).ok()?)?);
                    }
                    other => value.push(other),
                }
            }
            _ => value.push(c),
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entries(text: &str) -> Vec<(String, Vec<(String, String)>)> {
        parse(text)
            .unwrap()
            .into_iter()
            .map(|s| (s.name, s.entries))
            .collect()
    }

    fn pair(key: &str, value: &str) -> (String, String) {
        (key.to_owned(), value.to_owned())
    }

    #[test]
    fn reads_sections_keys_and_comments() {
        let text = "; Engine configuration file.\n\nconfig_version=5\n\n[application]\n\nconfig/name=\"Idle Factory\"\n";
        assert_eq!(
            entries(text),
            [
                (String::new(), vec![pair("config_version", "5")]),
                (
                    "application".to_owned(),
                    vec![pair("config/name", "\"Idle Factory\"")]
                ),
            ]
        );
    }

    #[test]
    fn multi_line_strings_hide_headers_and_keys() {
        let text = "[a]\nhead=\"<script>\n[preset.9]\nname=\\\"Fake\\\"\n</script>\"\nafter=1\n";
        let sections = parse(text).unwrap();
        assert_eq!(sections.len(), 2);
        let a = &sections[1];
        assert_eq!(
            a.string("head").unwrap().unwrap(),
            "<script>\n[preset.9]\nname=\"Fake\"\n</script>"
        );
        assert_eq!(a.get("after"), Some("1"));
        assert_eq!(a.get("name"), None);
    }

    #[test]
    fn escaped_quotes_and_backslashes() {
        let text = "[a]\nq=\"say \\\"hi\\\" \\\\\"\nnext=\"\\u00e9\\n\"\n";
        let a = &parse(text).unwrap()[1];
        assert_eq!(a.string("q").unwrap().unwrap(), "say \"hi\" \\");
        assert_eq!(a.string("next").unwrap().unwrap(), "é\n");
    }

    #[test]
    fn multi_line_arrays_and_dictionaries() {
        let text = "[a]\narr=[1,\n\"],\",\n[2]\n]\ndict={\n\"k\": Vector2(1, 2),\n\"}\": {}\n}\nlast=PackedStringArray(\"4.7\",\n \"Forward Plus\")\n";
        let a = &parse(text).unwrap()[1];
        assert_eq!(a.get("arr"), Some("[1,\n\"],\",\n[2]\n]"));
        assert!(a.get("dict").unwrap().ends_with("\"}\": {}\n}"));
        assert_eq!(
            parse_packed_strings(a.get("last").unwrap()).unwrap(),
            ["4.7", "Forward Plus"]
        );
    }

    #[test]
    fn quoted_keys() {
        let text = "[runnable_presets]\n\nWeb=\"Web\"\n\"Windows Desktop\"=\"Windows Desktop\"\n";
        let section = &parse(text).unwrap()[1];
        assert_eq!(
            section.string("Windows Desktop").unwrap().as_deref(),
            Some("Windows Desktop")
        );
    }

    #[test]
    fn malformed_input_is_an_error() {
        for text in [
            "[a]\nx=\"open",
            "[a]\nx=[1, 2",
            "[a]\nx=(1]",
            "[a\n",
            "[a]\njunk\n",
        ] {
            assert!(parse(text).is_err(), "{text:?}");
        }
    }

    #[test]
    fn decoding_rejects_non_strings() {
        assert_eq!(parse_string("\"a\" \"b\""), None);
        assert_eq!(parse_string("12"), None);
        assert_eq!(parse_packed_strings("PackedStringArray()"), Some(vec![]));
        assert_eq!(parse_packed_strings("PackedStringArray(\"a\" \"b\")"), None);
        assert_eq!(parse_packed_strings("[\"a\"]"), None);
    }
}
