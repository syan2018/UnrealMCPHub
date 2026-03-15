use anyhow::{Result, anyhow, bail};
use serde_json::{Map, Number, Value};

pub fn parse_object_argument(raw: &str) -> Result<Map<String, Value>> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(Map::new());
    }

    match serde_json::from_str::<Value>(trimmed) {
        Ok(Value::Object(map)) => return Ok(map),
        Ok(Value::Null) => return Ok(Map::new()),
        Ok(other) => bail!("expected JSON object for --arguments-json, got {other}"),
        Err(_) => {}
    }

    let mut parser = RelaxedValueParser::new(trimmed);
    match parser.parse_root_object() {
        Ok(map) => Ok(map),
        Err(relaxed_error) => {
            if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
                return match value {
                    Value::Object(map) => Ok(map),
                    Value::Null => Ok(Map::new()),
                    other => bail!("expected JSON object for --arguments-json, got {other}"),
                };
            }

            Err(anyhow!(
                "failed to parse --arguments-json as strict JSON or relaxed PowerShell object syntax: {relaxed_error}"
            ))
        }
    }
}

struct RelaxedValueParser<'a> {
    source: &'a str,
    bytes: &'a [u8],
    cursor: usize,
}

impl<'a> RelaxedValueParser<'a> {
    fn new(source: &'a str) -> Self {
        Self {
            source,
            bytes: source.as_bytes(),
            cursor: 0,
        }
    }

    fn parse_root_object(&mut self) -> Result<Map<String, Value>> {
        self.skip_whitespace();
        self.expect_byte(b'{')?;
        let object = self.parse_object_body()?;
        self.skip_whitespace();
        if self.cursor != self.bytes.len() {
            bail!(
                "unexpected trailing content at byte {}: {}",
                self.cursor,
                self.remaining().trim()
            );
        }
        Ok(object)
    }

    fn parse_object_body(&mut self) -> Result<Map<String, Value>> {
        let mut object = Map::new();
        loop {
            self.skip_whitespace();
            if self.peek() == Some(b'}') {
                self.cursor += 1;
                break;
            }

            let key = self.parse_key()?;
            self.skip_whitespace();
            self.expect_byte(b':')?;
            self.skip_whitespace();
            let value = self.parse_value()?;
            object.insert(key, value);
            self.skip_whitespace();

            match self.peek() {
                Some(b',') => {
                    self.cursor += 1;
                }
                Some(b'}') => {
                    self.cursor += 1;
                    break;
                }
                Some(other) => bail!(
                    "expected ',' or '}}' at byte {}, found '{}'",
                    self.cursor,
                    other as char
                ),
                None => bail!("unterminated object literal"),
            }
        }
        Ok(object)
    }

    fn parse_array(&mut self) -> Result<Vec<Value>> {
        self.expect_byte(b'[')?;
        let mut array = Vec::new();
        loop {
            self.skip_whitespace();
            if self.peek() == Some(b']') {
                self.cursor += 1;
                break;
            }

            array.push(self.parse_value()?);
            self.skip_whitespace();
            match self.peek() {
                Some(b',') => {
                    self.cursor += 1;
                }
                Some(b']') => {
                    self.cursor += 1;
                    break;
                }
                Some(other) => bail!(
                    "expected ',' or ']' at byte {}, found '{}'",
                    self.cursor,
                    other as char
                ),
                None => bail!("unterminated array literal"),
            }
        }
        Ok(array)
    }

    fn parse_key(&mut self) -> Result<String> {
        self.skip_whitespace();
        match self.peek() {
            Some(b'"') | Some(b'\'') => self.parse_quoted_string(),
            Some(_) => {
                let start = self.cursor;
                while let Some(byte) = self.peek() {
                    if byte == b':' || byte.is_ascii_whitespace() {
                        break;
                    }
                    if matches!(byte, b',' | b'{' | b'}' | b'[' | b']') {
                        break;
                    }
                    self.cursor += 1;
                }
                let key = self.source[start..self.cursor].trim();
                if key.is_empty() {
                    bail!("expected object key at byte {}", start);
                }
                Ok(key.to_string())
            }
            None => bail!("expected object key at end of input"),
        }
    }

    fn parse_value(&mut self) -> Result<Value> {
        self.skip_whitespace();
        match self.peek() {
            Some(b'{') => Ok(Value::Object(self.parse_object_body_after_open()?)),
            Some(b'[') => Ok(Value::Array(self.parse_array()?)),
            Some(b'"') | Some(b'\'') => Ok(Value::String(self.parse_quoted_string()?)),
            Some(_) => self.parse_bare_value(),
            None => bail!("expected value at end of input"),
        }
    }

    fn parse_object_body_after_open(&mut self) -> Result<Map<String, Value>> {
        self.expect_byte(b'{')?;
        self.parse_object_body()
    }

    fn parse_quoted_string(&mut self) -> Result<String> {
        let quote = self
            .peek()
            .ok_or_else(|| anyhow!("expected quoted string at end of input"))?;
        self.cursor += 1;
        let mut segment_start = self.cursor;
        let mut value = String::new();

        while let Some(byte) = self.peek() {
            if byte == quote {
                value.push_str(&self.source[segment_start..self.cursor]);
                self.cursor += 1;
                return Ok(value);
            }
            if byte == b'\\' {
                value.push_str(&self.source[segment_start..self.cursor]);
                self.cursor += 1;
                let Some(escaped) = self.peek() else {
                    value.push('\\');
                    return Ok(value);
                };
                self.cursor += 1;
                match escaped {
                    b'"' => value.push('"'),
                    b'\'' => value.push('\''),
                    b'\\' => value.push('\\'),
                    b'/' => value.push('/'),
                    b'b' => value.push('\u{0008}'),
                    b'f' => value.push('\u{000c}'),
                    b'n' => value.push('\n'),
                    b'r' => value.push('\r'),
                    b't' => value.push('\t'),
                    b'u' => value.push(self.parse_unicode_escape()?),
                    other => {
                        value.push('\\');
                        value.push(other as char);
                    }
                }
                segment_start = self.cursor;
                continue;
            }

            self.cursor += 1;
        }

        bail!("unterminated quoted string")
    }

    fn parse_unicode_escape(&mut self) -> Result<char> {
        let end = self.cursor.saturating_add(4);
        if end > self.bytes.len() {
            bail!("incomplete unicode escape at byte {}", self.cursor);
        }
        let digits = &self.source[self.cursor..end];
        self.cursor = end;
        let code = u32::from_str_radix(digits, 16)
            .map_err(|error| anyhow!("invalid unicode escape '\\u{digits}': {error}"))?;
        char::from_u32(code).ok_or_else(|| anyhow!("invalid unicode scalar '\\u{digits}'"))
    }

    fn parse_bare_value(&mut self) -> Result<Value> {
        let start = self.cursor;
        let mut brace_depth = 0usize;
        let mut bracket_depth = 0usize;
        let mut active_quote = None::<u8>;

        while let Some(byte) = self.peek() {
            if let Some(quote) = active_quote {
                self.cursor += 1;
                if byte == b'\\' {
                    if self.peek().is_some() {
                        self.cursor += 1;
                    }
                    continue;
                }
                if byte == quote {
                    active_quote = None;
                }
                continue;
            }

            match byte {
                b'"' | b'\'' => {
                    active_quote = Some(byte);
                    self.cursor += 1;
                }
                b'{' => {
                    brace_depth += 1;
                    self.cursor += 1;
                }
                b'}' => {
                    if brace_depth == 0 && bracket_depth == 0 {
                        break;
                    }
                    brace_depth = brace_depth.saturating_sub(1);
                    self.cursor += 1;
                }
                b'[' => {
                    bracket_depth += 1;
                    self.cursor += 1;
                }
                b']' => {
                    bracket_depth = bracket_depth.saturating_sub(1);
                    self.cursor += 1;
                }
                b',' if brace_depth == 0 && bracket_depth == 0 => break,
                _ => {
                    self.cursor += 1;
                }
            }
        }

        let token = self.source[start..self.cursor].trim();
        if token.is_empty() {
            bail!("expected value at byte {}", start);
        }

        if let Some(boolean) = parse_bool_literal(token) {
            return Ok(Value::Bool(boolean));
        }
        if token.eq_ignore_ascii_case("null") || token.eq_ignore_ascii_case("none") {
            return Ok(Value::Null);
        }
        if let Some(number) = parse_number_literal(token) {
            return Ok(Value::Number(number));
        }

        Ok(Value::String(decode_relaxed_string(token)))
    }

    fn skip_whitespace(&mut self) {
        while let Some(byte) = self.peek() {
            if byte.is_ascii_whitespace() {
                self.cursor += 1;
            } else {
                break;
            }
        }
    }

    fn expect_byte(&mut self, expected: u8) -> Result<()> {
        match self.peek() {
            Some(byte) if byte == expected => {
                self.cursor += 1;
                Ok(())
            }
            Some(found) => bail!(
                "expected '{}' at byte {}, found '{}'",
                expected as char,
                self.cursor,
                found as char
            ),
            None => bail!(
                "expected '{}' at end of input",
                expected as char
            ),
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.cursor).copied()
    }

    fn remaining(&self) -> &str {
        &self.source[self.cursor..]
    }
}

fn parse_bool_literal(token: &str) -> Option<bool> {
    if token.eq_ignore_ascii_case("true") {
        Some(true)
    } else if token.eq_ignore_ascii_case("false") {
        Some(false)
    } else {
        None
    }
}

fn parse_number_literal(token: &str) -> Option<Number> {
    if let Ok(value) = token.parse::<i64>() {
        return Some(Number::from(value));
    }
    if let Ok(value) = token.parse::<u64>() {
        return Some(Number::from(value));
    }
    let value = token.parse::<f64>().ok()?;
    Number::from_f64(value)
}

fn decode_relaxed_string(token: &str) -> String {
    let mut value = String::with_capacity(token.len());
    let mut chars = token.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch != '\\' {
            value.push(ch);
            continue;
        }

        let Some(next) = chars.next() else {
            value.push('\\');
            break;
        };

        match next {
            '"' => value.push('"'),
            '\'' => value.push('\''),
            '\\' => value.push('\\'),
            '/' => value.push('/'),
            'b' => value.push('\u{0008}'),
            'f' => value.push('\u{000c}'),
            'n' => value.push('\n'),
            'r' => value.push('\r'),
            't' => value.push('\t'),
            'u' => {
                let digits = chars.by_ref().take(4).collect::<String>();
                if digits.len() == 4 {
                    if let Ok(code) = u32::from_str_radix(&digits, 16) {
                        if let Some(decoded) = char::from_u32(code) {
                            value.push(decoded);
                            continue;
                        }
                    }
                }
                value.push('\\');
                value.push('u');
                value.push_str(&digits);
            }
            other => {
                value.push('\\');
                value.push(other);
            }
        }
    }

    value
}

#[cfg(test)]
mod tests {
    use super::parse_object_argument;
    use serde_json::{Map, Value, json};

    #[test]
    fn parses_strict_json_object() {
        let parsed = parse_object_argument(r#"{"skill_name":"cpp_editor_api","args":{}}"#)
            .expect("strict JSON should parse");
        let expected = json!({
            "skill_name": "cpp_editor_api",
            "args": {}
        });
        assert_eq!(Value::Object(parsed), expected);
    }

    #[test]
    fn parses_powershell_style_object_with_bare_words() {
        let parsed = parse_object_argument("{skill_name:cpp_editor_api,max_results:25}")
            .expect("relaxed object should parse");
        assert_eq!(
            parsed,
            Map::from_iter([
                ("skill_name".to_string(), Value::String("cpp_editor_api".to_string())),
                ("max_results".to_string(), Value::from(25)),
            ])
        );
    }

    #[test]
    fn parses_relaxed_python_source_and_decodes_unicode_escapes() {
        let parsed = parse_object_argument(
            r#"{python:RESULT = {\u0027ok\u0027: True, \u0027source\u0027: \u0027manual-cli-smoke\u0027},args:{}}"#,
        )
        .expect("relaxed Python payload should parse");

        assert_eq!(
            parsed.get("python"),
            Some(&Value::String(
                "RESULT = {'ok': True, 'source': 'manual-cli-smoke'}".to_string()
            ))
        );
        assert_eq!(parsed.get("args"), Some(&Value::Object(Map::new())));
    }
}
